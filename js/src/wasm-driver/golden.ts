import { Vp9Decoder } from "../wasm";
import type { DecodeStep, NativeFrame, Plane } from "../wasm";
import { spawnWorkerPool } from "./pool";
import type { WorkerPool } from "./pool";
import { parseIvf as demuxIvf } from "../ivf";
import { parseWebm } from "../webm";
import {
  createScratchVip9rMemory,
  createVip9rMemory,
  makeVip9rImports,
  sessionMaxPages,
} from "./wasm-env";
import type { WasmLog, WasmLogSink } from "./wasm-env";

export { formatWasmLog, WasmLogKind } from "./wasm-env";
export type { WasmLog, WasmLogSink } from "./wasm-env";

export type Vp9Packet = {
  index: number;
  timestamp: bigint;
  keyframe?: boolean;
  visible?: boolean;
  payload: Uint8Array;
};

export type DemuxedVp9 = {
  container: "ivf" | "webm";
  codec: "VP90" | "V_VP9";
  width: number;
  height: number;
  timebaseDenominator?: number;
  timebaseNumerator?: number;
  declaredFrameCount?: number;
  timestampScale?: number;
  packets: Vp9Packet[];
};

export type IvfPacket = Vp9Packet;

export type IvfFile = DemuxedVp9 & {
  container: "ivf";
  codec: "VP90";
  fourcc: "VP90";
  timebaseDenominator: number;
  timebaseNumerator: number;
  declaredFrameCount: number;
};

export type GoldenFrame = {
  md5: string;
  name: string;
};

export type BenchmarkOptions = {
  outputOffset: number;
  outputFrames: number;
  warmupMs: number;
  targetMs: number;
};

export const DEFAULT_BENCHMARK_OPTIONS: BenchmarkOptions = {
  outputOffset: 0,
  outputFrames: 82,
  warmupMs: 1000,
  targetMs: 5000,
};
const DEFAULT_GOLDEN_INPUT = "/bulk/vip9r/chromium/bear-vp9.ivf";
const DEFAULT_BENCHMARK_INPUT = "/bulk/vip9r/chromium/bear-vp9.ivf";

export type DriverArgs = {
  allowMismatch: boolean;
  pool: boolean;
  wasmPath: string;
  inputPath: string;
  goldenPath: string;
  progressFrames?: number;
  frames?: DecodeWindow;
  bench?: BenchmarkOptions;
  timed?: TimedOptions;
};

// Timed mode measures one wall-clock decode of the first `packets` demuxed
// packets (all of them when absent). The unit is packets, not output frames,
// to match the web bench page: both sides feed the identical packet list and
// divide by frames actually output.
export type TimedOptions = {
  packets?: number;
};

export type FrameComparison = {
  frameNumber: number;
  expectedMd5?: string;
  expectedName?: string;
  actualMd5: string;
  decodedWidth: number;
  decodedHeight: number;
  renderWidth: number;
  renderHeight: number;
};

export type ComparisonReport = {
  inputPath: string;
  goldenPath: string;
  container: "ivf" | "webm";
  codec: "VP90" | "V_VP9";
  width: number;
  height: number;
  timebaseDenominator?: number;
  timebaseNumerator?: number;
  declaredFrameCount?: number;
  timestampScale?: number;
  packetCount: number;
  codedFrames: number;
  decodedOutputFrames: number;
  skippedOutputFrames: number;
  comparisons: FrameComparison[];
  expectedCount: number;
  // Final linear memory size; memory only grows, so this is peak use.
  // Evidence for the packet-tail budget in sessionMaxPages.
  finalMemoryBytes?: number;
};

export type ProgressEvent = {
  packetIndex: number;
  packetCount: number;
  codedFrames: number;
  decodedOutputFrames: number;
  comparedFrames: number;
  expectedCount: number;
};

export type CompareOptions = {
  progressFrames?: number;
  onProgress?: (event: ProgressEvent) => void;
};

export type GoldenIo = {
  read(path: string): string;
  readbuffer(path: string): ArrayBuffer;
  log: WasmLogSink;
  progress?: (event: ProgressEvent) => void;
  now?: () => number;
};

export type FrameDecoder = {
  beginPacket(packet: Uint8Array): void;
  decodeNext(): DecodeStep;
  planeBytes(plane: Plane): Uint8Array;
};

export type DecodeWindow = Pick<
  BenchmarkOptions,
  "outputOffset" | "outputFrames"
>;

export type DecodeWindowStats = {
  codedFrames: number;
  decodedOutputFrames: number;
  skippedOutputFrames: number;
  selectedOutputFrames: number;
};

type DecodeWindowDeadline = {
  phase: string;
  pass: number;
  startMs: number;
  limitMs: number;
  now: () => number;
};

export type BenchmarkDecodePlan = {
  input: DemuxedVp9;
  decodeWindow: DecodeWindow;
  goldenOffset: number;
};

export type BenchmarkTimedPasses = {
  targetMs: number;
  elapsedMs: number;
  passes: number;
  passMs: number[];
  minPassMs: number;
  maxPassMs: number;
  codedFrames: number;
  decodedOutputFrames: number;
  outputFrames: number;
  skippedOutputFrames: number;
  codedFramesPerPass: number;
  outputFramesPerPass: number;
  msPerFrame: number;
  fps: number;
};

export type BenchmarkReport = {
  mode: "bench";
  ok: true;
  input: string;
  golden: string;
  container: "ivf" | "webm";
  codec: "VP90" | "V_VP9";
  width: number;
  height: number;
  packetCount: number;
  pool: boolean;
  outputOffset: number;
  outputFrames: number;
  validation: {
    matchedCount: number;
    mismatchedCount: number;
    missingCount: number;
    extraCount: number;
    codedFrames: number;
    decodedOutputFrames: number;
    outputFrames: number;
    skippedOutputFrames: number;
  };
  warmup: BenchmarkTimedPasses;
  measurement: BenchmarkTimedPasses;
};

export type TimedReport = {
  mode: "timed";
  ok: true;
  input: string;
  golden: string;
  container: "ivf" | "webm";
  codec: "VP90" | "V_VP9";
  width: number;
  height: number;
  pool: boolean;
  packets: number;
  validation: {
    elapsedMs: number;
    matchedFrames: number;
    codedFrames: number;
    decodedOutputFrames: number;
  };
  wallMs: number;
  codedFrames: number;
  decodedOutputFrames: number;
  msPerFrame: number;
  fps: number;
};

export class DriverUsageError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DriverUsageError";
  }
}

export function parseDriverArgs(args: string[]): DriverArgs {
  let allowMismatch = false;
  let pool = false;
  let progressFrames: number | undefined;
  let frames: DecodeWindow | undefined;
  let bench = false;
  let timed = false;
  let packets: number | undefined;
  const benchmarkOptions = { ...DEFAULT_BENCHMARK_OPTIONS };
  const paths: string[] = [];

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--allow-mismatch") {
      allowMismatch = true;
      continue;
    }
    if (arg === "--bench") {
      bench = true;
      continue;
    }
    if (arg === "--timed") {
      timed = true;
      continue;
    }
    if (arg === "--packets") {
      const value = args[index + 1];
      if (value === undefined) {
        throw new Error("--packets requires a count");
      }
      index += 1;
      packets = parsePositiveInteger(value, "--packets");
      continue;
    }
    if (arg === "--pool") {
      pool = true;
      continue;
    }
    if (arg.startsWith("--progress-frames=")) {
      progressFrames = parsePositiveInteger(
        arg.slice("--progress-frames=".length),
        "--progress-frames",
      );
      continue;
    }
    if (arg === "--frames") {
      const value = args[index + 1];
      if (value === undefined) {
        throw new Error("--frames requires START:LAST");
      }
      index += 1;
      frames = parseOutputFrameRange(value);
      continue;
    }
    if (arg.startsWith("-")) {
      throw new Error(`unknown argument: ${arg}`);
    }
    paths.push(arg);
  }

  if (paths.length < 1 || paths.length > 2) {
    throw new DriverUsageError("invalid path count");
  }
  if (bench && timed) {
    throw new Error("--bench cannot be used with --timed");
  }
  if (!timed && packets !== undefined) {
    throw new Error("--packets requires --timed");
  }
  if (timed && allowMismatch) {
    throw new Error("--allow-mismatch cannot be used with --timed");
  }
  if (timed && progressFrames !== undefined) {
    throw new Error("--progress-frames cannot be used with --timed");
  }
  if (timed && frames !== undefined) {
    throw new Error("--frames cannot be used with --timed; use --packets");
  }
  if (bench && allowMismatch) {
    throw new Error("--allow-mismatch cannot be used with --bench");
  }
  if (bench && progressFrames !== undefined) {
    throw new Error("--progress-frames cannot be used with --bench");
  }
  if (!bench && frames !== undefined && progressFrames !== undefined) {
    throw new Error("--progress-frames cannot be used with --frames");
  }
  if (bench && frames !== undefined) {
    benchmarkOptions.outputOffset = frames.outputOffset;
    benchmarkOptions.outputFrames = frames.outputFrames;
  }

  const [wasmPath, explicitInputPath] = paths;
  const inputPath =
    explicitInputPath ??
    (bench || timed ? DEFAULT_BENCHMARK_INPUT : DEFAULT_GOLDEN_INPUT);
  const goldenPath = `${inputPath}.md5`;
  return {
    allowMismatch,
    pool,
    wasmPath,
    inputPath,
    goldenPath,
    progressFrames,
    frames: bench ? undefined : frames,
    bench: bench ? benchmarkOptions : undefined,
    timed: timed ? { packets } : undefined,
  };
}

export function compareWasmToGolden(
  args: DriverArgs,
  io: GoldenIo,
): ComparisonReport {
  const wasm = new WebAssembly.Module(io.readbuffer(args.wasmPath));
  const { input, golden, decoderDimensions } = readGoldenWorkload(args, io);
  // The single pool (if any) lives for the whole comparison; workers die with
  // the process (Worker.terminate at exit is the pinned shutdown story).
  const { decoder } = instantiateVp9Decoder(
    wasm,
    decoderDimensions,
    io.log,
    args.pool,
  );

  const report =
    args.frames !== undefined
      ? compareDecodedVp9WindowToGolden(
          args.inputPath,
          args.goldenPath,
          input,
          golden,
          decoder,
          args.frames,
        )
      : compareDecodedVp9ToGolden(
          args.inputPath,
          args.goldenPath,
          input,
          golden,
          decoder,
          {
            progressFrames: args.progressFrames,
            onProgress: io.progress,
          },
        );
  report.finalMemoryBytes = decoder.memoryByteLength();
  return report;
}

export function benchmarkWasmGolden(
  args: DriverArgs,
  io: GoldenIo,
): BenchmarkReport {
  if (args.bench === undefined) {
    throw new Error("benchmark options missing");
  }

  const wasm = new WebAssembly.Module(io.readbuffer(args.wasmPath));
  const { input, golden, decoderDimensions } = readGoldenWorkload(args, io);
  const window = normalizeDecodeWindow(args.bench);
  const warmupMs = validNonNegativeInteger(
    "benchmark warmup ms",
    args.bench.warmupMs,
  );
  const targetMs = validPositiveIntegerValue(
    "benchmark target ms",
    args.bench.targetMs,
  );
  validateGoldenWindow(golden, window);
  const plan = planBenchmarkDecode(input, window);
  // Each pass gets a fresh decoder over a fresh shared memory; parked pool
  // workers would keep every retired pass's memory reservation alive, so the
  // previous pool is terminated before the next decoder spawns its own.
  let livePool: WorkerPool | undefined;
  const makeDecoder = () => {
    livePool?.terminate();
    const made = instantiateVp9Decoder(
      wasm,
      decoderDimensions,
      io.log,
      args.pool,
    );
    livePool = made.pool;
    return made.decoder;
  };
  const now = io.now ?? monotonicNow;

  const { validation, warmup } = runTimedWarmupValidation(
    args.inputPath,
    args.goldenPath,
    plan.input,
    golden,
    plan.decodeWindow,
    plan.goldenOffset,
    makeDecoder,
    warmupMs,
    targetMs,
    now,
  );

  const measurement = runTimedDecodePasses(
    plan.input,
    golden,
    plan.decodeWindow,
    makeDecoder,
    targetMs,
    targetMs,
    now,
    true,
    "measurement",
  );

  return {
    mode: "bench",
    ok: true,
    input: args.inputPath,
    golden: args.goldenPath,
    container: input.container,
    codec: input.codec,
    width: input.width,
    height: input.height,
    packetCount: input.packets.length,
    pool: args.pool,
    outputOffset: window.outputOffset,
    outputFrames: window.outputFrames,
    validation: {
      matchedCount: matchedCount(validation),
      mismatchedCount: mismatchCount(validation),
      missingCount: missingCount(validation),
      extraCount: extraCount(validation),
      codedFrames: validation.codedFrames,
      decodedOutputFrames: validation.decodedOutputFrames,
      outputFrames: validation.comparisons.length,
      skippedOutputFrames: validation.skippedOutputFrames,
    },
    warmup,
    measurement,
  };
}

export function timedWasmGolden(args: DriverArgs, io: GoldenIo): TimedReport {
  if (args.timed === undefined) {
    throw new Error("timed options missing");
  }

  const wasm = new WebAssembly.Module(io.readbuffer(args.wasmPath));
  const { input, golden, decoderDimensions } = readGoldenWorkload(args, io);
  const requested = args.timed.packets;
  if (requested !== undefined && requested > input.packets.length) {
    throw new Error(
      `--packets ${requested} exceeds packet count ${input.packets.length}`,
    );
  }
  const timedInput =
    requested === undefined
      ? input
      : { ...input, packets: input.packets.slice(0, requested) };
  // Every comparable output frame lands in the window; golden covers the whole
  // clip, so a truncated packet list just stops short of filling it.
  const window = { outputOffset: 0, outputFrames: golden.length };

  let livePool: WorkerPool | undefined;
  const makeDecoder = () => {
    livePool?.terminate();
    const made = instantiateVp9Decoder(
      wasm,
      decoderDimensions,
      io.log,
      args.pool,
    );
    livePool = made.pool;
    return made.decoder;
  };
  const now = io.now ?? monotonicNow;

  // The validation pass doubles as warmup and keeps all md5 work out of the
  // timed pass.
  const validationStart = now();
  const validationDecoder = makeDecoder();
  let matchedFrames = 0;
  const validationStats = decodeVp9Window(
    timedInput,
    golden,
    validationDecoder,
    window,
    (frame, context) => {
      const actual = md5Hex(compactI420(validationDecoder, frame));
      const expected = golden[context.selectedIndex];
      if (actual !== expected.md5) {
        throw new Error(
          `timed validation mismatch at output frame ${context.selectedIndex + 1}: expected ${expected.md5} (${expected.name}), got ${actual}`,
        );
      }
      matchedFrames += 1;
    },
  );
  if (validationStats.selectedOutputFrames === 0) {
    throw new Error("timed validation decoded no output frames");
  }
  const validationMs = now() - validationStart;

  const timedDecoder = makeDecoder();
  const before = now();
  const stats = decodeVp9Window(timedInput, golden, timedDecoder, window);
  const wallMs = now() - before;

  return {
    mode: "timed",
    ok: true,
    input: args.inputPath,
    golden: args.goldenPath,
    container: input.container,
    codec: input.codec,
    width: input.width,
    height: input.height,
    pool: args.pool,
    packets: timedInput.packets.length,
    validation: {
      elapsedMs: validationMs,
      matchedFrames,
      codedFrames: validationStats.codedFrames,
      decodedOutputFrames: validationStats.decodedOutputFrames,
    },
    wallMs,
    codedFrames: stats.codedFrames,
    decodedOutputFrames: stats.decodedOutputFrames,
    ...frameRate(stats.decodedOutputFrames, wallMs),
  };
}

export function compareDecodedVp9ToGolden(
  inputPath: string,
  goldenPath: string,
  input: DemuxedVp9,
  golden: GoldenFrame[],
  decoder: FrameDecoder,
  options: CompareOptions = {},
): ComparisonReport {
  const comparisons: FrameComparison[] = [];
  const comparedDimensions = zeroDimensionIvf(input)
    ? comparedGoldenDimensions(golden)
    : undefined;
  const progressFrames = normalizeProgressFrames(options.progressFrames);
  let nextProgressFrame = progressFrames;
  let coded = 0;
  let decodedOutputFrames = 0;
  let skippedOutputFrames = 0;
  for (const packet of input.packets) {
    try {
      decoder.beginPacket(packet.payload);
    } catch (error) {
      throw contextError(
        `decode packet ${packet.index} timestamp ${packet.timestamp}`,
        error,
      );
    }

    let codedIndex = 0;
    while (true) {
      let step: DecodeStep;
      try {
        step = decoder.decodeNext();
      } catch (error) {
        throw contextError(
          `decode packet ${packet.index} coded frame ${codedIndex}`,
          error,
        );
      }
      coded += 1;
      codedIndex += 1;

      if (step.kind === "output") {
        decodedOutputFrames += 1;
        if (
          comparedDimensions !== undefined &&
          !comparedDimensions.has(
            dimensionKey(step.frame.decodedWidth, step.frame.decodedHeight),
          )
        ) {
          skippedOutputFrames += 1;
          if (step.packetDone) {
            break;
          }
          continue;
        }

        let actual: string;
        try {
          actual = md5Hex(compactI420(decoder, step.frame));
        } catch (error) {
          throw contextError(
            `decode packet ${packet.index} coded frame ${codedIndex - 1} output frame ${comparisons.length + 1}`,
            error,
          );
        }
        const expected = golden[comparisons.length];
        comparisons.push({
          frameNumber: comparisons.length + 1,
          expectedMd5: expected?.md5,
          expectedName: expected?.name,
          actualMd5: actual,
          decodedWidth: step.frame.decodedWidth,
          decodedHeight: step.frame.decodedHeight,
          renderWidth: step.frame.renderWidth,
          renderHeight: step.frame.renderHeight,
        });
        if (
          progressFrames !== undefined &&
          nextProgressFrame !== undefined &&
          options.onProgress !== undefined
        ) {
          while (comparisons.length >= nextProgressFrame) {
            options.onProgress({
              packetIndex: packet.index,
              packetCount: input.packets.length,
              codedFrames: coded,
              decodedOutputFrames,
              comparedFrames: comparisons.length,
              expectedCount: golden.length,
            });
            nextProgressFrame += progressFrames;
          }
        }
      }
      if (step.packetDone) {
        break;
      }
    }
  }

  return makeReport(
    inputPath,
    goldenPath,
    input,
    coded,
    decodedOutputFrames,
    skippedOutputFrames,
    comparisons,
    golden.length,
  );
}

export function compareDecodedVp9WindowToGolden(
  inputPath: string,
  goldenPath: string,
  input: DemuxedVp9,
  golden: GoldenFrame[],
  decoder: FrameDecoder,
  window: DecodeWindow,
  options: { goldenOffset?: number; deadline?: DecodeWindowDeadline } = {},
): ComparisonReport {
  const normalizedWindow = normalizeDecodeWindow(window);
  const goldenOffset =
    options.goldenOffset === undefined
      ? normalizedWindow.outputOffset
      : validNonNegativeInteger("golden output offset", options.goldenOffset);
  validateGoldenWindow(golden, {
    outputOffset: goldenOffset,
    outputFrames: normalizedWindow.outputFrames,
  });
  const comparisons: FrameComparison[] = [];
  const stats = decodeVp9Window(
    input,
    golden,
    decoder,
    normalizedWindow,
    (frame, context) => {
      let actual: string;
      try {
        actual = md5Hex(compactI420(decoder, frame));
      } catch (error) {
        throw contextError(
          `decode packet ${context.packetIndex} coded frame ${context.codedFrameIndex} output frame ${
            goldenOffset + context.selectedIndex + 1
          }`,
          error,
        );
      }
      const expected = golden[goldenOffset + context.selectedIndex];
      comparisons.push({
        frameNumber: goldenOffset + context.selectedIndex + 1,
        expectedMd5: expected.md5,
        expectedName: expected.name,
        actualMd5: actual,
        decodedWidth: frame.decodedWidth,
        decodedHeight: frame.decodedHeight,
        renderWidth: frame.renderWidth,
        renderHeight: frame.renderHeight,
      });
    },
    options.deadline,
  );

  return makeReport(
    inputPath,
    goldenPath,
    input,
    stats.codedFrames,
    stats.decodedOutputFrames,
    stats.skippedOutputFrames,
    comparisons,
    normalizedWindow.outputFrames,
  );
}

export function planBenchmarkDecode(
  input: DemuxedVp9,
  window: DecodeWindow,
): BenchmarkDecodePlan {
  const normalizedWindow = normalizeDecodeWindow(window);
  if (normalizedWindow.outputOffset === 0) {
    return {
      input,
      decodeWindow: normalizedWindow,
      goldenOffset: 0,
    };
  }

  if (input.container !== "webm") {
    throw new Error(
      "nonzero benchmark --frames start requires WebM keyframe metadata",
    );
  }

  let visibleIndex = 0;
  for (
    let packetIndex = 0;
    packetIndex < input.packets.length;
    packetIndex += 1
  ) {
    const packet = input.packets[packetIndex];
    if (packet.visible === false) {
      continue;
    }

    if (visibleIndex === normalizedWindow.outputOffset) {
      if (packet.keyframe !== true) {
        throw new Error(
          `benchmark start frame ${normalizedWindow.outputOffset} maps to WebM packet ${packet.index}, which is not marked as a keyframe`,
        );
      }
      return {
        input: {
          ...input,
          packets: input.packets.slice(packetIndex),
        },
        decodeWindow: {
          outputOffset: 0,
          outputFrames: normalizedWindow.outputFrames,
        },
        goldenOffset: normalizedWindow.outputOffset,
      };
    }

    visibleIndex += 1;
  }

  throw new Error(
    `benchmark start frame ${normalizedWindow.outputOffset} exceeds WebM visible packet count ${visibleIndex}`,
  );
}

export function decodeVp9Window(
  input: DemuxedVp9,
  golden: GoldenFrame[],
  decoder: FrameDecoder,
  window: DecodeWindow,
  onSelectedOutput?: (frame: NativeFrame, context: WindowOutputContext) => void,
  deadline?: DecodeWindowDeadline,
): DecodeWindowStats {
  const normalizedWindow = normalizeDecodeWindow(window);
  const comparedDimensions = zeroDimensionIvf(input)
    ? comparedGoldenDimensions(golden)
    : undefined;
  let codedFrames = 0;
  let decodedOutputFrames = 0;
  let skippedOutputFrames = 0;
  let comparableOutputFrames = 0;
  let selectedOutputFrames = 0;

  const stats = (): DecodeWindowStats => ({
    codedFrames,
    decodedOutputFrames,
    skippedOutputFrames,
    selectedOutputFrames,
  });

  const checkDeadline = (): void => {
    if (deadline === undefined) {
      return;
    }
    const elapsedMs = deadline.now() - deadline.startMs;
    if (elapsedMs < 0) {
      throw new Error("benchmark clock moved backwards");
    }
    if (elapsedMs > deadline.limitMs) {
      throw new Error(
        `benchmark ${deadline.phase} pass ${deadline.pass} exceeded ${deadline.limitMs}ms before completing decode window: selected ${selectedOutputFrames}/${normalizedWindow.outputFrames} output frames, decoded ${decodedOutputFrames} outputs, coded ${codedFrames} frames, elapsed ${elapsedMs}ms`,
      );
    }
  };

  for (const packet of input.packets) {
    try {
      decoder.beginPacket(packet.payload);
    } catch (error) {
      throw contextError(
        `decode packet ${packet.index} timestamp ${packet.timestamp}`,
        error,
      );
    }
    checkDeadline();

    let codedIndex = 0;
    while (true) {
      let step: DecodeStep;
      try {
        step = decoder.decodeNext();
      } catch (error) {
        throw contextError(
          `decode packet ${packet.index} coded frame ${codedIndex}`,
          error,
        );
      }
      codedFrames += 1;
      codedIndex += 1;

      if (step.kind === "output") {
        decodedOutputFrames += 1;
        if (
          comparedDimensions !== undefined &&
          !comparedDimensions.has(
            dimensionKey(step.frame.decodedWidth, step.frame.decodedHeight),
          )
        ) {
          skippedOutputFrames += 1;
          checkDeadline();
          if (step.packetDone) {
            break;
          }
          continue;
        }

        const outputIndex = comparableOutputFrames;
        comparableOutputFrames += 1;
        let windowComplete = false;
        if (
          outputIndex >= normalizedWindow.outputOffset &&
          selectedOutputFrames < normalizedWindow.outputFrames
        ) {
          const selectedIndex = selectedOutputFrames;
          selectedOutputFrames += 1;
          onSelectedOutput?.(step.frame, {
            packetIndex: packet.index,
            codedFrameIndex: codedIndex - 1,
            outputIndex,
            selectedIndex,
          });
          windowComplete =
            selectedOutputFrames >= normalizedWindow.outputFrames;
        }
        checkDeadline();
        if (windowComplete) {
          return stats();
        }
      } else {
        checkDeadline();
      }
      if (step.packetDone) {
        break;
      }
    }
  }

  return stats();
}

type WindowOutputContext = {
  packetIndex: number;
  codedFrameIndex: number;
  outputIndex: number;
  selectedIndex: number;
};

function normalizeProgressFrames(
  progressFrames: number | undefined,
): number | undefined {
  if (progressFrames === undefined) {
    return undefined;
  }
  if (!Number.isSafeInteger(progressFrames) || progressFrames <= 0) {
    throw new Error(`invalid progress frame interval: ${progressFrames}`);
  }
  return progressFrames;
}

function normalizeDecodeWindow(window: DecodeWindow): DecodeWindow {
  const outputOffset = validNonNegativeInteger(
    "benchmark output offset",
    window.outputOffset,
  );
  const outputFrames = validPositiveIntegerValue(
    "benchmark output frames",
    window.outputFrames,
  );
  return { outputOffset, outputFrames };
}

function validateGoldenWindow(
  golden: GoldenFrame[],
  window: DecodeWindow,
): void {
  const end = checkedAdd(
    window.outputOffset,
    window.outputFrames,
    "output window",
  );
  if (end > golden.length) {
    throw new Error(
      `output window ${window.outputOffset}..${end} exceeds golden frame count ${golden.length}`,
    );
  }
}

export function runTimedWarmupValidation(
  inputPath: string,
  goldenPath: string,
  input: DemuxedVp9,
  golden: GoldenFrame[],
  window: DecodeWindow,
  goldenOffset: number,
  makeDecoder: () => FrameDecoder,
  warmupMs: number,
  passLimitMs: number,
  now: () => number,
): { validation: ComparisonReport; warmup: BenchmarkTimedPasses } {
  const start = now();
  let elapsedMs = 0;
  let passCount = 0;
  const passMs: number[] = [];
  let codedFrames = 0;
  let decodedOutputFrames = 0;
  let outputFrames = 0;
  let skippedOutputFrames = 0;

  const recordPass = (passStartMs: number, stats: DecodeWindowStats): void => {
    passCount += 1;
    codedFrames += stats.codedFrames;
    decodedOutputFrames += stats.decodedOutputFrames;
    outputFrames += stats.selectedOutputFrames;
    skippedOutputFrames += stats.skippedOutputFrames;
    const passEnd = now();
    passMs.push(passEnd - passStartMs);
    elapsedMs = passEnd - start;
    if (elapsedMs < 0) {
      throw new Error("benchmark clock moved backwards");
    }
  };

  const passStart = now();
  if (passStart - start < 0) {
    throw new Error("benchmark clock moved backwards");
  }
  const validation = compareDecodedVp9WindowToGolden(
    inputPath,
    goldenPath,
    input,
    golden,
    makeDecoder(),
    window,
    {
      goldenOffset,
      deadline: {
        phase: "warmup validation",
        pass: 1,
        startMs: passStart,
        limitMs: passLimitMs,
        now,
      },
    },
  );
  if (!passes(validation, false)) {
    throw new Error(
      `benchmark validation failed: ${matchedCount(validation)} matched, ${mismatchCount(validation)} mismatched, ${missingCount(validation)} missing, ${extraCount(validation)} extra`,
    );
  }
  recordPass(passStart, {
    codedFrames: validation.codedFrames,
    decodedOutputFrames: validation.decodedOutputFrames,
    skippedOutputFrames: validation.skippedOutputFrames,
    selectedOutputFrames: validation.comparisons.length,
  });

  while (elapsedMs < warmupMs) {
    const warmupPassStart = now();
    if (warmupPassStart - start < 0) {
      throw new Error("benchmark clock moved backwards");
    }
    const stats = decodeVp9Window(
      input,
      golden,
      makeDecoder(),
      window,
      undefined,
      {
        phase: "warmup",
        pass: passCount + 1,
        startMs: warmupPassStart,
        limitMs: passLimitMs,
        now,
      },
    );
    if (stats.selectedOutputFrames !== window.outputFrames) {
      throw new Error(
        `benchmark decode window incomplete: selected ${stats.selectedOutputFrames}/${window.outputFrames} output frames`,
      );
    }
    recordPass(warmupPassStart, stats);
  }

  return {
    validation,
    warmup: {
      targetMs: warmupMs,
      elapsedMs,
      passes: passCount,
      passMs,
      ...passRange(passMs),
      codedFrames,
      decodedOutputFrames,
      outputFrames,
      skippedOutputFrames,
      codedFramesPerPass: codedFrames / passCount,
      outputFramesPerPass: outputFrames / passCount,
      ...frameRate(outputFrames, elapsedMs),
    },
  };
}

export function runTimedDecodePasses(
  input: DemuxedVp9,
  golden: GoldenFrame[],
  window: DecodeWindow,
  makeDecoder: () => FrameDecoder,
  targetMs: number,
  passLimitMs: number,
  now: () => number,
  runAtLeastOnce: boolean,
  phase: string,
): BenchmarkTimedPasses {
  const start = now();
  let elapsedMs = 0;
  let passes = 0;
  const passMs: number[] = [];
  let codedFrames = 0;
  let decodedOutputFrames = 0;
  let outputFrames = 0;
  let skippedOutputFrames = 0;

  while ((runAtLeastOnce && passes === 0) || elapsedMs < targetMs) {
    const passStart = now();
    if (passStart - start < 0) {
      throw new Error("benchmark clock moved backwards");
    }
    const stats = decodeVp9Window(
      input,
      golden,
      makeDecoder(),
      window,
      undefined,
      {
        phase,
        pass: passes + 1,
        startMs: passStart,
        limitMs: passLimitMs,
        now,
      },
    );
    if (stats.selectedOutputFrames !== window.outputFrames) {
      throw new Error(
        `benchmark decode window incomplete: selected ${stats.selectedOutputFrames}/${window.outputFrames} output frames`,
      );
    }
    passes += 1;
    codedFrames += stats.codedFrames;
    decodedOutputFrames += stats.decodedOutputFrames;
    outputFrames += stats.selectedOutputFrames;
    skippedOutputFrames += stats.skippedOutputFrames;
    const passEnd = now();
    passMs.push(passEnd - passStart);
    elapsedMs = passEnd - start;
    if (elapsedMs < 0) {
      throw new Error("benchmark clock moved backwards");
    }
  }

  if (passes === 0) {
    elapsedMs = now() - start;
    if (elapsedMs < 0) {
      throw new Error("benchmark clock moved backwards");
    }
  }

  return {
    targetMs,
    elapsedMs,
    passes,
    passMs,
    ...passRange(passMs),
    codedFrames,
    decodedOutputFrames,
    outputFrames,
    skippedOutputFrames,
    codedFramesPerPass: passes === 0 ? 0 : codedFrames / passes,
    outputFramesPerPass: passes === 0 ? 0 : outputFrames / passes,
    ...frameRate(outputFrames, elapsedMs),
  };
}

function passRange(passMs: number[]): { minPassMs: number; maxPassMs: number } {
  if (passMs.length === 0) {
    return { minPassMs: 0, maxPassMs: 0 };
  }
  return { minPassMs: Math.min(...passMs), maxPassMs: Math.max(...passMs) };
}

function frameRate(
  frames: number,
  elapsedMs: number,
): { msPerFrame: number; fps: number } {
  if (frames === 0 || elapsedMs <= 0) {
    return { msPerFrame: 0, fps: 0 };
  }
  return {
    msPerFrame: elapsedMs / frames,
    fps: (frames * 1000) / elapsedMs,
  };
}

function parsePositiveInteger(value: string, name: string): number {
  if (!/^[1-9]\d*$/.test(value)) {
    throw new Error(`${name} must be a positive integer`);
  }
  return parseSafeInteger(value, name);
}

function parseSafeInteger(value: string, name: string): number {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed)) {
    throw new Error(`${name} is too large: ${value}`);
  }
  return parsed;
}

function parseOutputFrameRange(value: string): DecodeWindow {
  const match = /^(\d+):(\d+)$/.exec(value);
  if (match === null) {
    throw new Error(
      "--frames must be START:LAST with non-negative integer start and last",
    );
  }
  const start = parseSafeInteger(match[1], "--frames start");
  const last = parseSafeInteger(match[2], "--frames last");
  if (last < start) {
    throw new Error("--frames last must be greater than or equal to start");
  }
  return {
    outputOffset: start,
    outputFrames: checkedAdd(last - start, 1, "output frame count"),
  };
}

function validPositiveIntegerValue(name: string, value: number): number {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`${name} must be a positive integer`);
  }
  return value;
}

function validNonNegativeInteger(name: string, value: number): number {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${name} must be a non-negative integer`);
  }
  return value;
}

function zeroDimensionIvf(input: DemuxedVp9): boolean {
  return input.container === "ivf" && input.width === 0 && input.height === 0;
}

export function parseVp9Input(data: Uint8Array): DemuxedVp9 {
  if (data.byteLength >= 4 && ascii(data, 0, 4) === "DKIF") {
    return parseIvf(data);
  }
  if (
    data.byteLength >= 4 &&
    data[0] === 0x1a &&
    data[1] === 0x45 &&
    data[2] === 0xdf &&
    data[3] === 0xa3
  ) {
    const webm = parseWebm(data);
    return {
      container: "webm",
      codec: webm.codecId,
      width: webm.width,
      height: webm.height,
      timestampScale: webm.timestampScale,
      packets: webm.packets,
    };
  }
  throw new Error(
    "unsupported input container: expected IVF DKIF or WebM EBML",
  );
}

export function parseIvf(data: Uint8Array): IvfFile {
  const { packets, ...header } = demuxIvf(data);
  return {
    container: "ivf",
    codec: header.fourcc,
    ...header,
    packets,
  };
}

export function parseGolden(text: string): GoldenFrame[] {
  const frames: GoldenFrame[] = [];
  for (const [lineIndex, line] of text.split(/\r?\n/).entries()) {
    const lineNumber = lineIndex + 1;
    const trimmed = line.trim();
    if (trimmed === "") {
      continue;
    }
    const fields = trimmed.split(/\s+/);
    if (fields.length !== 2) {
      throw new Error(`line ${lineNumber}: expected md5 and frame name`);
    }
    const [hash, name] = fields;
    if (!/^[0-9a-fA-F]{32}$/.test(hash)) {
      throw new Error(`line ${lineNumber}: invalid md5: ${hash}`);
    }
    frames.push({ md5: hash.toLowerCase(), name });
  }
  if (frames.length === 0) {
    throw new Error("golden contains no frames");
  }
  return frames;
}

export function decoderDimensionsForGolden(
  input: Pick<DemuxedVp9, "width" | "height">,
  golden: GoldenFrame[],
): { width: number; height: number } {
  const maxDimensions = maxGoldenDimensions(golden);
  const width = Math.max(input.width, maxDimensions?.width ?? 0);
  const height = Math.max(input.height, maxDimensions?.height ?? 0);
  if (
    !Number.isSafeInteger(width) ||
    !Number.isSafeInteger(height) ||
    width <= 0 ||
    height <= 0
  ) {
    throw new Error(
      `decoder dimensions unavailable: container=${input.width}x${input.height}, golden=${
        maxDimensions === undefined
          ? "none"
          : `${maxDimensions.width}x${maxDimensions.height}`
      }`,
    );
  }
  return { width, height };
}

export function maxGoldenDimensions(
  golden: GoldenFrame[],
): { width: number; height: number } | undefined {
  let width = 0;
  let height = 0;
  for (const frame of golden) {
    const dimensions = dimensionsFromGoldenName(frame.name);
    if (dimensions === undefined) {
      continue;
    }
    width = Math.max(width, dimensions.width);
    height = Math.max(height, dimensions.height);
  }
  if (width === 0 || height === 0) {
    return undefined;
  }
  return { width, height };
}

function comparedGoldenDimensions(
  golden: GoldenFrame[],
): Set<string> | undefined {
  const dimensions = new Set<string>();
  for (const frame of golden) {
    const parsed = dimensionsFromGoldenName(frame.name);
    if (parsed === undefined) {
      return undefined;
    }
    dimensions.add(dimensionKey(parsed.width, parsed.height));
  }
  return dimensions;
}

function dimensionsFromGoldenName(
  name: string,
): { width: number; height: number } | undefined {
  const xSeparated = /(?:^|[-_])(\d+)x(\d+)-\d+\.i420$/i.exec(name);
  const dashSeparated = /(?:^|[-_])(\d+)-(\d+)-\d+\.i420$/i.exec(name);
  const match = xSeparated ?? dashSeparated;
  if (match === null) {
    return undefined;
  }

  const width = Number(match[1]);
  const height = Number(match[2]);
  if (
    !Number.isSafeInteger(width) ||
    !Number.isSafeInteger(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return undefined;
  }
  return { width, height };
}

function dimensionKey(width: number, height: number): string {
  return `${width}x${height}`;
}

export function makeReport(
  inputPath: string,
  goldenPath: string,
  input: DemuxedVp9,
  codedFrames: number,
  decodedOutputFrames: number,
  skippedOutputFrames: number,
  comparisons: FrameComparison[],
  expectedCount: number,
): ComparisonReport {
  return {
    inputPath,
    goldenPath,
    container: input.container,
    codec: input.codec,
    width: input.width,
    height: input.height,
    timebaseDenominator: input.timebaseDenominator,
    timebaseNumerator: input.timebaseNumerator,
    declaredFrameCount: input.declaredFrameCount,
    timestampScale: input.timestampScale,
    packetCount: input.packets.length,
    codedFrames,
    decodedOutputFrames,
    skippedOutputFrames,
    comparisons,
    expectedCount,
  };
}

export function isMatch(comparison: FrameComparison): boolean {
  return comparison.expectedMd5 === comparison.actualMd5;
}

export function matchedCount(report: ComparisonReport): number {
  return report.comparisons.filter(isMatch).length;
}

export function mismatchCount(report: ComparisonReport): number {
  return report.comparisons.filter(
    (comparison) =>
      comparison.expectedMd5 !== undefined && !isMatch(comparison),
  ).length;
}

export function missingCount(report: ComparisonReport): number {
  return Math.max(report.expectedCount - report.comparisons.length, 0);
}

export function extraCount(report: ComparisonReport): number {
  return Math.max(report.comparisons.length - report.expectedCount, 0);
}

export function passes(
  report: ComparisonReport,
  allowMismatch: boolean,
): boolean {
  const matches =
    mismatchCount(report) === 0 &&
    missingCount(report) === 0 &&
    extraCount(report) === 0;
  return (
    matches ||
    (allowMismatch && missingCount(report) === 0 && extraCount(report) === 0)
  );
}

export function formatReport(report: ComparisonReport): string[] {
  const lines = [
    `input: ${report.inputPath}`,
    `golden: ${report.goldenPath}`,
    formatContainerLine(report),
    `decoder: coded_frames=${report.codedFrames} decoded_outputs=${report.decodedOutputFrames} compared_frames=${report.comparisons.length} skipped_outputs=${report.skippedOutputFrames}`,
    `frames: ${matchedCount(report)} matched, ${mismatchCount(report)} mismatched, ${missingCount(report)} missing, ${extraCount(report)} extra`,
  ];

  for (const comparison of report.comparisons
    .filter((comparison) => !isMatch(comparison))
    .slice(0, 10)) {
    lines.push(
      `mismatch frame ${comparison.frameNumber} ${comparison.expectedName ?? "<extra>"}: expected ${comparison.expectedMd5 ?? "<none>"}, actual ${comparison.actualMd5}, size=${comparison.decodedWidth}x${comparison.decodedHeight} render=${comparison.renderWidth}x${comparison.renderHeight}`,
    );
  }

  return lines;
}

export function formatProgress(event: ProgressEvent): string {
  const percent =
    event.expectedCount > 0
      ? ` ${((event.comparedFrames / event.expectedCount) * 100).toFixed(1)}%`
      : "";
  return `progress: compared=${event.comparedFrames}/${event.expectedCount}${percent} decoded_outputs=${event.decodedOutputFrames} coded_frames=${event.codedFrames} packet=${event.packetIndex + 1}/${event.packetCount}`;
}

function formatContainerLine(report: ComparisonReport): string {
  if (report.container === "ivf") {
    return `ivf: fourcc=${report.codec} size=${report.width}x${report.height} timebase=${report.timebaseNumerator}/${report.timebaseDenominator} declared_frames=${report.declaredFrameCount} packets=${report.packetCount}`;
  }
  return `webm: codec=${report.codec} size=${report.width}x${report.height} timestamp_scale=${report.timestampScale} packets=${report.packetCount}`;
}

export function compactI420(
  decoder: FrameDecoder,
  frame: NativeFrame,
): Uint8Array {
  const width = validPositiveInteger("decoded width", frame.decodedWidth);
  const height = validPositiveInteger("decoded height", frame.decodedHeight);
  const chromaWidth = Math.ceil(width / 2);
  const chromaHeight = Math.ceil(height / 2);
  const output = new Uint8Array(requiredI420Length(width, height));

  let offset = 0;
  offset = copyPlane(
    output,
    offset,
    planeBytes(decoder, frame.y, "Y"),
    frame.y.stride,
    width,
    height,
    "Y",
  );
  offset = copyPlane(
    output,
    offset,
    planeBytes(decoder, frame.u, "U"),
    frame.u.stride,
    chromaWidth,
    chromaHeight,
    "U",
  );
  offset = copyPlane(
    output,
    offset,
    planeBytes(decoder, frame.v, "V"),
    frame.v.stride,
    chromaWidth,
    chromaHeight,
    "V",
  );
  if (offset !== output.byteLength) {
    throw new Error(
      `compact I420 length mismatch: wrote ${offset}, expected ${output.byteLength}`,
    );
  }
  return output;
}

function planeBytes(
  decoder: FrameDecoder,
  plane: Plane,
  name: string,
): Uint8Array {
  try {
    return decoder.planeBytes(plane);
  } catch (error) {
    throw contextError(`invalid ${name} plane descriptor`, error);
  }
}

function copyPlane(
  output: Uint8Array,
  outputOffset: number,
  input: Uint8Array,
  stride: number,
  width: number,
  height: number,
  name: string,
): number {
  if (!Number.isInteger(stride) || stride < width) {
    throw new Error(`invalid ${name} plane: stride ${stride} < width ${width}`);
  }

  const requiredInput = checkedAdd(
    checkedMul(stride, height - 1, `${name} plane offset`),
    width,
    `${name} plane input length`,
  );
  if (input.byteLength < requiredInput) {
    throw new Error(
      `invalid ${name} plane: data too short ${input.byteLength} < ${requiredInput}`,
    );
  }

  const requiredOutput = checkedMul(
    width,
    height,
    `${name} plane output length`,
  );
  if (output.byteLength - outputOffset < requiredOutput) {
    throw new Error(
      `compact I420 output too small: ${output.byteLength - outputOffset} < ${requiredOutput}`,
    );
  }

  for (let row = 0; row < height; row += 1) {
    const inputStart = row * stride;
    output.set(input.subarray(inputStart, inputStart + width), outputOffset);
    outputOffset += width;
  }
  return outputOffset;
}

export function md5Hex(input: Uint8Array): string {
  const paddedLen = (Math.floor((input.byteLength + 8) / 64) + 1) * 64;
  const padded = new Uint8Array(paddedLen);
  padded.set(input);
  padded[input.byteLength] = 0x80;

  const bitLen = input.byteLength * 8;
  writeLe32(padded, paddedLen - 8, bitLen >>> 0);
  writeLe32(padded, paddedLen - 4, Math.floor(bitLen / 0x1_0000_0000));

  let a0 = 0x67452301;
  let b0 = 0xefcdab89;
  let c0 = 0x98badcfe;
  let d0 = 0x10325476;
  const words = new Uint32Array(16);

  for (let chunk = 0; chunk < paddedLen; chunk += 64) {
    for (let index = 0; index < 16; index += 1) {
      words[index] = le32(padded, chunk + index * 4);
    }

    let a = a0;
    let b = b0;
    let c = c0;
    let d = d0;

    for (let index = 0; index < 64; index += 1) {
      let f: number;
      let g: number;
      if (index < 16) {
        f = (b & c) | (~b & d);
        g = index;
      } else if (index < 32) {
        f = (d & b) | (~d & c);
        g = (5 * index + 1) & 15;
      } else if (index < 48) {
        f = b ^ c ^ d;
        g = (3 * index + 5) & 15;
      } else {
        f = c ^ (b | ~d);
        g = (7 * index) & 15;
      }

      const nextD = c;
      c = b;
      b =
        (b +
          rotateLeft((a + f + MD5_K[index] + words[g]) >>> 0, MD5_S[index])) >>>
        0;
      a = d;
      d = nextD;
    }

    a0 = (a0 + a) >>> 0;
    b0 = (b0 + b) >>> 0;
    c0 = (c0 + c) >>> 0;
    d0 = (d0 + d) >>> 0;
  }

  const digest = new Uint8Array(16);
  writeLe32(digest, 0, a0);
  writeLe32(digest, 4, b0);
  writeLe32(digest, 8, c0);
  writeLe32(digest, 12, d0);
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
}

function requiredI420Length(width: number, height: number): number {
  const chromaWidth = Math.ceil(width / 2);
  const chromaHeight = Math.ceil(height / 2);
  const luma = checkedMul(width, height, "I420 luma length");
  const chroma = checkedMul(chromaWidth, chromaHeight, "I420 chroma length");
  return checkedAdd(
    luma,
    checkedMul(2, chroma, "I420 chroma pair length"),
    "I420 length",
  );
}

function validPositiveInteger(name: string, value: number): number {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error(`invalid frame dimensions: ${name}=${value}`);
  }
  return value;
}

function checkedAdd(a: number, b: number, name: string): number {
  const value = a + b;
  if (!Number.isSafeInteger(value)) {
    throw new Error(`${name} overflow`);
  }
  return value;
}

function checkedMul(a: number, b: number, name: string): number {
  const value = a * b;
  if (!Number.isSafeInteger(value)) {
    throw new Error(`${name} overflow`);
  }
  return value;
}

function ascii(data: Uint8Array, start: number, end: number): string {
  let out = "";
  for (let index = start; index < end; index += 1) {
    out += String.fromCharCode(data[index]);
  }
  return out;
}

function le32(data: Uint8Array, offset: number): number {
  return (
    (data[offset] |
      (data[offset + 1] << 8) |
      (data[offset + 2] << 16) |
      (data[offset + 3] << 24)) >>>
    0
  );
}

const MD5_S = [
  7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5,
  9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11,
  16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15,
  21,
];

const MD5_K = Array.from(
  { length: 64 },
  (_, index) => Math.floor(Math.abs(Math.sin(index + 1)) * 0x1_0000_0000) >>> 0,
);

function rotateLeft(value: number, bits: number): number {
  return (value << bits) | (value >>> (32 - bits));
}

function writeLe32(data: Uint8Array, offset: number, value: number): void {
  data[offset] = value & 0xff;
  data[offset + 1] = (value >>> 8) & 0xff;
  data[offset + 2] = (value >>> 16) & 0xff;
  data[offset + 3] = (value >>> 24) & 0xff;
}

function contextError(context: string, error: unknown): Error {
  const message = error instanceof Error ? error.message : String(error);
  const wrapped = new Error(`${context}: ${message}`);
  if (error instanceof Error && error.stack) {
    wrapped.stack = `${wrapped.message}\nCaused by: ${error.stack}`;
  }
  return wrapped;
}

function readGoldenWorkload(
  args: Pick<DriverArgs, "inputPath" | "goldenPath">,
  io: Pick<GoldenIo, "read" | "readbuffer">,
): {
  input: DemuxedVp9;
  golden: GoldenFrame[];
  decoderDimensions: { width: number; height: number };
} {
  const input = parseVp9Input(new Uint8Array(io.readbuffer(args.inputPath)));
  const golden = parseGolden(io.read(args.goldenPath));
  const decoderDimensions = decoderDimensionsForGolden(input, golden);
  return { input, golden, decoderDimensions };
}

function instantiateVp9Decoder(
  wasm: WebAssembly.Module,
  decoderDimensions: { width: number; height: number },
  log: WasmLogSink,
  pool: boolean,
): { decoder: Vp9Decoder; pool?: WorkerPool } {
  const scratch = new WebAssembly.Instance(
    wasm,
    makeVip9rImports(createScratchVip9rMemory(), log),
  );
  const memory = createVip9rMemory(
    sessionMaxPages(scratch.exports, decoderDimensions),
  );
  const instance = new WebAssembly.Instance(
    wasm,
    makeVip9rImports(memory, log),
  );
  const decoder = new Vp9Decoder(
    instance,
    decoderDimensions.width,
    decoderDimensions.height,
  );
  if (!pool) {
    return { decoder };
  }
  const workers = spawnWorkerPool(wasm, memory);
  const activate = instance.exports.vip9r_pool_activate;
  if (typeof activate !== "function") {
    workers.terminate();
    throw new Error("missing wasm export: vip9r_pool_activate");
  }
  activate();
  return { decoder, pool: workers };
}

function monotonicNow(): number {
  if (
    typeof performance === "object" &&
    typeof performance.now === "function"
  ) {
    return performance.now();
  }
  return Date.now();
}

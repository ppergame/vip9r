import { Vp9Decoder } from "../wasm";
import type { DecodeStep, NativeFrame, Plane } from "../wasm";
import { parseWebm } from "../webm";

export const WasmLogKind = {
  Diagnostic: 0,
  TestFailure: 1,
  Panic: 2,
} as const;

export type WasmLog = {
  kind: number;
  message: string;
};

export type WasmLogSink = (log: WasmLog) => void;

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
  wasmPath: string;
  inputPath: string;
  goldenPath: string;
  progressFrames?: number;
  bench?: BenchmarkOptions;
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

export type DecodeWindow = Pick<BenchmarkOptions, "outputOffset" | "outputFrames">;

export type DecodeWindowStats = {
  codedFrames: number;
  decodedOutputFrames: number;
  skippedOutputFrames: number;
  selectedOutputFrames: number;
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
  input: string;
  golden: string;
  container: "ivf" | "webm";
  codec: "VP90" | "V_VP9";
  width: number;
  height: number;
  packetCount: number;
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

export class DriverUsageError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "DriverUsageError";
  }
}

export function parseDriverArgs(args: string[]): DriverArgs {
  let allowMismatch = false;
  let progressFrames: number | undefined;
  let bench = false;
  let sawBenchmarkOption = false;
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
    if (arg.startsWith("--progress-frames=")) {
      progressFrames = parsePositiveInteger(arg.slice("--progress-frames=".length), "--progress-frames");
      continue;
    }
    if (arg === "--bench-frames") {
      const value = args[index + 1];
      if (value === undefined) {
        throw new Error("--bench-frames requires START:COUNT");
      }
      index += 1;
      sawBenchmarkOption = true;
      const frameRange = parseBenchmarkFrameRange(value);
      benchmarkOptions.outputOffset = frameRange.outputOffset;
      benchmarkOptions.outputFrames = frameRange.outputFrames;
      continue;
    }
    if (arg.startsWith("--bench-frames=")) {
      sawBenchmarkOption = true;
      const frameRange = parseBenchmarkFrameRange(arg.slice("--bench-frames=".length));
      benchmarkOptions.outputOffset = frameRange.outputOffset;
      benchmarkOptions.outputFrames = frameRange.outputFrames;
      continue;
    }
    if (arg.startsWith("--bench-warmup-ms=")) {
      sawBenchmarkOption = true;
      benchmarkOptions.warmupMs = parseNonNegativeInteger(
        arg.slice("--bench-warmup-ms=".length),
        "--bench-warmup-ms",
      );
      continue;
    }
    if (arg.startsWith("--bench-target-ms=")) {
      sawBenchmarkOption = true;
      benchmarkOptions.targetMs = parsePositiveInteger(arg.slice("--bench-target-ms=".length), "--bench-target-ms");
      continue;
    }
    if (arg.startsWith("-")) {
      throw new Error(`unknown argument: ${arg}`);
    }
    paths.push(arg);
  }

  if (paths.length < 1 || paths.length > 3) {
    throw new DriverUsageError("invalid path count");
  }
  if (sawBenchmarkOption && !bench) {
    throw new Error("benchmark options require --bench");
  }
  if (bench && allowMismatch) {
    throw new Error("--allow-mismatch cannot be used with --bench");
  }
  if (bench && progressFrames !== undefined) {
    throw new Error("--progress-frames cannot be used with --bench");
  }

  const [wasmPath, explicitInputPath, explicitGoldenPath] = paths;
  const inputPath = explicitInputPath ?? (bench ? DEFAULT_BENCHMARK_INPUT : DEFAULT_GOLDEN_INPUT);
  const goldenPath = explicitGoldenPath ?? `${inputPath}.md5`;
  return {
    allowMismatch,
    wasmPath,
    inputPath,
    goldenPath,
    progressFrames,
    bench: bench ? benchmarkOptions : undefined,
  };
}

export function compareWasmToGolden(args: DriverArgs, io: GoldenIo): ComparisonReport {
  const wasm = new WebAssembly.Module(io.readbuffer(args.wasmPath));
  const { input, golden, decoderDimensions } = readGoldenWorkload(args, io);
  const decoder = instantiateVp9Decoder(wasm, decoderDimensions, io.log);

  return compareDecodedVp9ToGolden(args.inputPath, args.goldenPath, input, golden, decoder, {
    progressFrames: args.progressFrames,
    onProgress: io.progress,
  });
}

export function benchmarkWasmGolden(args: DriverArgs, io: GoldenIo): BenchmarkReport {
  if (args.bench === undefined) {
    throw new Error("benchmark options missing");
  }

  const wasm = new WebAssembly.Module(io.readbuffer(args.wasmPath));
  const { input, golden, decoderDimensions } = readGoldenWorkload(args, io);
  const window = normalizeDecodeWindow(args.bench);
  const warmupMs = validNonNegativeInteger("benchmark warmup ms", args.bench.warmupMs);
  const targetMs = validPositiveIntegerValue("benchmark target ms", args.bench.targetMs);
  validateGoldenWindow(golden, window);
  const plan = planBenchmarkDecode(input, window);
  const makeDecoder = () => instantiateVp9Decoder(wasm, decoderDimensions, io.log);

  const validation = compareDecodedVp9WindowToGolden(
    args.inputPath,
    args.goldenPath,
    plan.input,
    golden,
    makeDecoder(),
    plan.decodeWindow,
    { goldenOffset: plan.goldenOffset },
  );
  if (!passes(validation, false)) {
    throw new Error(
      `benchmark validation failed: ${matchedCount(validation)} matched, ${mismatchCount(validation)} mismatched, ${missingCount(validation)} missing, ${extraCount(validation)} extra`,
    );
  }

  const now = io.now ?? monotonicNow;
  const warmup = runTimedDecodePasses(plan.input, golden, plan.decodeWindow, makeDecoder, warmupMs, now, false);
  const measurement = runTimedDecodePasses(plan.input, golden, plan.decodeWindow, makeDecoder, targetMs, now, true);

  return {
    mode: "bench",
    input: args.inputPath,
    golden: args.goldenPath,
    container: input.container,
    codec: input.codec,
    width: input.width,
    height: input.height,
    packetCount: input.packets.length,
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

export function formatWasmLog(log: WasmLog): string {
  switch (log.kind) {
    case WasmLogKind.Diagnostic:
      return `wasm diag: ${log.message}`;
    case WasmLogKind.TestFailure:
      return `wasm test failure: ${log.message}`;
    case WasmLogKind.Panic:
      return `wasm panic: ${log.message}`;
    default:
      return `wasm log ${log.kind}: ${log.message}`;
  }
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
  const comparedDimensions = zeroDimensionIvf(input) ? comparedGoldenDimensions(golden) : undefined;
  const progressFrames = normalizeProgressFrames(options.progressFrames);
  let nextProgressFrame = progressFrames;
  let coded = 0;
  let decodedOutputFrames = 0;
  let skippedOutputFrames = 0;
  for (const packet of input.packets) {
    try {
      decoder.beginPacket(packet.payload);
    } catch (error) {
      throw contextError(`decode packet ${packet.index} timestamp ${packet.timestamp}`, error);
    }

    let codedIndex = 0;
    while (true) {
      let step: DecodeStep;
      try {
        step = decoder.decodeNext();
      } catch (error) {
        throw contextError(`decode packet ${packet.index} coded frame ${codedIndex}`, error);
      }
      coded += 1;
      codedIndex += 1;

      if (step.kind === "output") {
        decodedOutputFrames += 1;
        if (
          comparedDimensions !== undefined &&
          !comparedDimensions.has(dimensionKey(step.frame.decodedWidth, step.frame.decodedHeight))
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
        if (progressFrames !== undefined && nextProgressFrame !== undefined && options.onProgress !== undefined) {
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
  options: { goldenOffset?: number } = {},
): ComparisonReport {
  const normalizedWindow = normalizeDecodeWindow(window);
  const goldenOffset =
    options.goldenOffset === undefined
      ? normalizedWindow.outputOffset
      : validNonNegativeInteger("golden output offset", options.goldenOffset);
  validateGoldenWindow(golden, { outputOffset: goldenOffset, outputFrames: normalizedWindow.outputFrames });
  const comparisons: FrameComparison[] = [];
  const stats = decodeVp9Window(input, golden, decoder, normalizedWindow, (frame, context) => {
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
  });

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

export function planBenchmarkDecode(input: DemuxedVp9, window: DecodeWindow): BenchmarkDecodePlan {
  const normalizedWindow = normalizeDecodeWindow(window);
  if (normalizedWindow.outputOffset === 0) {
    return {
      input,
      decodeWindow: normalizedWindow,
      goldenOffset: 0,
    };
  }

  if (input.container !== "webm") {
    throw new Error("nonzero --bench-frames start requires WebM keyframe metadata");
  }

  let visibleIndex = 0;
  for (let packetIndex = 0; packetIndex < input.packets.length; packetIndex += 1) {
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
): DecodeWindowStats {
  const normalizedWindow = normalizeDecodeWindow(window);
  const comparedDimensions = zeroDimensionIvf(input) ? comparedGoldenDimensions(golden) : undefined;
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

  for (const packet of input.packets) {
    try {
      decoder.beginPacket(packet.payload);
    } catch (error) {
      throw contextError(`decode packet ${packet.index} timestamp ${packet.timestamp}`, error);
    }

    let codedIndex = 0;
    while (true) {
      let step: DecodeStep;
      try {
        step = decoder.decodeNext();
      } catch (error) {
        throw contextError(`decode packet ${packet.index} coded frame ${codedIndex}`, error);
      }
      codedFrames += 1;
      codedIndex += 1;

      if (step.kind === "output") {
        decodedOutputFrames += 1;
        if (
          comparedDimensions !== undefined &&
          !comparedDimensions.has(dimensionKey(step.frame.decodedWidth, step.frame.decodedHeight))
        ) {
          skippedOutputFrames += 1;
          if (step.packetDone) {
            break;
          }
          continue;
        }

        const outputIndex = comparableOutputFrames;
        comparableOutputFrames += 1;
        if (outputIndex >= normalizedWindow.outputOffset && selectedOutputFrames < normalizedWindow.outputFrames) {
          const selectedIndex = selectedOutputFrames;
          selectedOutputFrames += 1;
          onSelectedOutput?.(step.frame, {
            packetIndex: packet.index,
            codedFrameIndex: codedIndex - 1,
            outputIndex,
            selectedIndex,
          });
          if (selectedOutputFrames >= normalizedWindow.outputFrames) {
            return stats();
          }
        }
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

function normalizeProgressFrames(progressFrames: number | undefined): number | undefined {
  if (progressFrames === undefined) {
    return undefined;
  }
  if (!Number.isSafeInteger(progressFrames) || progressFrames <= 0) {
    throw new Error(`invalid progress frame interval: ${progressFrames}`);
  }
  return progressFrames;
}

function normalizeDecodeWindow(window: DecodeWindow): DecodeWindow {
  const outputOffset = validNonNegativeInteger("benchmark output offset", window.outputOffset);
  const outputFrames = validPositiveIntegerValue("benchmark output frames", window.outputFrames);
  return { outputOffset, outputFrames };
}

function validateGoldenWindow(golden: GoldenFrame[], window: DecodeWindow): void {
  const end = checkedAdd(window.outputOffset, window.outputFrames, "benchmark output window");
  if (end > golden.length) {
    throw new Error(
      `benchmark output window ${window.outputOffset}..${end} exceeds golden frame count ${golden.length}`,
    );
  }
}

function runTimedDecodePasses(
  input: DemuxedVp9,
  golden: GoldenFrame[],
  window: DecodeWindow,
  makeDecoder: () => FrameDecoder,
  targetMs: number,
  now: () => number,
  runAtLeastOnce: boolean,
): BenchmarkTimedPasses {
  const start = now();
  let elapsedMs = 0;
  let passes = 0;
  let codedFrames = 0;
  let decodedOutputFrames = 0;
  let outputFrames = 0;
  let skippedOutputFrames = 0;

  while ((runAtLeastOnce && passes === 0) || elapsedMs < targetMs) {
    const stats = decodeVp9Window(input, golden, makeDecoder(), window);
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
    elapsedMs = now() - start;
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
    codedFrames,
    decodedOutputFrames,
    outputFrames,
    skippedOutputFrames,
    codedFramesPerPass: passes === 0 ? 0 : codedFrames / passes,
    outputFramesPerPass: passes === 0 ? 0 : outputFrames / passes,
    ...frameRate(outputFrames, elapsedMs),
  };
}

function frameRate(frames: number, elapsedMs: number): { msPerFrame: number; fps: number } {
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

function parseNonNegativeInteger(value: string, name: string): number {
  if (!/^(0|[1-9]\d*)$/.test(value)) {
    throw new Error(`${name} must be a non-negative integer`);
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

function parseBenchmarkFrameRange(value: string): DecodeWindow {
  const match = /^(\d+):(\d+)$/.exec(value);
  if (match === null) {
    throw new Error("--bench-frames must be START:COUNT with non-negative integer start and positive count");
  }
  const start = parseSafeInteger(match[1], "--bench-frames start");
  const count = parseSafeInteger(match[2], "--bench-frames count");
  if (count <= 0) {
    throw new Error("--bench-frames count must be positive");
  }
  return {
    outputOffset: start,
    outputFrames: count,
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

export const compareDecodedIvfToGolden = compareDecodedVp9ToGolden;

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
  throw new Error("unsupported input container: expected IVF DKIF or WebM EBML");
}

export function parseIvf(data: Uint8Array): IvfFile {
  if (data.byteLength < 32) {
    throw new Error("IVF header is truncated");
  }
  if (ascii(data, 0, 4) !== "DKIF") {
    throw new Error("IVF signature is not DKIF");
  }

  const version = le16(data, 4);
  if (version !== 0) {
    throw new Error(`unsupported IVF version: ${version}`);
  }

  const headerLength = le16(data, 6);
  if (headerLength < 32) {
    throw new Error(`IVF header length is too small: ${headerLength}`);
  }
  if (data.byteLength < headerLength) {
    throw new Error(`IVF header length exceeds file size: ${headerLength}`);
  }

  const fourcc = ascii(data, 8, 12);
  if (fourcc !== "VP90") {
    throw new Error(`unsupported IVF fourcc: ${fourcc}`);
  }

  const width = le16(data, 12);
  const height = le16(data, 14);

  const packets: IvfPacket[] = [];
  let offset = headerLength;
  while (offset < data.byteLength) {
    const index = packets.length;
    if (data.byteLength - offset < 12) {
      throw new Error(`packet ${index} header is truncated`);
    }

    const len = le32(data, offset);
    const timestamp = le64(data, offset + 4);
    const payloadStart = offset + 12;
    const payloadEnd = payloadStart + len;
    if (payloadEnd > data.byteLength) {
      throw new Error(`packet ${index} payload is truncated`);
    }
    packets.push({
      index,
      timestamp,
      payload: data.subarray(payloadStart, payloadEnd),
    });
    offset = payloadEnd;
  }

  if (packets.length === 0) {
    throw new Error("IVF contains no packets");
  }

  return {
    container: "ivf",
    codec: "VP90",
    fourcc,
    width,
    height,
    timebaseDenominator: le32(data, 16),
    timebaseNumerator: le32(data, 20),
    declaredFrameCount: le32(data, 24),
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
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width <= 0 || height <= 0) {
    throw new Error(
      `decoder dimensions unavailable: container=${input.width}x${input.height}, golden=${
        maxDimensions === undefined ? "none" : `${maxDimensions.width}x${maxDimensions.height}`
      }`,
    );
  }
  return { width, height };
}

export function maxGoldenDimensions(golden: GoldenFrame[]): { width: number; height: number } | undefined {
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

function comparedGoldenDimensions(golden: GoldenFrame[]): Set<string> | undefined {
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

function dimensionsFromGoldenName(name: string): { width: number; height: number } | undefined {
  const xSeparated = /(?:^|[-_])(\d+)x(\d+)-\d+\.i420$/i.exec(name);
  const dashSeparated = /(?:^|[-_])(\d+)-(\d+)-\d+\.i420$/i.exec(name);
  const match = xSeparated ?? dashSeparated;
  if (match === null) {
    return undefined;
  }

  const width = Number(match[1]);
  const height = Number(match[2]);
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width <= 0 || height <= 0) {
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
    (comparison) => comparison.expectedMd5 !== undefined && !isMatch(comparison),
  ).length;
}

export function missingCount(report: ComparisonReport): number {
  return Math.max(report.expectedCount - report.comparisons.length, 0);
}

export function extraCount(report: ComparisonReport): number {
  return Math.max(report.comparisons.length - report.expectedCount, 0);
}

export function passes(report: ComparisonReport, allowMismatch: boolean): boolean {
  const matches = mismatchCount(report) === 0 && missingCount(report) === 0 && extraCount(report) === 0;
  return matches || (allowMismatch && missingCount(report) === 0 && extraCount(report) === 0);
}

export function formatReport(report: ComparisonReport): string[] {
  const lines = [
    `input: ${report.inputPath}`,
    `golden: ${report.goldenPath}`,
    formatContainerLine(report),
    `decoder: coded_frames=${report.codedFrames} decoded_outputs=${report.decodedOutputFrames} compared_frames=${report.comparisons.length} skipped_outputs=${report.skippedOutputFrames}`,
    `frames: ${matchedCount(report)} matched, ${mismatchCount(report)} mismatched, ${missingCount(report)} missing, ${extraCount(report)} extra`,
  ];

  for (const comparison of report.comparisons.filter((comparison) => !isMatch(comparison)).slice(0, 10)) {
    lines.push(
      `mismatch frame ${comparison.frameNumber} ${comparison.expectedName ?? "<extra>"}: expected ${comparison.expectedMd5 ?? "<none>"}, actual ${comparison.actualMd5}, size=${comparison.decodedWidth}x${comparison.decodedHeight} render=${comparison.renderWidth}x${comparison.renderHeight}`,
    );
  }

  return lines;
}

export function formatProgress(event: ProgressEvent): string {
  const percent =
    event.expectedCount > 0 ? ` ${(event.comparedFrames / event.expectedCount * 100).toFixed(1)}%` : "";
  return `progress: compared=${event.comparedFrames}/${event.expectedCount}${percent} decoded_outputs=${event.decodedOutputFrames} coded_frames=${event.codedFrames} packet=${event.packetIndex + 1}/${event.packetCount}`;
}

function formatContainerLine(report: ComparisonReport): string {
  if (report.container === "ivf") {
    return `ivf: fourcc=${report.codec} size=${report.width}x${report.height} timebase=${report.timebaseNumerator}/${report.timebaseDenominator} declared_frames=${report.declaredFrameCount} packets=${report.packetCount}`;
  }
  return `webm: codec=${report.codec} size=${report.width}x${report.height} timestamp_scale=${report.timestampScale} packets=${report.packetCount}`;
}

export function compactI420(decoder: FrameDecoder, frame: NativeFrame): Uint8Array {
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
    throw new Error(`compact I420 length mismatch: wrote ${offset}, expected ${output.byteLength}`);
  }
  return output;
}

function planeBytes(decoder: FrameDecoder, plane: Plane, name: string): Uint8Array {
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
    throw new Error(`invalid ${name} plane: data too short ${input.byteLength} < ${requiredInput}`);
  }

  const requiredOutput = checkedMul(width, height, `${name} plane output length`);
  if (output.byteLength - outputOffset < requiredOutput) {
    throw new Error(`compact I420 output too small: ${output.byteLength - outputOffset} < ${requiredOutput}`);
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
      b = (b + rotateLeft((a + f + MD5_K[index] + words[g]) >>> 0, MD5_S[index])) >>> 0;
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
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function requiredI420Length(width: number, height: number): number {
  const chromaWidth = Math.ceil(width / 2);
  const chromaHeight = Math.ceil(height / 2);
  const luma = checkedMul(width, height, "I420 luma length");
  const chroma = checkedMul(chromaWidth, chromaHeight, "I420 chroma length");
  return checkedAdd(luma, checkedMul(2, chroma, "I420 chroma pair length"), "I420 length");
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

function le16(data: Uint8Array, offset: number): number {
  return data[offset] | (data[offset + 1] << 8);
}

function le32(data: Uint8Array, offset: number): number {
  return (
    data[offset] |
    (data[offset + 1] << 8) |
    (data[offset + 2] << 16) |
    (data[offset + 3] << 24)
  ) >>> 0;
}

function le64(data: Uint8Array, offset: number): bigint {
  return BigInt(le32(data, offset)) | (BigInt(le32(data, offset + 4)) << 32n);
}

const MD5_S = [
  7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
  14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
  21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const MD5_K = Array.from({ length: 64 }, (_, index) =>
  Math.floor(Math.abs(Math.sin(index + 1)) * 0x1_0000_0000) >>> 0,
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
): Vp9Decoder {
  let instance: WebAssembly.Instance | undefined;
  const imports = makeVip9rImports(() => {
    if (instance === undefined) {
      throw new Error("vip9r_log called before wasm instance was assigned");
    }
    return instanceMemory(instance);
  }, log);
  instance = new WebAssembly.Instance(wasm, imports);
  return new Vp9Decoder(instance, decoderDimensions.width, decoderDimensions.height);
}

function monotonicNow(): number {
  if (typeof performance === "object" && typeof performance.now === "function") {
    return performance.now();
  }
  return Date.now();
}

const utf8Decoder = typeof TextDecoder === "function" ? new TextDecoder("utf-8") : undefined;

function makeVip9rImports(
  getMemory: () => WebAssembly.Memory,
  sink: WasmLogSink,
): WebAssembly.Imports {
  return {
    env: {
      vip9r_log(kind: number, ptr: number, len: number): void {
        const memory = getMemory();
        if (!Number.isInteger(ptr) || !Number.isInteger(len) || ptr < 0 || len < 0) {
          throw new Error(`invalid wasm log span: ptr=${ptr} len=${len}`);
        }
        if (ptr + len > memory.buffer.byteLength) {
          throw new Error(`wasm log span out of bounds: ptr=${ptr} len=${len}`);
        }
        const bytes = new Uint8Array(memory.buffer, ptr, len);
        sink({ kind, message: decodeUtf8(bytes) });
      },
    },
  };
}

function instanceMemory(instance: WebAssembly.Instance): WebAssembly.Memory {
  const memory = instance.exports.memory;
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("missing wasm export: memory");
  }
  return memory;
}

function decodeUtf8(bytes: Uint8Array): string {
  if (utf8Decoder !== undefined) {
    return utf8Decoder.decode(bytes);
  }

  let text = "";
  for (const byte of bytes) {
    text += String.fromCharCode(byte);
  }
  return text;
}

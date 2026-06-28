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

export type DriverArgs = {
  allowMismatch: boolean;
  wasmPath: string;
  inputPath: string;
  goldenPath: string;
  progressFrames?: number;
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
};

export type FrameDecoder = {
  beginPacket(packet: Uint8Array): void;
  decodeNext(): DecodeStep;
  planeBytes(plane: Plane): Uint8Array;
};

export function compareWasmToGolden(args: DriverArgs, io: GoldenIo): ComparisonReport {
  const wasm = new WebAssembly.Module(io.readbuffer(args.wasmPath));
  let instance: WebAssembly.Instance | undefined;
  const imports = makeVip9rImports(() => {
    if (instance === undefined) {
      throw new Error("vip9r_log called before wasm instance was assigned");
    }
    return instanceMemory(instance);
  }, io.log);
  instance = new WebAssembly.Instance(wasm, imports);
  const input = parseVp9Input(new Uint8Array(io.readbuffer(args.inputPath)));
  const golden = parseGolden(io.read(args.goldenPath));
  const decoderDimensions = decoderDimensionsForGolden(input, golden);
  const decoder = new Vp9Decoder(instance, decoderDimensions.width, decoderDimensions.height);

  return compareDecodedVp9ToGolden(args.inputPath, args.goldenPath, input, golden, decoder, {
    progressFrames: args.progressFrames,
    onProgress: io.progress,
  });
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

function normalizeProgressFrames(progressFrames: number | undefined): number | undefined {
  if (progressFrames === undefined) {
    return undefined;
  }
  if (!Number.isSafeInteger(progressFrames) || progressFrames <= 0) {
    throw new Error(`invalid progress frame interval: ${progressFrames}`);
  }
  return progressFrames;
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

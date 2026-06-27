import { Vp9Decoder } from "../wasm";
import type { NativeFrame } from "../wasm";

declare const read: (path: string) => string;
declare const readbuffer: (path: string) => ArrayBuffer;
declare const print: (...values: unknown[]) => void;
declare const quit: (code?: number) => never;

type D8Global = typeof globalThis & {
  arguments?: string[];
  scriptArgs?: string[];
};

type IvfPacket = {
  index: number;
  timestamp: number;
  payload: Uint8Array;
};

type IvfFile = {
  width: number;
  height: number;
  packets: IvfPacket[];
};

type GoldenFrame = {
  md5: string;
  name: string;
};

type DriverArgs = {
  allowMismatch: boolean;
  wasmPath: string;
  inputPath: string;
  goldenPath: string;
};

type FrameComparison = {
  frameNumber: number;
  expectedMd5?: string;
  expectedName?: string;
  actualMd5: string;
  decodedWidth: number;
  decodedHeight: number;
  renderWidth: number;
  renderHeight: number;
};

function main(args: string[]): void {
  const { allowMismatch, wasmPath, inputPath, goldenPath } = parseArgs(args);
  const wasm = new WebAssembly.Module(readbuffer(wasmPath));
  const instance = new WebAssembly.Instance(wasm, {});
  const ivf = parseIvf(new Uint8Array(readbuffer(inputPath)));
  const golden = parseGolden(read(goldenPath));
  const decoder = new Vp9Decoder(instance, ivf.width, ivf.height);

  const comparisons: FrameComparison[] = [];
  let coded = 0;
  for (const packet of ivf.packets) {
    decoder.beginPacket(packet.payload);
    while (true) {
      const step = decoder.decodeNext();
      coded += 1;
      if (step.kind === "output") {
        const actual = md5Hex(compactI420(decoder, step.frame));
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
      }
      if (step.packetDone) {
        break;
      }
    }
  }

  const report = makeReport(inputPath, goldenPath, ivf, coded, comparisons, golden.length);
  printReport(report);

  if (!passes(report, allowMismatch)) {
    throw new Error(
      `golden mismatch: ${matchedCount(report)} matched, ${mismatchCount(report)} mismatched, ${missingCount(report)} missing, ${extraCount(report)} extra`,
    );
  }
}

function parseArgs(args: string[]): DriverArgs {
  let allowMismatch = false;
  const paths: string[] = [];
  for (const arg of args) {
    if (arg === "-h" || arg === "--help") {
      printUsage();
      quit(0);
    }
    if (arg === "--allow-mismatch") {
      allowMismatch = true;
      continue;
    }
    if (arg.startsWith("-")) {
      throw new Error(`unknown argument: ${arg}`);
    }
    paths.push(arg);
  }

  if (paths.length < 2 || paths.length > 3) {
    printUsage();
    quit(2);
  }

  const [wasmPath, inputPath, goldenPath = `${inputPath}.md5`] = paths;
  return { allowMismatch, wasmPath, inputPath, goldenPath };
}

function printUsage(): void {
  print(
    "usage: d8 dist/wasm-driver/main.js -- [--allow-mismatch] vip9r.wasm input.ivf [input.ivf.md5]",
  );
}

function parseIvf(data: Uint8Array): IvfFile {
  if (data.byteLength < 32) {
    throw new Error("IVF header is truncated");
  }
  if (ascii(data, 0, 4) !== "DKIF") {
    throw new Error("bad IVF signature");
  }
  if (ascii(data, 8, 12) !== "VP90") {
    throw new Error(`unsupported IVF fourcc: ${ascii(data, 8, 12)}`);
  }

  const width = le16(data, 12);
  const height = le16(data, 14);
  const packets: IvfPacket[] = [];
  let offset = 32;
  while (offset < data.byteLength) {
    if (offset + 12 > data.byteLength) {
      throw new Error("IVF packet header is truncated");
    }
    const len = le32(data, offset);
    const timestamp = le64(data, offset + 4);
    offset += 12;
    if (offset + len > data.byteLength) {
      throw new Error("IVF packet payload is truncated");
    }
    packets.push({
      index: packets.length,
      timestamp,
      payload: data.subarray(offset, offset + len),
    });
    offset += len;
  }

  return { width, height, packets };
}

function parseGolden(text: string): GoldenFrame[] {
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

type ComparisonReport = {
  inputPath: string;
  goldenPath: string;
  width: number;
  height: number;
  packetCount: number;
  codedFrames: number;
  comparisons: FrameComparison[];
  expectedCount: number;
};

function makeReport(
  inputPath: string,
  goldenPath: string,
  ivf: IvfFile,
  codedFrames: number,
  comparisons: FrameComparison[],
  expectedCount: number,
): ComparisonReport {
  return {
    inputPath,
    goldenPath,
    width: ivf.width,
    height: ivf.height,
    packetCount: ivf.packets.length,
    codedFrames,
    comparisons,
    expectedCount,
  };
}

function isMatch(comparison: FrameComparison): boolean {
  return comparison.expectedMd5 === comparison.actualMd5;
}

function matchedCount(report: ComparisonReport): number {
  return report.comparisons.filter(isMatch).length;
}

function mismatchCount(report: ComparisonReport): number {
  return report.comparisons.filter(
    (comparison) => comparison.expectedMd5 !== undefined && !isMatch(comparison),
  ).length;
}

function missingCount(report: ComparisonReport): number {
  return Math.max(report.expectedCount - report.comparisons.length, 0);
}

function extraCount(report: ComparisonReport): number {
  return Math.max(report.comparisons.length - report.expectedCount, 0);
}

function passes(report: ComparisonReport, allowMismatch: boolean): boolean {
  const matches = mismatchCount(report) === 0 && missingCount(report) === 0 && extraCount(report) === 0;
  return matches || (allowMismatch && missingCount(report) === 0 && extraCount(report) === 0);
}

function printReport(report: ComparisonReport): void {
  print(`input: ${report.inputPath}`);
  print(`golden: ${report.goldenPath}`);
  print(`ivf: fourcc=VP90 size=${report.width}x${report.height} packets=${report.packetCount}`);
  print(`decoder: coded_frames=${report.codedFrames} shown_frames=${report.comparisons.length}`);
  print(
    `frames: ${matchedCount(report)} matched, ${mismatchCount(report)} mismatched, ${missingCount(report)} missing, ${extraCount(report)} extra`,
  );

  for (const comparison of report.comparisons.filter((comparison) => !isMatch(comparison)).slice(0, 10)) {
    print(
      `mismatch frame ${comparison.frameNumber} ${comparison.expectedName ?? "<extra>"}: expected ${comparison.expectedMd5 ?? "<none>"}, actual ${comparison.actualMd5}, size=${comparison.decodedWidth}x${comparison.decodedHeight} render=${comparison.renderWidth}x${comparison.renderHeight}`,
    );
  }
}

function compactI420(decoder: Vp9Decoder, frame: NativeFrame): Uint8Array {
  const chromaWidth = Math.ceil(frame.decodedWidth / 2);
  const chromaHeight = Math.ceil(frame.decodedHeight / 2);
  const output = new Uint8Array(
    frame.decodedWidth * frame.decodedHeight + 2 * chromaWidth * chromaHeight,
  );
  let offset = 0;
  offset = copyPlane(
    output,
    offset,
    decoder.planeBytes(frame.y),
    frame.y.stride,
    frame.decodedWidth,
    frame.decodedHeight,
  );
  offset = copyPlane(
    output,
    offset,
    decoder.planeBytes(frame.u),
    frame.u.stride,
    chromaWidth,
    chromaHeight,
  );
  copyPlane(
    output,
    offset,
    decoder.planeBytes(frame.v),
    frame.v.stride,
    chromaWidth,
    chromaHeight,
  );
  return output;
}

function copyPlane(
  output: Uint8Array,
  outputOffset: number,
  input: Uint8Array,
  stride: number,
  width: number,
  height: number,
): number {
  for (let row = 0; row < height; row += 1) {
    const inputStart = row * stride;
    output.set(input.subarray(inputStart, inputStart + width), outputOffset);
    outputOffset += width;
  }
  return outputOffset;
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

function le64(data: Uint8Array, offset: number): number {
  return le32(data, offset) + le32(data, offset + 4) * 0x1_0000_0000;
}

const MD5_S = [
  7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
  14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
  21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const MD5_K = Array.from({ length: 64 }, (_, index) =>
  Math.floor(Math.abs(Math.sin(index + 1)) * 0x1_0000_0000) >>> 0,
);

function md5Hex(input: Uint8Array): string {
  const paddedLen = (((input.byteLength + 8) >>> 6) + 1) * 64;
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

function rotateLeft(value: number, bits: number): number {
  return (value << bits) | (value >>> (32 - bits));
}

function writeLe32(data: Uint8Array, offset: number, value: number): void {
  data[offset] = value & 0xff;
  data[offset + 1] = (value >>> 8) & 0xff;
  data[offset + 2] = (value >>> 16) & 0xff;
  data[offset + 3] = (value >>> 24) & 0xff;
}

try {
  const d8 = globalThis as D8Global;
  main(d8.scriptArgs ?? d8.arguments ?? []);
} catch (error) {
  print(error instanceof Error && error.stack ? error.stack : String(error));
  quit(1);
}

import { describe, expect, test } from "vitest";

import type { ComparisonReport, FrameDecoder } from "./golden";
import {
  compactI420,
  compareDecodedIvfToGolden,
  formatReport,
  md5Hex,
  parseGolden,
  parseIvf,
  passes,
} from "./golden";
import type { NativeFrame, Plane } from "../wasm";

describe("wasm golden runner helpers", () => {
  test("parses IVF headers and packets, including extended headers", () => {
    const ivf = parseIvf(sampleIvf({ headerLength: 36 }));

    expect(ivf.fourcc).toBe("VP90");
    expect(ivf.width).toBe(320);
    expect(ivf.height).toBe(240);
    expect(ivf.timebaseDenominator).toBe(1000);
    expect(ivf.timebaseNumerator).toBe(1);
    expect(ivf.declaredFrameCount).toBe(2);
    expect(ivf.packets).toHaveLength(2);
    expect(ivf.packets[0].timestamp).toBe(0n);
    expect([...ivf.packets[0].payload]).toEqual([1, 2, 3]);
    expect(ivf.packets[1].timestamp).toBe(1n);
    expect([...ivf.packets[1].payload]).toEqual([4, 5]);
  });

  test("rejects malformed IVF files", () => {
    const truncatedPayload = sampleIvf();
    expect(() => parseIvf(truncatedPayload.subarray(0, truncatedPayload.byteLength - 1))).toThrow(
      "packet 1 payload is truncated",
    );

    const badHeaderLength = sampleIvf();
    badHeaderLength[6] = 31;
    badHeaderLength[7] = 0;
    expect(() => parseIvf(badHeaderLength)).toThrow("IVF header length is too small: 31");

    const zeroWidth = sampleIvf();
    zeroWidth[12] = 0;
    zeroWidth[13] = 0;
    expect(() => parseIvf(zeroWidth)).toThrow("IVF dimensions must be non-zero: 0x240");
  });

  test("parses libvpx md5 sidecars", () => {
    const frames = parseGolden(
      "4FF2537E44588E6473E236D8A6FC0054  img-320-240-0001.i420\n" +
        "8328efce9d9580304a3833a26a23321a  img-320-240-0002.i420\n",
    );

    expect(frames).toEqual([
      {
        md5: "4ff2537e44588e6473e236d8a6fc0054",
        name: "img-320-240-0001.i420",
      },
      {
        md5: "8328efce9d9580304a3833a26a23321a",
        name: "img-320-240-0002.i420",
      },
    ]);
  });

  test("formats IVF timebase and declared frame metadata", () => {
    expect(formatReport(sampleReport([], 0))).toContain(
      "ivf: fourcc=VP90 size=320x240 timebase=1/1000 declared_frames=2 packets=2",
    );
  });

  test("md5 implementation matches known vectors", () => {
    expect(md5Hex(new Uint8Array())).toBe("d41d8cd98f00b204e9800998ecf8427e");
    expect(md5Hex(new Uint8Array([0x61, 0x62, 0x63]))).toBe("900150983cd24fb0d6963f7d28e17f72");
  });

  test("compact I420 strips stride and orders planes as Y, U, V", () => {
    const frame = frameWithPlanes(
      { data: [1, 2, 3, 99, 4, 5, 6, 99, 7, 8, 9, 99], stride: 4 },
      { data: [10, 11, 99, 12, 13, 99], stride: 3 },
      { data: [14, 15, 99, 16, 17, 99], stride: 3 },
    );

    expect([...compactI420(frame.decoder, frame.native)]).toEqual([
      1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17,
    ]);
  });

  test("compact I420 rejects invalid plane layout", () => {
    const frame = frameWithPlanes(
      { data: [1, 2, 3, 99, 4, 5, 6, 99, 7, 8, 9, 99], stride: 4 },
      { data: [10, 11, 12, 13], stride: 1 },
      { data: [14, 15, 99, 16, 17, 99], stride: 3 },
    );

    expect(() => compactI420(frame.decoder, frame.native)).toThrow("invalid U plane: stride 1 < width 2");
  });

  test("allow-mismatch still rejects missing and extra shown frames", () => {
    let report = sampleReport(
      [
        {
          frameNumber: 1,
          expectedMd5: "expected",
          expectedName: "frame.i420",
          actualMd5: "actual",
          decodedWidth: 2,
          decodedHeight: 2,
          renderWidth: 2,
          renderHeight: 2,
        },
      ],
      1,
    );
    expect(passes(report, false)).toBe(false);
    expect(passes(report, true)).toBe(true);

    report = { ...report, expectedCount: 2 };
    expect(passes(report, true)).toBe(false);

    report = { ...report, expectedCount: 0 };
    expect(passes(report, true)).toBe(false);
  });

  test("adds packet context to begin-packet failures without wasm ABI help", () => {
    const ivf = parseIvf(sampleIvf());
    const golden = parseGolden("d41d8cd98f00b204e9800998ecf8427e  frame.i420\n");
    const decoder: FrameDecoder = {
      beginPacket() {
        throw new Error("bad packet");
      },
      decodeNext() {
        throw new Error("unreachable");
      },
      planeBytes() {
        throw new Error("unreachable");
      },
    };

    expect(() => compareDecodedIvfToGolden("input.ivf", "input.ivf.md5", ivf, golden, decoder)).toThrow(
      "decode packet 0 timestamp 0: bad packet",
    );
  });

  test("adds coded-frame context to decode failures without wasm ABI help", () => {
    const ivf = parseIvf(sampleIvf());
    const golden = parseGolden("d41d8cd98f00b204e9800998ecf8427e  frame.i420\n");
    const decoder: FrameDecoder = {
      beginPacket() {},
      decodeNext() {
        throw new Error("bad coded frame");
      },
      planeBytes() {
        throw new Error("unreachable");
      },
    };

    expect(() => compareDecodedIvfToGolden("input.ivf", "input.ivf.md5", ivf, golden, decoder)).toThrow(
      "decode packet 0 coded frame 0: bad coded frame",
    );
  });
});

function sampleReport(comparisons: ComparisonReport["comparisons"], expectedCount: number): ComparisonReport {
  return {
    inputPath: "input.ivf",
    goldenPath: "input.ivf.md5",
    fourcc: "VP90",
    width: 320,
    height: 240,
    timebaseDenominator: 1000,
    timebaseNumerator: 1,
    declaredFrameCount: 2,
    packetCount: 2,
    codedFrames: 2,
    comparisons,
    expectedCount,
  };
}

function frameWithPlanes(
  y: { data: number[]; stride: number },
  u: { data: number[]; stride: number },
  v: { data: number[]; stride: number },
): { decoder: FrameDecoder; native: NativeFrame } {
  const planeBytes = new Map<number, Uint8Array>([
    [1, new Uint8Array(y.data)],
    [2, new Uint8Array(u.data)],
    [3, new Uint8Array(v.data)],
  ]);
  const decoder: FrameDecoder = {
    beginPacket() {
      throw new Error("not used");
    },
    decodeNext() {
      throw new Error("not used");
    },
    planeBytes(plane: Plane) {
      const bytes = planeBytes.get(plane.offset);
      if (!bytes) {
        throw new Error(`unknown plane offset ${plane.offset}`);
      }
      return bytes.subarray(0, plane.byteLength);
    },
  };
  const native: NativeFrame = {
    decodedWidth: 3,
    decodedHeight: 3,
    renderWidth: 3,
    renderHeight: 3,
    y: { offset: 1, byteLength: y.data.length, stride: y.stride },
    u: { offset: 2, byteLength: u.data.length, stride: u.stride },
    v: { offset: 3, byteLength: v.data.length, stride: v.stride },
  };
  return { decoder, native };
}

function sampleIvf(options: { headerLength?: number } = {}): Uint8Array {
  const headerLength = options.headerLength ?? 32;
  const bytes: number[] = [];
  ascii(bytes, "DKIF");
  le16(bytes, 0);
  le16(bytes, headerLength);
  ascii(bytes, "VP90");
  le16(bytes, 320);
  le16(bytes, 240);
  le32(bytes, 1000);
  le32(bytes, 1);
  le32(bytes, 2);
  le32(bytes, 0);
  while (bytes.length < headerLength) {
    bytes.push(0);
  }
  packet(bytes, 0n, [1, 2, 3]);
  packet(bytes, 1n, [4, 5]);
  return new Uint8Array(bytes);
}

function packet(bytes: number[], timestamp: bigint, payload: number[]): void {
  le32(bytes, payload.length);
  le64(bytes, timestamp);
  bytes.push(...payload);
}

function ascii(bytes: number[], text: string): void {
  for (let index = 0; index < text.length; index += 1) {
    bytes.push(text.charCodeAt(index));
  }
}

function le16(bytes: number[], value: number): void {
  bytes.push(value & 0xff, (value >>> 8) & 0xff);
}

function le32(bytes: number[], value: number): void {
  bytes.push(value & 0xff, (value >>> 8) & 0xff, (value >>> 16) & 0xff, (value >>> 24) & 0xff);
}

function le64(bytes: number[], value: bigint): void {
  le32(bytes, Number(value & 0xffff_ffffn));
  le32(bytes, Number(value >> 32n));
}

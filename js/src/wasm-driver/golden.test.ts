import { describe, expect, test } from "vitest";

import type { ComparisonReport, DemuxedVp9, FrameDecoder } from "./golden";
import {
  compactI420,
  compareDecodedVp9WindowToGolden,
  compareDecodedVp9ToGolden,
  decoderDimensionsForGolden,
  decodeVp9Window,
  DEFAULT_BENCHMARK_OPTIONS,
  formatProgress,
  formatReport,
  matchedCount,
  maxGoldenDimensions,
  md5Hex,
  missingCount,
  parseGolden,
  parseDriverArgs,
  parseIvf,
  parseVp9Input,
  planBenchmarkDecode,
  passes,
  runTimedDecodePasses,
  runTimedWarmupValidation,
} from "./golden";
import type { DecodeStep, NativeFrame, Plane } from "../wasm";

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

  test("preserves zero IVF dimensions for sidecar-sized vectors", () => {
    const ivf = parseIvf(sampleIvf({ width: 0, height: 0 }));

    expect(ivf.width).toBe(0);
    expect(ivf.height).toBe(0);
    expect(ivf.packets).toHaveLength(2);
  });

  test("dispatches IVF and WebM inputs by magic bytes", () => {
    const ivf = parseVp9Input(sampleIvf());
    expect(ivf).toMatchObject({
      container: "ivf",
      codec: "VP90",
      width: 320,
      height: 240,
    });

    const webm = parseVp9Input(sampleWebm());
    expect(webm).toMatchObject({
      container: "webm",
      codec: "V_VP9",
      width: 160,
      height: 90,
      timestampScale: 1_000_000,
    });
    expect(webm.packets).toHaveLength(1);
    expect([...webm.packets[0].payload]).toEqual([9, 8, 7]);

    expect(() => parseVp9Input(new Uint8Array([1, 2, 3, 4]))).toThrow(
      "unsupported input container: expected IVF DKIF or WebM EBML",
    );
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

    const noPackets = sampleIvf().subarray(0, 32);
    expect(() => parseIvf(noPackets)).toThrow("IVF contains no packets");
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

  test("formats WebM timestamp-scale metadata", () => {
    expect(
      formatReport({
        ...sampleReport([], 0),
        container: "webm",
        codec: "V_VP9",
        timestampScale: 1_000_000,
        declaredFrameCount: undefined,
        timebaseDenominator: undefined,
        timebaseNumerator: undefined,
      }),
    ).toContain("webm: codec=V_VP9 size=320x240 timestamp_scale=1000000 packets=2");
  });

  test("reports optional compared-frame progress", () => {
    const ivf = parseIvf(sampleIvf());
    const frames = [
      scriptedFrame(2, 2, 1, 2, 3, 1),
      scriptedFrame(2, 2, 4, 5, 6, 10),
      scriptedFrame(2, 2, 7, 8, 9, 20),
      scriptedFrame(2, 2, 10, 11, 12, 30),
    ];
    const golden = parseGolden(
      frames.map((frame, index) => `${md5Hex(frame.compact)}  frame-000${index + 1}.i420`).join("\n"),
    );
    const progress: string[] = [];

    const report = compareDecodedVp9ToGolden("input.ivf", "input.ivf.md5", ivf, golden, scriptedDecoder(frames), {
      progressFrames: 2,
      onProgress(event) {
        progress.push(formatProgress(event));
      },
    });

    expect(passes(report, false)).toBe(true);
    expect(progress).toEqual([
      "progress: compared=2/4 50.0% decoded_outputs=2 coded_frames=2 packet=1/2",
      "progress: compared=4/4 100.0% decoded_outputs=4 coded_frames=4 packet=2/2",
    ]);
  });

  test("parses default and benchmark driver arguments", () => {
    expect(parseDriverArgs(["vip9r.wasm", "input.ivf"])).toMatchObject({
      allowMismatch: false,
      wasmPath: "vip9r.wasm",
      inputPath: "input.ivf",
      goldenPath: "input.ivf.md5",
      bench: undefined,
    });
    expect(parseDriverArgs(["vip9r.wasm"])).toMatchObject({
      wasmPath: "vip9r.wasm",
      inputPath: "/bulk/vip9r/chromium/bear-vp9.ivf",
      goldenPath: "/bulk/vip9r/chromium/bear-vp9.ivf.md5",
      bench: undefined,
    });
    expect(parseDriverArgs(["--frames", "2:6", "vip9r.wasm", "input.ivf"])).toMatchObject({
      wasmPath: "vip9r.wasm",
      inputPath: "input.ivf",
      goldenPath: "input.ivf.md5",
      frames: {
        outputOffset: 2,
        outputFrames: 5,
      },
      bench: undefined,
    });

    expect(
      parseDriverArgs([
        "--bench",
        "--frames",
        "2:6",
        "vip9r.wasm",
        "input.webm",
      ]),
    ).toMatchObject({
      allowMismatch: false,
      wasmPath: "vip9r.wasm",
      inputPath: "input.webm",
      goldenPath: "input.webm.md5",
      bench: {
        outputOffset: 2,
        outputFrames: 5,
        warmupMs: DEFAULT_BENCHMARK_OPTIONS.warmupMs,
        targetMs: DEFAULT_BENCHMARK_OPTIONS.targetMs,
      },
    });

    expect(parseDriverArgs(["--bench", "vip9r.wasm", "input.ivf"]).bench).toEqual(DEFAULT_BENCHMARK_OPTIONS);
    expect(parseDriverArgs(["--bench", "vip9r.wasm"])).toMatchObject({
      wasmPath: "vip9r.wasm",
      inputPath: "/bulk/vip9r/chromium/bear-vp9.ivf",
      goldenPath: "/bulk/vip9r/chromium/bear-vp9.ivf.md5",
      bench: DEFAULT_BENCHMARK_OPTIONS,
    });
  });

  test("rejects benchmark options that would make output non-benchmark or non-machine-readable", () => {
    expect(() => parseDriverArgs(["--bench", "--allow-mismatch", "vip9r.wasm", "input.ivf"])).toThrow(
      "--allow-mismatch cannot be used with --bench",
    );
    expect(() => parseDriverArgs(["--bench", "--progress-frames=1", "vip9r.wasm", "input.ivf"])).toThrow(
      "--progress-frames cannot be used with --bench",
    );
  });

  test("rejects invalid validation frame selections", () => {
    expect(() => parseDriverArgs(["--frames", "2:1", "vip9r.wasm", "input.ivf"])).toThrow(
      "--frames last must be greater than or equal to start",
    );
    expect(() =>
      parseDriverArgs(["--frames", "0:1", "--progress-frames=1", "vip9r.wasm", "input.ivf"]),
    ).toThrow("--progress-frames cannot be used with --frames");
  });

  test("chooses decoder dimensions from sidecar frame names when larger than container", () => {
    const golden = parseGolden(
      "4ff2537e44588e6473e236d8a6fc0054  resize-640x240-0001.i420\n" +
        "8328efce9d9580304a3833a26a23321a  img-320-480-0002.i420\n" +
        "d41d8cd98f00b204e9800998ecf8427e  frame.i420\n",
    );

    expect(maxGoldenDimensions(golden)).toEqual({ width: 640, height: 480 });
    expect(decoderDimensionsForGolden({ width: 320, height: 240 }, golden)).toEqual({
      width: 640,
      height: 480,
    });
  });

  test("chooses decoder dimensions from sidecar frame names when IVF dimensions are zero", () => {
    const golden = parseGolden("369f3d6ce1ba7ad7bd5716d0aef8daf4  svc-1280x720-0001.i420\n");

    expect(decoderDimensionsForGolden({ width: 0, height: 0 }, golden)).toEqual({
      width: 1280,
      height: 720,
    });
  });

  test("falls back to container dimensions when sidecar names do not include dimensions", () => {
    const golden = parseGolden("d41d8cd98f00b204e9800998ecf8427e  frame.i420\n");

    expect(maxGoldenDimensions(golden)).toBeUndefined();
    expect(decoderDimensionsForGolden({ width: 320, height: 240 }, golden)).toEqual({
      width: 320,
      height: 240,
    });
  });

  test("rejects decoder dimensions when neither container nor sidecar gives a size", () => {
    const golden = parseGolden("d41d8cd98f00b204e9800998ecf8427e  frame.i420\n");

    expect(() => decoderDimensionsForGolden({ width: 0, height: 0 }, golden)).toThrow(
      "decoder dimensions unavailable: container=0x0, golden=none",
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

  test("filters decoded outputs whose dimensions are absent from dimensioned sidecar names", () => {
    const ivf = parseIvf(sampleIvf({ width: 0, height: 0 }));
    const top1 = scriptedFrame(4, 4, 10, 20, 30, 1);
    const top2 = scriptedFrame(4, 4, 11, 21, 31, 10);
    const decoder = scriptedDecoder([
      scriptedFrame(2, 2, 1, 2, 3, 20),
      top1,
      scriptedFrame(2, 2, 4, 5, 6, 30),
      top2,
    ]);
    const golden = parseGolden(
      `${md5Hex(top1.compact)}  svc-4x4-0001.i420\n` +
        `${md5Hex(top2.compact)}  svc-4x4-0002.i420\n`,
    );

    const report = compareDecodedVp9ToGolden("input.ivf", "input.ivf.md5", ivf, golden, decoder);

    expect(report.decodedOutputFrames).toBe(4);
    expect(report.skippedOutputFrames).toBe(2);
    expect(report.comparisons).toHaveLength(2);
    expect(passes(report, false)).toBe(true);
  });

  test("decodes benchmark windows from stream start without touching output planes", () => {
    const ivf = parseIvf(sampleIvf());
    const frames = [
      scriptedFrame(2, 2, 1, 2, 3, 1),
      scriptedFrame(2, 2, 4, 5, 6, 10),
      scriptedFrame(2, 2, 7, 8, 9, 20),
      scriptedFrame(2, 2, 10, 11, 12, 30),
    ];
    const golden = parseGolden(
      frames.map((frame, index) => `${md5Hex(frame.compact)}  frame-000${index + 1}.i420`).join("\n"),
    );
    const decoder: FrameDecoder = {
      ...scriptedDecoder(frames),
      planeBytes() {
        throw new Error("plane bytes should not be read");
      },
    };
    const stats = decodeVp9Window(ivf, golden, decoder, {
      outputOffset: 1,
      outputFrames: 2,
    });

    expect(stats).toEqual({
      codedFrames: 3,
      decodedOutputFrames: 3,
      skippedOutputFrames: 0,
      selectedOutputFrames: 2,
    });
  });

  test("aborts benchmark window decode when pass deadline expires", () => {
    const ivf = parseIvf(sampleIvf());
    const frames = [
      scriptedFrame(2, 2, 1, 2, 3, 1),
      scriptedFrame(2, 2, 4, 5, 6, 10),
      scriptedFrame(2, 2, 7, 8, 9, 20),
      scriptedFrame(2, 2, 10, 11, 12, 30),
    ];
    const nowSamples = [0, 1, 6];
    const now = () => {
      const sample = nowSamples.shift();
      if (sample === undefined) {
        throw new Error("unexpected clock read");
      }
      return sample;
    };

    expect(() =>
      decodeVp9Window(
        ivf,
        [],
        scriptedDecoder(frames),
        {
          outputOffset: 0,
          outputFrames: 4,
        },
        undefined,
        {
          phase: "measurement",
          pass: 1,
          startMs: 0,
          limitMs: 5,
          now,
        },
      ),
    ).toThrow(
      "benchmark measurement pass 1 exceeded 5ms before completing decode window: selected 2/4 output frames",
    );
  });

  test("aborts benchmark window validation when pass deadline expires", () => {
    const ivf = parseIvf(sampleIvf());
    const frames = [scriptedFrame(2, 2, 1, 2, 3, 1), scriptedFrame(2, 2, 4, 5, 6, 10)];
    const golden = parseGolden(
      frames.map((frame, index) => `${md5Hex(frame.compact)}  frame-000${index + 1}.i420`).join("\n"),
    );
    const nowSamples = [0, 6];
    const now = () => {
      const sample = nowSamples.shift();
      if (sample === undefined) {
        throw new Error("unexpected clock read");
      }
      return sample;
    };

    expect(() =>
      compareDecodedVp9WindowToGolden(
        "input.ivf",
        "input.ivf.md5",
        ivf,
        golden,
        scriptedDecoder(frames),
        {
          outputOffset: 0,
          outputFrames: 2,
        },
        {
          deadline: {
            phase: "warmup validation",
            pass: 1,
            startMs: 0,
            limitMs: 5,
            now,
          },
        },
      ),
    ).toThrow(
      "benchmark warmup validation pass 1 exceeded 5ms before completing decode window: selected 1/2 output frames",
    );
  });

  test("records per-pass wall times across measurement passes", () => {
    const ivf = parseIvf(sampleIvf());
    const frames = [
      scriptedFrame(2, 2, 1, 2, 3, 1),
      scriptedFrame(2, 2, 4, 5, 6, 10),
      scriptedFrame(2, 2, 7, 8, 9, 20),
      scriptedFrame(2, 2, 10, 11, 12, 30),
    ];
    const golden = parseGolden(
      frames.map((frame, index) => `${md5Hex(frame.compact)}  frame-000${index + 1}.i420`).join("\n"),
    );
    let timeMs = 0;
    const now = () => timeMs;
    const perFrameMs = [5, 10];
    let pass = 0;
    const makeDecoder = (): FrameDecoder => {
      const stepMs = perFrameMs[pass];
      pass += 1;
      const inner = scriptedDecoder(frames);
      return {
        ...inner,
        decodeNext() {
          timeMs += stepMs;
          return inner.decodeNext();
        },
      };
    };

    const measurement = runTimedDecodePasses(
      ivf,
      golden,
      { outputOffset: 0, outputFrames: 4 },
      makeDecoder,
      30,
      1000,
      now,
      true,
      "measurement",
    );

    expect(measurement.passes).toBe(2);
    expect(measurement.passMs).toEqual([20, 40]);
    expect(measurement.minPassMs).toBe(20);
    expect(measurement.maxPassMs).toBe(40);
    expect(measurement.elapsedMs).toBe(60);
    expect(measurement.msPerFrame).toBe(7.5);
  });

  test("records per-pass wall times including the warmup validation pass", () => {
    const ivf = parseIvf(sampleIvf());
    const frames = [
      scriptedFrame(2, 2, 1, 2, 3, 1),
      scriptedFrame(2, 2, 4, 5, 6, 10),
      scriptedFrame(2, 2, 7, 8, 9, 20),
      scriptedFrame(2, 2, 10, 11, 12, 30),
    ];
    const golden = parseGolden(
      frames.map((frame, index) => `${md5Hex(frame.compact)}  frame-000${index + 1}.i420`).join("\n"),
    );
    let timeMs = 0;
    const now = () => timeMs;
    const perFrameMs = [5, 10];
    let pass = 0;
    const makeDecoder = (): FrameDecoder => {
      const stepMs = perFrameMs[pass];
      pass += 1;
      const inner = scriptedDecoder(frames);
      return {
        ...inner,
        decodeNext() {
          timeMs += stepMs;
          return inner.decodeNext();
        },
      };
    };

    const { validation, warmup } = runTimedWarmupValidation(
      "input.ivf",
      "input.ivf.md5",
      ivf,
      golden,
      { outputOffset: 0, outputFrames: 4 },
      0,
      makeDecoder,
      30,
      1000,
      now,
    );

    expect(matchedCount(validation)).toBe(4);
    expect(warmup.passes).toBe(2);
    expect(warmup.passMs).toEqual([20, 40]);
    expect(warmup.minPassMs).toBe(20);
    expect(warmup.maxPassMs).toBe(40);
    expect(warmup.elapsedMs).toBe(60);
  });

  test("validates only the selected benchmark output window", () => {
    const ivf = parseIvf(sampleIvf());
    const frames = [
      scriptedFrame(2, 2, 1, 2, 3, 1),
      scriptedFrame(2, 2, 4, 5, 6, 10),
      scriptedFrame(2, 2, 7, 8, 9, 20),
      scriptedFrame(2, 2, 10, 11, 12, 30),
    ];
    const golden = parseGolden(
      frames.map((frame, index) => `${md5Hex(frame.compact)}  frame-000${index + 1}.i420`).join("\n"),
    );

    const report = compareDecodedVp9WindowToGolden(
      "input.ivf",
      "input.ivf.md5",
      ivf,
      golden,
      scriptedDecoder(frames),
      {
        outputOffset: 1,
        outputFrames: 2,
      },
    );

    expect(report.comparisons.map((comparison) => comparison.frameNumber)).toEqual([2, 3]);
    expect(matchedCount(report)).toBe(2);
    expect(missingCount(report)).toBe(0);
    expect(passes(report, false)).toBe(true);
  });

  test("validates benchmark windows against an explicit golden offset", () => {
    const input = sampleDemuxedWebm([
      samplePacket(2, [7], { keyframe: true, visible: true }),
      samplePacket(3, [8], { keyframe: false, visible: true }),
    ]);
    const frames = [
      scriptedFrame(2, 2, 1, 2, 3, 1),
      scriptedFrame(2, 2, 4, 5, 6, 10),
      scriptedFrame(2, 2, 7, 8, 9, 20),
      scriptedFrame(2, 2, 10, 11, 12, 30),
    ];
    const golden = parseGolden(
      frames.map((frame, index) => `${md5Hex(frame.compact)}  frame-000${index + 1}.i420`).join("\n"),
    );

    const report = compareDecodedVp9WindowToGolden(
      "input.webm",
      "input.webm.md5",
      input,
      golden,
      scriptedDecoder(frames.slice(2)),
      {
        outputOffset: 0,
        outputFrames: 2,
      },
      { goldenOffset: 2 },
    );

    expect(report.comparisons.map((comparison) => comparison.frameNumber)).toEqual([3, 4]);
    expect(matchedCount(report)).toBe(2);
    expect(passes(report, false)).toBe(true);
  });

  test("plans nonzero WebM benchmark windows from visible keyframe packets", () => {
    const input = sampleDemuxedWebm([
      samplePacket(0, [0], { keyframe: true, visible: true }),
      samplePacket(1, [1], { keyframe: true, visible: false }),
      samplePacket(2, [2], { keyframe: false, visible: true }),
      samplePacket(3, [3], { keyframe: true, visible: true }),
      samplePacket(4, [4], { keyframe: false, visible: false }),
    ]);

    const plan = planBenchmarkDecode(input, {
      outputOffset: 2,
      outputFrames: 5,
    });

    expect(plan.goldenOffset).toBe(2);
    expect(plan.decodeWindow).toEqual({
      outputOffset: 0,
      outputFrames: 5,
    });
    expect(plan.input.packets.map((packet) => packet.index)).toEqual([3, 4]);
  });

  test("rejects nonzero benchmark starts without selected keyframe metadata", () => {
    expect(() =>
      planBenchmarkDecode(parseIvf(sampleIvf()), {
        outputOffset: 1,
        outputFrames: 1,
      }),
    ).toThrow("nonzero benchmark --frames start requires WebM keyframe metadata");

    expect(() =>
      planBenchmarkDecode(sampleDemuxedWebm([samplePacket(0, [0], { keyframe: false, visible: true })]), {
        outputOffset: 0,
        outputFrames: 1,
      }),
    ).not.toThrow();

    expect(() =>
      planBenchmarkDecode(
        sampleDemuxedWebm([
          samplePacket(0, [0], { keyframe: true, visible: true }),
          samplePacket(1, [1], { keyframe: false, visible: true }),
        ]),
        {
          outputOffset: 1,
          outputFrames: 1,
        },
      ),
    ).toThrow("benchmark start frame 1 maps to WebM packet 1, which is not marked as a keyframe");
  });

  test("benchmark window validation reports missing selected outputs", () => {
    const ivf = parseIvf(sampleIvf());
    const frames = [scriptedFrame(2, 2, 1, 2, 3, 1), scriptedFrame(2, 2, 4, 5, 6, 10)];
    const golden = parseGolden(
      [
        `${md5Hex(frames[0].compact)}  frame-0001.i420`,
        `${md5Hex(frames[1].compact)}  frame-0002.i420`,
        `${md5Hex(frames[1].compact)}  frame-0003.i420`,
      ].join("\n"),
    );

    const report = compareDecodedVp9WindowToGolden(
      "input.ivf",
      "input.ivf.md5",
      ivf,
      golden,
      scriptedDecoderWithTrailingEnd(frames),
      {
        outputOffset: 0,
        outputFrames: 3,
      },
    );

    expect(report.comparisons).toHaveLength(2);
    expect(missingCount(report)).toBe(1);
    expect(passes(report, false)).toBe(false);
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

    expect(() => compareDecodedVp9ToGolden("input.ivf", "input.ivf.md5", ivf, golden, decoder)).toThrow(
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

    expect(() => compareDecodedVp9ToGolden("input.ivf", "input.ivf.md5", ivf, golden, decoder)).toThrow(
      "decode packet 0 coded frame 0: bad coded frame",
    );
  });
});

function sampleReport(comparisons: ComparisonReport["comparisons"], expectedCount: number): ComparisonReport {
  return {
    inputPath: "input.ivf",
    goldenPath: "input.ivf.md5",
    container: "ivf",
    codec: "VP90",
    width: 320,
    height: 240,
    timebaseDenominator: 1000,
    timebaseNumerator: 1,
    declaredFrameCount: 2,
    packetCount: 2,
    codedFrames: 2,
    decodedOutputFrames: comparisons.length,
    skippedOutputFrames: 0,
    comparisons,
    expectedCount,
  };
}

function sampleDemuxedWebm(packets: DemuxedVp9["packets"]): DemuxedVp9 {
  return {
    container: "webm",
    codec: "V_VP9",
    width: 320,
    height: 240,
    timestampScale: 1_000_000,
    packets,
  };
}

function samplePacket(
  index: number,
  payload: number[],
  metadata: { keyframe: boolean; visible: boolean },
): DemuxedVp9["packets"][number] {
  return {
    index,
    timestamp: BigInt(index),
    payload: new Uint8Array(payload),
    keyframe: metadata.keyframe,
    visible: metadata.visible,
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

type ScriptedFrame = {
  native: NativeFrame;
  compact: Uint8Array;
  planes: Map<number, Uint8Array>;
};

function scriptedDecoder(frames: ScriptedFrame[]): FrameDecoder {
  return scriptedDecoderInternal(frames, false);
}

function scriptedDecoderWithTrailingEnd(frames: ScriptedFrame[]): FrameDecoder {
  return scriptedDecoderInternal(frames, true);
}

function scriptedDecoderInternal(frames: ScriptedFrame[], trailingEnd: boolean): FrameDecoder {
  const planeBytes = new Map<number, Uint8Array>();
  for (const frame of frames) {
    for (const [offset, bytes] of frame.planes) {
      planeBytes.set(offset, bytes);
    }
  }
  let index = 0;
  return {
    beginPacket() {},
    decodeNext(): DecodeStep {
      if (index >= frames.length) {
        if (trailingEnd) {
          return { kind: "no-output", packetDone: true };
        }
        throw new Error("scripted decoder exhausted");
      }
      const frame = frames[index].native;
      index += 1;
      return { kind: "output", packetDone: index % 2 === 0, frame };
    },
    planeBytes(plane: Plane) {
      const bytes = planeBytes.get(plane.offset);
      if (!bytes) {
        throw new Error(`unknown plane offset ${plane.offset}`);
      }
      return bytes.subarray(0, plane.byteLength);
    },
  };
}

function scriptedFrame(
  width: number,
  height: number,
  yValue: number,
  uValue: number,
  vValue: number,
  offsetBase: number,
): ScriptedFrame {
  const chromaWidth = Math.ceil(width / 2);
  const chromaHeight = Math.ceil(height / 2);
  const y = new Uint8Array(width * height).fill(yValue);
  const u = new Uint8Array(chromaWidth * chromaHeight).fill(uValue);
  const v = new Uint8Array(chromaWidth * chromaHeight).fill(vValue);
  const compact = new Uint8Array(y.byteLength + u.byteLength + v.byteLength);
  compact.set(y, 0);
  compact.set(u, y.byteLength);
  compact.set(v, y.byteLength + u.byteLength);

  return {
    native: {
      decodedWidth: width,
      decodedHeight: height,
      renderWidth: width,
      renderHeight: height,
      y: { offset: offsetBase, byteLength: y.byteLength, stride: width },
      u: { offset: offsetBase + 1, byteLength: u.byteLength, stride: chromaWidth },
      v: { offset: offsetBase + 2, byteLength: v.byteLength, stride: chromaWidth },
    },
    compact,
    planes: new Map([
      [offsetBase, y],
      [offsetBase + 1, u],
      [offsetBase + 2, v],
    ]),
  };
}

function sampleWebm(): Uint8Array {
  return webmConcat(
    webmElement(WEBM_ID.EBML, webmStringElement(WEBM_ID.DocType, "webm")),
    webmElement(
      WEBM_ID.Segment,
      webmConcat(
        webmElement(WEBM_ID.Info, webmUintElement(WEBM_ID.TimestampScale, 1_000_000)),
        webmElement(
          WEBM_ID.Tracks,
          webmElement(
            WEBM_ID.TrackEntry,
            webmConcat(
              webmUintElement(WEBM_ID.TrackNumber, 1),
              webmUintElement(WEBM_ID.TrackType, 1),
              webmStringElement(WEBM_ID.CodecID, "V_VP9"),
              webmUintElement(WEBM_ID.FlagLacing, 0),
              webmElement(
                WEBM_ID.Video,
                webmConcat(webmUintElement(WEBM_ID.PixelWidth, 160), webmUintElement(WEBM_ID.PixelHeight, 90)),
              ),
            ),
          ),
        ),
        webmElement(
          WEBM_ID.Cluster,
          webmConcat(
            webmUintElement(WEBM_ID.Timestamp, 0),
            webmElement(WEBM_ID.SimpleBlock, new Uint8Array([0x81, 0, 0, 0, 9, 8, 7])),
          ),
        ),
      ),
    ),
  );
}

function sampleIvf(options: { headerLength?: number; width?: number; height?: number } = {}): Uint8Array {
  const headerLength = options.headerLength ?? 32;
  const bytes: number[] = [];
  ascii(bytes, "DKIF");
  le16(bytes, 0);
  le16(bytes, headerLength);
  ascii(bytes, "VP90");
  le16(bytes, options.width ?? 320);
  le16(bytes, options.height ?? 240);
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

const WEBM_ID = {
  EBML: 0x1a45dfa3,
  DocType: 0x4282,
  Segment: 0x18538067,
  Info: 0x1549a966,
  TimestampScale: 0x2ad7b1,
  Tracks: 0x1654ae6b,
  TrackEntry: 0xae,
  TrackNumber: 0xd7,
  TrackType: 0x83,
  FlagLacing: 0x9c,
  CodecID: 0x86,
  Video: 0xe0,
  PixelWidth: 0xb0,
  PixelHeight: 0xba,
  Cluster: 0x1f43b675,
  Timestamp: 0xe7,
  SimpleBlock: 0xa3,
} as const;

function webmElement(id: number, content: Uint8Array): Uint8Array {
  return webmConcat(new Uint8Array(webmIdBytes(id)), new Uint8Array(webmSizeVint(content.byteLength)), content);
}

function webmUintElement(id: number, value: number): Uint8Array {
  return webmElement(id, new Uint8Array(webmUintBytes(value)));
}

function webmStringElement(id: number, value: string): Uint8Array {
  const bytes: number[] = [];
  for (let index = 0; index < value.length; index += 1) {
    bytes.push(value.charCodeAt(index));
  }
  return webmElement(id, new Uint8Array(bytes));
}

function webmIdBytes(id: number): number[] {
  const bytes: number[] = [];
  let started = false;
  for (let shift = 24; shift >= 0; shift -= 8) {
    const byte = (id >>> shift) & 0xff;
    if (byte !== 0 || started) {
      bytes.push(byte);
      started = true;
    }
  }
  return bytes;
}

function webmSizeVint(size: number): number[] {
  if (size <= 0x7e) {
    return [0x80 | size];
  }
  if (size <= 0x3ffe) {
    return [0x40 | (size >>> 8), size & 0xff];
  }
  throw new Error(`test fixture element is too large: ${size}`);
}

function webmUintBytes(value: number): number[] {
  if (value === 0) {
    return [0];
  }
  const bytes: number[] = [];
  let remaining = value;
  while (remaining > 0) {
    bytes.unshift(remaining & 0xff);
    remaining = Math.floor(remaining / 0x100);
  }
  return bytes;
}

function webmConcat(...parts: Uint8Array[]): Uint8Array {
  const length = parts.reduce((sum, part) => sum + part.byteLength, 0);
  const out = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.byteLength;
  }
  return out;
}

// Sequential pure-decode benchmark: vip9r wasm vs WebCodecs software vs
// WebCodecs hardware, one lane at a time on this worker thread. The vip9r
// headline number is the same measurand as the d8 perf oracle (summed
// decodeNext time); VideoFrame construction cost is measured separately.
import { Vp9Decoder } from "../wasm";
import type { NativeFrame } from "../wasm";
import { parseVp9Input } from "../wasm-driver/golden";
import type { DemuxedVp9, Vp9Packet } from "../wasm-driver/golden";
import { packetTimestampUs } from "./media-time";
import { instantiateVip9r } from "./vip9r-instance";

export type BenchLane = "vip9r" | "wc-sw" | "wc-hw";

export type BenchInit = {
  media: ArrayBuffer;
  // Packet-count prefix to bench; 0 benches the whole clip.
  packetLimit: number;
};

const LANES: BenchLane[] = ["vip9r", "wc-sw", "wc-hw"];

export type LaneResult = {
  lane: BenchLane;
  frames: number;
  wallMs: number;
  decodeMs?: number;
  videoFrameMs?: number;
};

export type BenchEvent =
  | { type: "meta"; container: string; width: number; height: number; packets: number; budgetMs: number }
  | { type: "log"; message: string; error: boolean }
  | { type: "lane"; result: LaneResult }
  | { type: "lane-skipped"; lane: BenchLane; reason: string }
  | { type: "done" }
  | { type: "error"; message: string };

const WARMUP_PACKETS = 60;

function post(event: BenchEvent): void {
  self.postMessage(event);
}

self.onmessage = (event: MessageEvent<BenchInit>) => {
  run(event.data).catch((error: unknown) => {
    post({ type: "error", message: String(error) });
  });
};

async function run(init: BenchInit): Promise<void> {
  const input = parseVp9Input(new Uint8Array(init.media));
  const packets =
    init.packetLimit > 0 ? input.packets.slice(0, init.packetLimit) : input.packets;
  const first = packetTimestampUs(input, packets[0].timestamp);
  const last = packetTimestampUs(input, packets[packets.length - 1].timestamp);
  const budgetMs = packets.length < 2 ? 0 : (last - first) / 1000 / (packets.length - 1);
  post({
    type: "meta",
    container: input.container,
    width: input.width,
    height: input.height,
    packets: packets.length,
    budgetMs,
  });

  for (const lane of LANES) {
    // Let the previous lane's frames and decoder teardown settle.
    await new Promise((resolve) => setTimeout(resolve, 50));
    if (lane === "vip9r") {
      post({ type: "lane", result: await vip9rLane(input, packets) });
    } else {
      const mode = lane === "wc-sw" ? "prefer-software" : "prefer-hardware";
      const result = await webCodecsLane(lane, mode, input, packets);
      if (typeof result === "string") {
        post({ type: "lane-skipped", lane, reason: result });
      } else {
        post({ type: "lane", result });
      }
    }
  }
  post({ type: "done" });
}

type Vip9rPassStats = {
  frames: number;
  decodeMs: number;
  videoFrameMs: number;
};

async function vip9rLane(input: DemuxedVp9, packets: Vp9Packet[]): Promise<LaneResult> {
  const { instance, memory } = await instantiateVip9r(input, (message) =>
    post({ type: "log", message, error: true }),
  );
  const decoder = new Vp9Decoder(instance, input.width, input.height);
  post({ type: "log", message: "vip9r: warmup", error: false });
  vip9rPass(decoder, memory, input, packets.slice(0, WARMUP_PACKETS));
  post({ type: "log", message: `vip9r: timing ${packets.length} packets`, error: false });
  const before = performance.now();
  const stats = vip9rPass(decoder, memory, input, packets);
  const wallMs = performance.now() - before;
  return { lane: "vip9r", wallMs, ...stats };
}

function vip9rPass(
  decoder: Vp9Decoder,
  memory: WebAssembly.Memory,
  input: DemuxedVp9,
  packets: Vp9Packet[],
): Vip9rPassStats {
  const stats: Vip9rPassStats = { frames: 0, decodeMs: 0, videoFrameMs: 0 };
  for (const packet of packets) {
    decoder.beginPacket(packet.payload);
    let packetDone = false;
    while (!packetDone) {
      const before = performance.now();
      const step = decoder.decodeNext();
      stats.decodeMs += performance.now() - before;
      packetDone = step.packetDone;
      if (step.kind !== "output") {
        continue;
      }
      const vfBefore = performance.now();
      makeVideoFrame(memory, step.frame, packetTimestampUs(input, packet.timestamp)).close();
      stats.videoFrameMs += performance.now() - vfBefore;
      stats.frames += 1;
    }
  }
  return stats;
}

function makeVideoFrame(
  memory: WebAssembly.Memory,
  native: NativeFrame,
  timestamp: number,
): VideoFrame {
  return new VideoFrame(memory.buffer, {
    format: "I420",
    codedWidth: native.decodedWidth,
    codedHeight: native.decodedHeight,
    visibleRect: { x: 0, y: 0, width: native.renderWidth, height: native.renderHeight },
    layout: [
      { offset: native.y.offset, stride: native.y.stride },
      { offset: native.u.offset, stride: native.u.stride },
      { offset: native.v.offset, stride: native.v.stride },
    ],
    timestamp,
  });
}

function vp9CodecString(input: DemuxedVp9): string {
  // Profile 0, 8-bit; level 3.1 covers up to 720p30, 5.1 covers the rest of
  // what this demo will see.
  const level = input.width * input.height <= 1280 * 720 ? "31" : "51";
  return `vp09.00.${level}.08`;
}

async function webCodecsLane(
  lane: BenchLane,
  mode: HardwareAcceleration,
  input: DemuxedVp9,
  packets: Vp9Packet[],
): Promise<LaneResult | string> {
  if (typeof VideoDecoder === "undefined") {
    return "WebCodecs unavailable";
  }
  const config: VideoDecoderConfig = {
    codec: vp9CodecString(input),
    codedWidth: input.width,
    codedHeight: input.height,
    hardwareAcceleration: mode,
  };
  const support = await VideoDecoder.isConfigSupported(config);
  if (!support.supported) {
    return `${config.codec} with ${mode} not supported`;
  }

  const chunks = packets.map(
    (packet, index) =>
      new EncodedVideoChunk({
        type: (packet.keyframe ?? index === 0) ? "key" : "delta",
        timestamp: packetTimestampUs(input, packet.timestamp),
        data: packet.payload,
      }),
  );

  let frames = 0;
  let failure: unknown;
  const decoder = new VideoDecoder({
    output: (frame) => {
      frames += 1;
      frame.close();
    },
    error: (error) => {
      failure = error;
    },
  });
  try {
    decoder.configure(config);
    post({ type: "log", message: `${lane}: warmup`, error: false });
    for (const chunk of chunks.slice(0, WARMUP_PACKETS)) {
      decoder.decode(chunk);
    }
    await decoder.flush();

    post({ type: "log", message: `${lane}: timing ${chunks.length} packets`, error: false });
    frames = 0;
    const before = performance.now();
    for (const chunk of chunks) {
      decoder.decode(chunk);
    }
    await decoder.flush();
    const wallMs = performance.now() - before;
    return { lane, frames, wallMs };
  } catch (error) {
    throw failure ?? error;
  } finally {
    if (decoder.state !== "closed") {
      decoder.close();
    }
  }
}

// Sequential pure-decode benchmark: vip9r wasm vs ogv.js vs WebCodecs software
// vs WebCodecs hardware, one lane at a time on this worker thread. The vip9r
// headline number is the same measurand as the d8 perf oracle (summed
// decodeNext time); VideoFrame construction cost is measured separately.
import { Vp9Decoder } from "../wasm";
import type { NativeFrame } from "../wasm";
import { openMediaStream } from "./media-stream";
import type { MediaHeader, MediaPacket } from "./media-stream";
import { packetTimestampUs } from "./media-time";
import { activateWorkerPool } from "./pool";
import { instantiateVip9r } from "./vip9r-instance";

export type BenchLane = "vip9r" | "ogv" | "wc-sw" | "wc-hw";

export type BenchInit = {
  url: string;
  // Packet-count prefix to bench; 0 benches the whole clip.
  packetLimit: number;
};

const LANES: BenchLane[] = ["vip9r", "ogv", "wc-sw", "wc-hw"];

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
  const media = await openMediaStream(init.url);
  const header = media.header;
  // The lanes replay the packet list twice (warmup + timed), so bench
  // retains it; a packet limit stops the download early.
  const packets: MediaPacket[] = [];
  for await (const packet of media.packets) {
    packets.push(packet);
    if (init.packetLimit > 0 && packets.length >= init.packetLimit) {
      break;
    }
  }
  const first = packetTimestampUs(header, packets[0].timestamp);
  const last = packetTimestampUs(header, packets[packets.length - 1].timestamp);
  const budgetMs = packets.length < 2 ? 0 : (last - first) / 1000 / (packets.length - 1);
  post({
    type: "meta",
    container: header.container,
    width: header.width,
    height: header.height,
    packets: packets.length,
    budgetMs,
  });

  for (const lane of LANES) {
    // Let the previous lane's frames and decoder teardown settle.
    await new Promise((resolve) => setTimeout(resolve, 50));
    if (lane === "vip9r") {
      post({ type: "lane", result: await vip9rLane(header, packets) });
    } else if (lane === "ogv") {
      const result = await ogvLane(header, packets);
      if (typeof result === "string") {
        post({ type: "lane-skipped", lane, reason: result });
      } else {
        post({ type: "lane", result });
      }
    } else {
      const mode = lane === "wc-sw" ? "prefer-software" : "prefer-hardware";
      const result = await webCodecsLane(lane, mode, header, packets);
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

async function vip9rLane(input: MediaHeader, packets: MediaPacket[]): Promise<LaneResult> {
  const { instance, module, memory } = await instantiateVip9r(input, (message) =>
    post({ type: "log", message, error: true }),
  );
  // Tile-parallel like playback, so the lane measures what the player runs.
  const pool = await activateWorkerPool(instance, module, memory, (message) =>
    post({ type: "log", message, error: true }),
  );
  const decoder = new Vp9Decoder(instance, input.width, input.height);
  post({ type: "log", message: "vip9r: warmup", error: false });
  vip9rPass(decoder, memory, input, packets.slice(0, WARMUP_PACKETS));
  post({ type: "log", message: `vip9r: timing ${packets.length} packets`, error: false });
  const before = performance.now();
  const stats = vip9rPass(decoder, memory, input, packets);
  const wallMs = performance.now() - before;
  // Idle pool workers cost nothing, but the WebCodecs lanes should not share
  // the process with three parked threads holding the wasm memory alive.
  pool.terminate();
  return { lane: "vip9r", wallMs, ...stats };
}

function ogvVideoFormat(input: MediaHeader): OgvVideoFormat {
  return {
    width: input.width,
    height: input.height,
    chromaWidth: (input.width + 1) >> 1,
    chromaHeight: (input.height + 1) >> 1,
    cropLeft: 0,
    cropTop: 0,
    cropWidth: input.width,
    cropHeight: input.height,
    displayWidth: input.width,
    displayHeight: input.height,
    fps: 0,
  };
}

async function ogvLane(input: MediaHeader, packets: MediaPacket[]): Promise<LaneResult | string> {
  // The ogv.js wrapper is a classic-worker script that uses importScripts().
  const worker = new Worker(new URL("./ogv-lane-worker.ts", import.meta.url));
  try {
    const init: OgvLaneInit = {
      moduleScript: "/ogv/ogv-decoder-video-vp9-mt.js",
      videoFormat: ogvVideoFormat(input),
      warmupPackets: WARMUP_PACKETS,
      packets: packets.map((packet) => ({
        data: packet.payload,
        timestampUs: packetTimestampUs(input, packet.timestamp),
      })),
    };
    return await new Promise<LaneResult | string>((resolve, reject) => {
      worker.onmessage = (event: MessageEvent<OgvLaneEvent>) => {
        const message = event.data;
        switch (message.type) {
          case "log":
            post({ type: "log", message: message.message, error: message.error });
            break;
          case "done":
            resolve({ lane: "ogv", frames: message.frames, wallMs: message.wallMs });
            break;
          case "skipped":
            resolve(message.reason);
            break;
          case "error":
            reject(new Error(message.message));
            break;
        }
      };
      worker.onerror = (event) => {
        reject(new Error(`ogv worker: ${event.message}`));
      };
      worker.postMessage(init);
    });
  } finally {
    worker.terminate();
  }
}

function vip9rPass(
  decoder: Vp9Decoder,
  memory: WebAssembly.Memory,
  input: MediaHeader,
  packets: MediaPacket[],
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

function vp9CodecString(input: MediaHeader): string {
  // Profile 0, 8-bit; level 3.1 covers up to 720p30, 5.1 covers the rest of
  // what this demo will see.
  const level = input.width * input.height <= 1280 * 720 ? "31" : "51";
  return `vp09.00.${level}.08`;
}

async function webCodecsLane(
  lane: "wc-sw" | "wc-hw",
  mode: HardwareAcceleration,
  input: MediaHeader,
  packets: MediaPacket[],
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

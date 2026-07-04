import { Vp9Decoder } from "../wasm";
import type { NativeFrame } from "../wasm";
import { parseVp9Input } from "../wasm-driver/golden";
import { packetTimestampUs } from "./media-time";
import { spawnWorkerPool } from "./pool";
import { instantiateVip9r } from "./vip9r-instance";

export type WorkerInit = {
  media: ArrayBuffer;
  queueDepth: number;
};

export type WorkerAck = {
  type: "ack";
  count: number;
};

export type WorkerEvent =
  | { type: "meta"; container: string; width: number; height: number; packets: number }
  | { type: "frame"; frame: VideoFrame; decodeMs: number }
  | { type: "log"; message: string; error: boolean }
  | { type: "done"; frames: number; totalDecodeMs: number }
  | { type: "error"; message: string };

function post(event: WorkerEvent, transfer: Transferable[] = []): void {
  self.postMessage(event, { transfer });
}

// Frame credits: main returns one credit per consumed (presented or dropped)
// frame, bounding decode-ahead memory.
let credits = 0;
let wake: (() => void) | undefined;

async function takeCredit(): Promise<void> {
  while (credits === 0) {
    await new Promise<void>((resolve) => {
      wake = resolve;
    });
  }
  credits -= 1;
}

let started = false;

self.onmessage = (event: MessageEvent<WorkerInit | WorkerAck>) => {
  const data = event.data;
  if ("type" in data && data.type === "ack") {
    credits += data.count;
    wake?.();
    wake = undefined;
    return;
  }
  if (started) {
    post({ type: "error", message: "worker already started" });
    return;
  }
  started = true;
  const init = data as WorkerInit;
  credits = init.queueDepth;
  decodeAll(init.media).catch((error: unknown) => {
    post({ type: "error", message: String(error) });
  });
};

function makeVideoFrame(memory: WebAssembly.Memory, native: NativeFrame, timestamp: number): VideoFrame {
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

async function decodeAll(media: ArrayBuffer): Promise<void> {
  const input = parseVp9Input(new Uint8Array(media));
  post({
    type: "meta",
    container: input.container,
    width: input.width,
    height: input.height,
    packets: input.packets.length,
  });

  const { instance, module, memory } = await instantiateVip9r(input, (message) =>
    post({ type: "log", message, error: true }),
  );
  // Tile workers park in the shared memory's pool; decode stays serial until
  // tile-parallel dispatch lands, but spawning here exercises the nested-
  // worker spawn and shadow-stack rebind path on every playback. The pool
  // dies with this worker.
  spawnWorkerPool(module, memory, (message) => post({ type: "log", message, error: true }));
  const activatePool = instance.exports.vip9r_pool_activate;
  if (typeof activatePool !== "function") {
    throw new Error("missing wasm export: vip9r_pool_activate");
  }
  activatePool();
  const decoder = new Vp9Decoder(instance, input.width, input.height);

  let frames = 0;
  let totalDecodeMs = 0;
  let pendingMs = 0;
  for (const packet of input.packets) {
    const timestamp = packetTimestampUs(input, packet.timestamp);
    decoder.beginPacket(packet.payload);
    let packetDone = false;
    while (!packetDone) {
      const before = performance.now();
      const step = decoder.decodeNext();
      const elapsed = performance.now() - before;
      totalDecodeMs += elapsed;
      pendingMs += elapsed;
      packetDone = step.packetDone;
      if (step.kind !== "output") {
        continue;
      }
      await takeCredit();
      const frame = makeVideoFrame(memory, step.frame, timestamp);
      post({ type: "frame", frame, decodeMs: pendingMs }, [frame]);
      frames += 1;
      pendingMs = 0;
    }
  }
  post({ type: "done", frames, totalDecodeMs });
}

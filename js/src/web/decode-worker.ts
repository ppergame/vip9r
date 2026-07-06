import { Vp9Decoder } from "../wasm";
import type { NativeFrame } from "../wasm";
import { openMediaStream } from "./media-stream";
import type { MediaPacket } from "./media-stream";
import { packetTimestampUs } from "./media-time";
import { activateWorkerPool } from "./pool";
import { instantiateVip9r } from "./vip9r-instance";

export type WorkerInit = {
  url: string;
  queueDepth: number;
  // Stop after this many output frames; 0 decodes the whole clip.
  frameLimit: number;
  // Perf-attribution probes (&probe=a,b): "prebuffer" downloads and demuxes
  // all packets before decoding (bench-style), "discard" decodes without
  // constructing or posting frames. Main-side flags ride along untouched.
  probe: string[];
};

export type WorkerAck = {
  type: "ack";
  count: number;
};

export type WorkerEvent =
  | { type: "meta"; container: string; width: number; height: number }
  | {
      type: "frame";
      frame: VideoFrame;
      decodeMs: number;
      packetBytes: number;
      keyframe: boolean;
    }
  | { type: "log"; message: string; error: boolean }
  | { type: "done"; frames: number; packets: number; totalDecodeMs: number }
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
  decodeAll(init).catch((error: unknown) => {
    post({ type: "error", message: String(error) });
  });
};

function makeVideoFrame(
  memory: WebAssembly.Memory,
  native: NativeFrame,
  timestamp: number,
): VideoFrame {
  return new VideoFrame(memory.buffer, {
    format: "I420",
    codedWidth: native.decodedWidth,
    codedHeight: native.decodedHeight,
    visibleRect: {
      x: 0,
      y: 0,
      width: native.renderWidth,
      height: native.renderHeight,
    },
    layout: [
      { offset: native.y.offset, stride: native.y.stride },
      { offset: native.u.offset, stride: native.u.stride },
      { offset: native.v.offset, stride: native.v.stride },
    ],
    timestamp,
  });
}

async function decodeAll(init: WorkerInit): Promise<void> {
  const media = await openMediaStream(init.url);
  const header = media.header;
  post({
    type: "meta",
    container: header.container,
    width: header.width,
    height: header.height,
  });

  const { instance, module, memory } = await instantiateVip9r(
    header,
    (message) => post({ type: "log", message, error: true }),
  );
  // The pool dies with this worker.
  await activateWorkerPool(instance, module, memory, (message) =>
    post({ type: "log", message, error: true }),
  );
  const decoder = new Vp9Decoder(instance, header.width, header.height);

  const discard = init.probe.includes("discard");
  // localvf: construct and immediately close each VideoFrame on this thread
  // without posting — isolates the plane-copy + frame GC from the transfer.
  const localvf = init.probe.includes("localvf");
  let source: AsyncIterable<MediaPacket> | MediaPacket[] = media.packets;
  if (init.probe.includes("prebuffer")) {
    const buffered: MediaPacket[] = [];
    for await (const packet of media.packets) {
      buffered.push(packet);
      if (init.frameLimit > 0 && buffered.length >= init.frameLimit) {
        break;
      }
    }
    post({
      type: "log",
      message: `probe: prebuffered ${buffered.length} packets`,
      error: false,
    });
    source = buffered;
  }

  let frames = 0;
  let packets = 0;
  let totalDecodeMs = 0;
  let pendingMs = 0;
  decode: for await (const packet of source) {
    const timestamp = packetTimestampUs(header, packet.timestamp);
    const keyframe = packet.keyframe ?? packets === 0;
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
      if (localvf) {
        makeVideoFrame(memory, step.frame, timestamp).close();
      } else if (!discard) {
        await takeCredit();
        const frame = makeVideoFrame(memory, step.frame, timestamp);
        post(
          {
            type: "frame",
            frame,
            decodeMs: pendingMs,
            packetBytes: packet.payload.byteLength,
            keyframe,
          },
          [frame],
        );
      }
      frames += 1;
      pendingMs = 0;
      if (frames === init.frameLimit) {
        packets += 1;
        break decode;
      }
    }
    packets += 1;
  }
  post({ type: "done", frames, packets, totalDecodeMs });
}

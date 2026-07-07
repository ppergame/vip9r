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

// iOS Safari's VideoFrame constructor ignores the layout option entirely:
// planes are read tightly packed from offset 0 of the buffer regardless of
// the given offsets and strides (verified on iOS 18.7 / Safari 26.5 — the
// symptom is a green frame, YUV zeros). Probe with an offset layout and see
// whether the marker bytes made it into the frame.
async function detectLayoutIgnored(): Promise<boolean> {
  const buffer = new Uint8Array(64 + 24);
  buffer.fill(7, 64);
  const frame = new VideoFrame(buffer, {
    format: "I420",
    codedWidth: 4,
    codedHeight: 4,
    layout: [
      { offset: 64, stride: 4 },
      { offset: 80, stride: 2 },
      { offset: 84, stride: 2 },
    ],
    timestamp: 0,
  });
  try {
    const out = new Uint8Array(24);
    await frame.copyTo(out);
    return out[0] !== 7;
  } catch {
    return true;
  } finally {
    frame.close();
  }
}

// Fallback for the layout bug: copy the planes into a tight I420 buffer and
// construct without layout. Costs an extra ~1.4 MB copy per 720p frame, on
// affected browsers only.
function makeRepackedVideoFrame(
  memory: WebAssembly.Memory,
  native: NativeFrame,
  timestamp: number,
): VideoFrame {
  const width = native.renderWidth;
  const height = native.renderHeight;
  const chromaWidth = (width + 1) >> 1;
  const chromaHeight = (height + 1) >> 1;
  const tight = new Uint8Array(width * height + 2 * chromaWidth * chromaHeight);
  const source = new Uint8Array(memory.buffer);
  let cursor = 0;
  const copyPlane = (plane: NativeFrame["y"], w: number, h: number) => {
    for (let row = 0; row < h; row += 1) {
      const start = plane.offset + row * plane.stride;
      tight.set(source.subarray(start, start + w), cursor);
      cursor += w;
    }
  };
  copyPlane(native.y, width, height);
  copyPlane(native.u, chromaWidth, chromaHeight);
  copyPlane(native.v, chromaWidth, chromaHeight);
  return new VideoFrame(tight, {
    format: "I420",
    codedWidth: width,
    codedHeight: height,
    timestamp,
  });
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

  const { instance, module, memory } = await instantiateVip9r((message) =>
    post({ type: "log", message, error: true }),
  );
  // The pool dies with this worker.
  await activateWorkerPool(instance, module, memory, (message) =>
    post({ type: "log", message, error: true }),
  );
  const decoder = new Vp9Decoder(instance, header.width, header.height);

  const layoutIgnored = await detectLayoutIgnored();
  if (layoutIgnored) {
    post({
      type: "log",
      message: "VideoFrame ignores plane layout here; repacking (Safari?)",
      error: false,
    });
  }
  const buildFrame = layoutIgnored ? makeRepackedVideoFrame : makeVideoFrame;

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
        buildFrame(memory, step.frame, timestamp).close();
      } else if (!discard) {
        await takeCredit();
        const frame = buildFrame(memory, step.frame, timestamp);
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

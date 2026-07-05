// This must stay a classic-worker script (the ogv wrapper needs
// importScripts), so no module syntax at all — even a type-only import makes
// Vite's dev transform append `export {}`. The wire protocol types are
// ambient, in ogv-lane-protocol.d.ts.
(() => {
  type OgvDecodeResult = {
    ok: boolean;
    frames: VideoFrame[];
  };

  type OgvDecoder = {
    decode(packet: OgvPacket): Promise<OgvDecodeResult>;
    flush(): Promise<OgvDecodeResult>;
    close(): void;
  };

  type OgvDecoderFactory = {
    create(options: {
      moduleScript: string;
      videoFormat: OgvVideoFormat;
    }): Promise<OgvDecoder>;
  };

  type OgvWorkerGlobal = {
    importScripts(...urls: string[]): void;
    OGVVP9VideoFrameDecoder?: OgvDecoderFactory;
    onmessage: ((event: MessageEvent<OgvLaneInit>) => void) | null;
    postMessage(message: OgvLaneEvent): void;
  };

  const worker = globalThis as unknown as OgvWorkerGlobal;

  function post(event: OgvLaneEvent): void {
    worker.postMessage(event);
  }

  worker.importScripts("/ogv/ogv-vp9-video-frame-decoder.js");

  worker.onmessage = (event: MessageEvent<OgvLaneInit>) => {
    run(event.data).catch((error: unknown) => {
      post({ type: "error", message: String(error) });
    });
  };

  // Emscripten pthread startup posts the shared wasm memory to its worker
  // pool; probe the serializer gate directly, same as pool.ts (the
  // crossOriginIsolated bool can disagree with what postMessage allows).
  function sharedMemoryTransferable(): boolean {
    let memory: WebAssembly.Memory;
    try {
      memory = new WebAssembly.Memory({ initial: 1, maximum: 1, shared: true });
    } catch {
      return false;
    }
    const channel = new MessageChannel();
    try {
      channel.port1.postMessage(memory);
      return true;
    } catch {
      return false;
    } finally {
      channel.port1.close();
      channel.port2.close();
    }
  }

  async function run(init: OgvLaneInit): Promise<void> {
    if (!sharedMemoryTransferable()) {
      post({ type: "skipped", reason: "shared memory transfer blocked" });
      return;
    }
    if (typeof VideoFrame === "undefined") {
      post({ type: "skipped", reason: "VideoFrame unavailable" });
      return;
    }
    const factory = worker.OGVVP9VideoFrameDecoder;
    if (factory === undefined) {
      throw new Error("OGVVP9VideoFrameDecoder unavailable");
    }

    const decoder = await factory.create({
      moduleScript: init.moduleScript,
      videoFormat: init.videoFormat,
    });
    try {
      post({ type: "log", message: "ogv: warmup", error: false });
      await decodePass(decoder, init.packets.slice(0, init.warmupPackets));
      post({ type: "log", message: `ogv: timing ${init.packets.length} packets`, error: false });
      const before = performance.now();
      const frames = await decodePass(decoder, init.packets);
      const wallMs = performance.now() - before;
      post({ type: "done", frames, wallMs });
    } finally {
      decoder.close();
    }
  }

  // Keep one packet in flight ahead of the awaited one so the ogv decode
  // pthread never idles while this thread builds VideoFrames — the
  // decode-ahead ogv.js's own player gets.
  async function decodePass(decoder: OgvDecoder, packets: OgvPacket[]): Promise<number> {
    let frames = 0;
    let pending: Promise<OgvDecodeResult> | null = null;
    for (const packet of packets) {
      const next = decoder.decode(packet);
      if (pending !== null) {
        frames += closeFrames((await pending).frames);
      }
      pending = next;
    }
    if (pending !== null) {
      frames += closeFrames((await pending).frames);
    }
    frames += closeFrames((await decoder.flush()).frames);
    return frames;
  }

  function closeFrames(frames: VideoFrame[]): number {
    for (const frame of frames) {
      frame.close();
    }
    return frames.length;
  }
})();

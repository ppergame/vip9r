import type { WorkerAck, WorkerEvent, WorkerInit } from "./decode-worker";

export type PlaybackHandle = {
  stop(): void;
};

export type PlaybackOptions = {
  media: ArrayBuffer;
  canvas: HTMLCanvasElement;
  log: (message: string, kind?: "info" | "error") => void;
  stats: (text: string) => void;
  onFinished?: () => void;
};

// Frames decoded ahead of the presentation clock. Bounds worker decode-ahead
// memory: each queued 720p VideoFrame holds ~1.4 MB.
const QUEUE_DEPTH = 8;

type QueuedFrame = {
  frame: VideoFrame;
  decodeMs: number;
};

export function startPlayback(options: PlaybackOptions): PlaybackHandle {
  const worker = new Worker(new URL("./decode-worker.ts", import.meta.url), {
    type: "module",
  });
  const context = options.canvas.getContext("2d");
  if (context === null) {
    throw new Error("canvas 2d context unavailable");
  }

  const queue: QueuedFrame[] = [];
  let epoch: number | undefined;
  let rafId: number | undefined;
  let stopped = false;
  let presented = 0;
  let dropped = 0;
  let presentedDecodeMs = 0;
  let done: { frames: number; totalDecodeMs: number } | undefined;

  const init: WorkerInit = { media: options.media, queueDepth: QUEUE_DEPTH };
  worker.postMessage(init, { transfer: [options.media] });

  worker.onmessage = (event: MessageEvent<WorkerEvent>) => {
    const message = event.data;
    switch (message.type) {
      case "meta":
        options.log(
          `demuxed: ${message.container} ${message.width}×${message.height}, ${message.packets} packets`,
        );
        break;
      case "frame":
        if (stopped) {
          message.frame.close();
          break;
        }
        queue.push({ frame: message.frame, decodeMs: message.decodeMs });
        if (rafId === undefined) {
          rafId = requestAnimationFrame(tick);
        }
        break;
      case "log":
        options.log(message.message, message.error ? "error" : "info");
        break;
      case "done":
        done = message;
        break;
      case "error":
        options.log(`decode: ${message.message}`, "error");
        finish();
        break;
    }
  };
  worker.onerror = (event) => {
    options.log(`worker: ${event.message}`, "error");
    finish();
  };

  function tick(now: number): void {
    rafId = undefined;
    if (stopped) {
      return;
    }

    if (queue.length > 0) {
      if (epoch === undefined) {
        epoch = now - queue[0].frame.timestamp / 1000;
      }
      const mediaNowMs = now - epoch;
      let take = -1;
      for (let i = 0; i < queue.length; i += 1) {
        if (queue[i].frame.timestamp / 1000 <= mediaNowMs) {
          take = i;
        }
      }
      if (take >= 0) {
        for (let i = 0; i < take; i += 1) {
          queue[i].frame.close();
          dropped += 1;
        }
        const entry = queue[take];
        presentFrame(entry.frame);
        presented += 1;
        presentedDecodeMs += entry.decodeMs;
        queue.splice(0, take + 1);
        const ack: WorkerAck = { type: "ack", count: take + 1 };
        worker.postMessage(ack);
        updateStats(mediaNowMs);
      }
    } else if (done !== undefined) {
      const avg = done.frames === 0 ? 0 : done.totalDecodeMs / done.frames;
      options.log(
        `done: ${done.frames} frames decoded, avg ${avg.toFixed(2)} ms/frame; ` +
          `${presented} presented, ${dropped} dropped`,
      );
      finish();
      options.onFinished?.();
      return;
    }
    rafId = requestAnimationFrame(tick);
  }

  function presentFrame(frame: VideoFrame): void {
    const canvas = options.canvas;
    if (canvas.width !== frame.displayWidth || canvas.height !== frame.displayHeight) {
      canvas.width = frame.displayWidth;
      canvas.height = frame.displayHeight;
    }
    context!.drawImage(frame, 0, 0);
    frame.close();
  }

  function updateStats(mediaNowMs: number): void {
    const avg = presented === 0 ? 0 : presentedDecodeMs / presented;
    const fps = mediaNowMs <= 0 ? 0 : (presented * 1000) / mediaNowMs;
    options.stats(
      `decode ${avg.toFixed(1)} ms/frame · shown ${presented} @ ${fps.toFixed(1)} fps · ` +
        `dropped ${dropped} · t=${(mediaNowMs / 1000).toFixed(1)}s`,
    );
  }

  function finish(): void {
    if (stopped) {
      return;
    }
    stopped = true;
    if (rafId !== undefined) {
      cancelAnimationFrame(rafId);
      rafId = undefined;
    }
    for (const entry of queue) {
      entry.frame.close();
    }
    queue.length = 0;
    worker.terminate();
  }

  return { stop: finish };
}

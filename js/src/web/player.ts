import type { WorkerAck, WorkerEvent, WorkerInit } from "./decode-worker";

export type PlaybackHandle = {
  stop(): void;
};

export type PlaybackOptions = {
  url: string;
  // Stop after this many decoded frames; 0 plays the whole clip.
  frameLimit: number;
  canvas: HTMLCanvasElement;
  log: (message: string, kind?: "info" | "error") => void;
  stats: (text: string) => void;
  // Decode-clock series: fires per decoded frame on arrival, before pacing,
  // with the running frame-budget estimate from media timestamps.
  onFrameDecoded?: (decodeMs: number, budgetMs: number) => void;
  onFinished?: () => void;
};

// Frames decoded ahead of the presentation clock. Bounds worker decode-ahead
// memory: each queued 720p VideoFrame holds ~1.4 MB. Presentation pre-rolls
// until the queue is full (or the stream ends), so the buffer absorbs
// keyframe spikes from t=0 even when decode has no surplus over realtime.
// Priming can complete because the worker's initial credits equal this depth.
const QUEUE_DEPTH = 32;

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
  let slipMs = 0;
  let wallStart: number | undefined;
  let presentedDecodeMs = 0;
  let preroll = true;
  let done:
    { frames: number; packets: number; totalDecodeMs: number } | undefined;
  let decoded = 0;
  let firstTimestampUs = 0;
  let lastTimestampUs = 0;

  const init: WorkerInit = {
    url: options.url,
    queueDepth: QUEUE_DEPTH,
    frameLimit: options.frameLimit,
  };
  worker.postMessage(init);

  worker.onmessage = (event: MessageEvent<WorkerEvent>) => {
    const message = event.data;
    switch (message.type) {
      case "meta":
        options.log(
          `demuxed: ${message.container} ${message.width}×${message.height}`,
        );
        break;
      case "frame": {
        if (stopped) {
          message.frame.close();
          break;
        }
        // Pacing policy: media clock caps the rate, decode floors it, no
        // frame is ever dropped. A frame arriving after its due time slips
        // the epoch by its lateness, so playback continues from there
        // instead of dropping to catch up. Only arrival lateness (decode)
        // slips; presentation lateness is rAF-grid noise and slipping on it
        // would compound into a permanent rate loss.
        const arrival = performance.now();
        const ptsMs = message.frame.timestamp / 1000;
        if (epoch !== undefined && arrival - epoch > ptsMs) {
          slipMs += arrival - epoch - ptsMs;
          epoch = arrival - ptsMs;
        }
        queue.push({ frame: message.frame, decodeMs: message.decodeMs });
        if (preroll && queue.length >= QUEUE_DEPTH) {
          preroll = false;
        }
        if (decoded === 0) {
          firstTimestampUs = message.frame.timestamp;
        }
        lastTimestampUs = message.frame.timestamp;
        decoded += 1;
        options.onFrameDecoded?.(
          message.decodeMs,
          decoded < 2
            ? 0
            : (lastTimestampUs - firstTimestampUs) / 1000 / (decoded - 1),
        );
        if (rafId === undefined) {
          rafId = requestAnimationFrame(tick);
        }
        break;
      }
      case "log":
        options.log(message.message, message.error ? "error" : "info");
        break;
      case "done":
        done = message;
        // A clip shorter than the queue can never prime it.
        preroll = false;
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

    if (!preroll && queue.length > 0) {
      if (epoch === undefined) {
        // Pre-roll complete: anchor the clock so queue[0] is due now and the
        // frames behind it are lead.
        epoch = now - queue[0].frame.timestamp / 1000;
      }
      const mediaNowMs = now - epoch;
      if (queue[0].frame.timestamp / 1000 <= mediaNowMs) {
        const entry = queue.shift()!;
        presentFrame(entry.frame);
        if (wallStart === undefined) {
          wallStart = now;
        }
        presented += 1;
        presentedDecodeMs += entry.decodeMs;
        const ack: WorkerAck = { type: "ack", count: 1 };
        worker.postMessage(ack);
        updateStats(now, mediaNowMs);
      }
    } else if (done !== undefined) {
      const avg = done.frames === 0 ? 0 : done.totalDecodeMs / done.frames;
      options.log(
        `done: ${done.frames} frames decoded from ${done.packets} packets, ` +
          `avg ${avg.toFixed(2)} ms/frame; ${presented} presented, ` +
          `slip ${(slipMs / 1000).toFixed(1)}s`,
      );
      finish();
      options.onFinished?.();
      return;
    }
    rafId = requestAnimationFrame(tick);
  }

  function presentFrame(frame: VideoFrame): void {
    const canvas = options.canvas;
    if (
      canvas.width !== frame.displayWidth ||
      canvas.height !== frame.displayHeight
    ) {
      canvas.width = frame.displayWidth;
      canvas.height = frame.displayHeight;
    }
    context!.drawImage(frame, 0, 0);
    frame.close();
  }

  // fps is wall-clock: under the slip policy the media clock tracks decode,
  // so media-relative fps would read nominal even on a struggling device.
  // Cumulative slip is the health signal.
  function updateStats(now: number, mediaNowMs: number): void {
    const avg = presented === 0 ? 0 : presentedDecodeMs / presented;
    const wallMs = wallStart === undefined ? 0 : now - wallStart;
    const fps = wallMs <= 0 ? 0 : ((presented - 1) * 1000) / wallMs;
    options.stats(
      `decode ${avg.toFixed(1)} ms/frame · shown ${presented} @ ${fps.toFixed(1)} fps · ` +
        `slip ${(slipMs / 1000).toFixed(1)}s · queue ${queue.length}/${QUEUE_DEPTH} · ` +
        `t=${(mediaNowMs / 1000).toFixed(1)}s`,
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

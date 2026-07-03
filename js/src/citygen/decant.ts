import {
  FPS,
  HEIGHT,
  WIDTH,
  citygenFileName,
  type EncodeSettings,
} from "./config";
import { muxVp9Webm, verifyRoundtrip, type MuxPacket } from "./mux";
import type { Scene, SceneParams } from "./scene";

export type DecantProgress = {
  frame: number;
  totalFrames: number;
  fps: number;
  etaS: number;
};

export type DecantResult = {
  webm: Uint8Array;
  packets: number;
  keyframes: number;
  durationMs: number;
  mbps: number;
  elapsedS: number;
  name: string;
};

export type DecantOptions = EncodeSettings & {
  canvas: HTMLCanvasElement;
  scene: Scene;
  params: SceneParams;
  samples?: number;
  hardwareAcceleration?: VideoEncoderConfig["hardwareAcceleration"];
  onProgress?: (progress: DecantProgress) => void;
  signal?: AbortSignal;
};

export async function decantCitygen(
  options: DecantOptions,
): Promise<DecantResult> {
  const totalFrames = Math.round(options.durationS * FPS);
  const kfFrames = Math.max(1, Math.round(options.keyframeIntervalS * FPS));
  const hardwareAcceleration =
    options.hardwareAcceleration ?? "prefer-software";
  const samples = options.samples ?? 4;

  const config: VideoEncoderConfig = {
    codec: "vp09.00.10.08",
    width: WIDTH,
    height: HEIGHT,
    bitrate: options.bitrate,
    framerate: FPS,
    hardwareAcceleration,
    latencyMode: "quality",
  };
  const support = await VideoEncoder.isConfigSupported(config);
  if (!support.supported) {
    throw new Error(
      `VideoEncoder does not support vp09.00.10.08 ${hardwareAcceleration}`,
    );
  }

  const packets: MuxPacket[] = [];
  let encodeError: Error | undefined;
  const encoder = new VideoEncoder({
    output: (chunk) => {
      const payload = new Uint8Array(chunk.byteLength);
      chunk.copyTo(payload);
      packets.push({
        payload,
        timestampMs: Math.round(chunk.timestamp / 1000),
        keyframe: chunk.type === "key",
      });
    },
    error: (error) => {
      encodeError = error;
      // Wake a pending dequeue wait so the encode loop can bail out.
      encoder.dispatchEvent(new Event("dequeue"));
    },
  });

  const started = performance.now();
  try {
    encoder.configure(config);

    for (let frame = 0; frame < totalFrames; frame += 1) {
      if (options.signal?.aborted) {
        throw new Error("decant aborted");
      }
      if (encodeError !== undefined) {
        throw encodeError;
      }

      options.scene.render(frame / FPS, options.params, samples);
      const videoFrame = new VideoFrame(options.canvas, {
        timestamp: Math.round((frame * 1_000_000) / FPS),
        duration: Math.round(1_000_000 / FPS),
      });
      encoder.encode(videoFrame, { keyFrame: frame % kfFrames === 0 });
      videoFrame.close();

      while (encodeError === undefined && encoder.encodeQueueSize > 4) {
        await new Promise((resolve) =>
          encoder.addEventListener("dequeue", resolve, { once: true }),
        );
      }
      if (encodeError !== undefined) {
        throw encodeError;
      }

      const elapsedS = Math.max((performance.now() - started) / 1000, 0.001);
      const fps = (frame + 1) / elapsedS;
      options.onProgress?.({
        frame: frame + 1,
        totalFrames,
        fps,
        etaS: (totalFrames - frame - 1) / Math.max(fps, 0.1),
      });

      if (frame % 4 === 3) {
        await new Promise((resolve) => setTimeout(resolve, 0));
      }
    }

    await encoder.flush();
    if (encodeError !== undefined) {
      throw encodeError;
    }
  } finally {
    encoder.close();
  }

  const durationMs = Math.round((totalFrames * 1000) / FPS);
  const muxOptions = {
    width: WIDTH,
    height: HEIGHT,
    durationMs,
    packets,
  };
  const webm = muxVp9Webm(muxOptions);
  verifyRoundtrip(webm, muxOptions);

  const elapsedS = (performance.now() - started) / 1000;
  const keyframes = packets.filter((packet) => packet.keyframe).length;
  return {
    webm,
    packets: packets.length,
    keyframes,
    durationMs,
    mbps: (webm.byteLength * 8) / (options.durationS * 1_000_000),
    elapsedS,
    name: citygenFileName(options.params, options.durationS),
  };
}

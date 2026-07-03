import { muxVp9Webm, verifyRoundtrip } from "./mux";
import type { MuxPacket } from "./mux";
import { createScene } from "./scene";
import type { SceneParams } from "./scene";

const WIDTH = 1280;
const HEIGHT = 720;
const FPS = 30;

type ParamSpec = {
  key: string;
  label: string;
  min: number;
  max: number;
  step: number;
  def: number;
};

const SCENE_SPECS: (ParamSpec & { key: keyof SceneParams })[] = [
  { key: "seed", label: "seed", min: 0, max: 999, step: 1, def: 1 },
  { key: "speed", label: "speed m/s", min: 5, max: 120, step: 1, def: 40 },
  {
    key: "altitude",
    label: "altitude m",
    min: 60,
    max: 400,
    step: 5,
    def: 195,
  },
  { key: "pitch", label: "pitch °", min: 0, max: 45, step: 1, def: 29 },
  { key: "fov", label: "fov °", min: 30, max: 100, step: 1, def: 62 },
  { key: "sway", label: "sway", min: 0, max: 1, step: 0.01, def: 0.5 },
  { key: "density", label: "density", min: 0.2, max: 1, step: 0.01, def: 0.78 },
  { key: "towers", label: "towers", min: 0, max: 1, step: 0.01, def: 0.35 },
  { key: "lit", label: "lit windows", min: 0, max: 1, step: 0.01, def: 0.35 },
  { key: "warmth", label: "warmth", min: 0, max: 1, step: 0.01, def: 0.45 },
  { key: "fog", label: "fog", min: 0, max: 1, step: 0.01, def: 0.28 },
  { key: "glow", label: "sky glow", min: 0, max: 2, step: 0.01, def: 1 },
  {
    key: "streets",
    label: "street lights",
    min: 0,
    max: 2,
    step: 0.01,
    def: 1,
  },
  { key: "traffic", label: "traffic", min: 0, max: 2, step: 0.01, def: 1 },
  { key: "neon", label: "neon signs", min: 0, max: 2, step: 0.01, def: 1 },
  { key: "grain", label: "grain", min: 0, max: 0.15, step: 0.005, def: 0.03 },
  {
    key: "exposure",
    label: "exposure",
    min: 0.3,
    max: 4,
    step: 0.05,
    def: 1.4,
  },
];

const ENCODE_SPECS: ParamSpec[] = [
  { key: "duration", label: "duration s", min: 1, max: 180, step: 1, def: 60 },
  { key: "mbps", label: "bitrate Mbps", min: 0.5, max: 12, step: 0.5, def: 4 },
  { key: "kf", label: "keyframe s", min: 1, max: 10, step: 1, def: 5 },
];

const ALL_SPECS = [...SCENE_SPECS, ...ENCODE_SPECS];

function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (node === null) {
    throw new Error(`missing element: #${id}`);
  }
  return node as T;
}

const canvas = el<HTMLCanvasElement>("scene-canvas");
canvas.width = WIDTH;
canvas.height = HEIGHT;
const controlsPane = el<HTMLDivElement>("controls");
const pauseButton = el<HTMLButtonElement>("pause");
const restartButton = el<HTMLButtonElement>("restart");
const decantButton = el<HTMLButtonElement>("decant");
const statusLine = el<HTMLDivElement>("status");
const resultVideo = el<HTMLVideoElement>("result-video");

const values = new Map<string, number>(
  ALL_SPECS.map((spec) => [spec.key, spec.def]),
);
const valueLabels = new Map<string, HTMLSpanElement>();

function sceneParams(): SceneParams {
  return Object.fromEntries(
    SCENE_SPECS.map((spec) => [spec.key, values.get(spec.key)]),
  ) as SceneParams;
}

// --- controls, persisted in the URL hash so a look is shareable/reproducible

function hashToValues(): void {
  const params = new URLSearchParams(location.hash.slice(1));
  for (const spec of ALL_SPECS) {
    const raw = params.get(spec.key);
    if (raw === null) {
      continue;
    }
    const value = Number(raw);
    if (Number.isFinite(value)) {
      values.set(spec.key, Math.min(spec.max, Math.max(spec.min, value)));
    }
  }
}

function valuesToHash(): void {
  const params = new URLSearchParams();
  for (const spec of ALL_SPECS) {
    const value = values.get(spec.key) ?? spec.def;
    if (value !== spec.def) {
      params.set(spec.key, String(value));
    }
  }
  history.replaceState(
    null,
    "",
    params.size > 0 ? `#${params}` : location.pathname,
  );
}

function formatValue(spec: ParamSpec, value: number): string {
  const decimals = spec.step >= 1 ? 0 : spec.step >= 0.01 ? 2 : 3;
  return value.toFixed(decimals);
}

function buildControls(): void {
  for (const spec of ALL_SPECS) {
    const row = document.createElement("label");
    row.className = "control";
    if (spec === ENCODE_SPECS[0]) {
      const divider = document.createElement("div");
      divider.className = "divider";
      divider.textContent = "encode";
      controlsPane.append(divider);
    }

    const name = document.createElement("span");
    name.textContent = spec.label;
    const slider = document.createElement("input");
    slider.type = "range";
    slider.min = String(spec.min);
    slider.max = String(spec.max);
    slider.step = String(spec.step);
    slider.value = String(values.get(spec.key));
    const value = document.createElement("span");
    value.className = "value";
    value.textContent = formatValue(spec, values.get(spec.key) ?? spec.def);
    valueLabels.set(spec.key, value);

    slider.addEventListener("input", () => {
      values.set(spec.key, Number(slider.value));
      value.textContent = formatValue(spec, Number(slider.value));
      valuesToHash();
    });
    row.append(name, slider, value);
    controlsPane.append(row);
  }
}

// --- preview loop ----------------------------------------------------------

const scene = createScene(canvas);
let previewTime = 0;
let playing = true;
let decanting = false;
let lastTick: number | undefined;

function tick(now: number): void {
  if (!decanting) {
    if (playing && lastTick !== undefined) {
      previewTime += Math.min((now - lastTick) / 1000, 0.1);
    }
    lastTick = now;
    scene.render(previewTime, sceneParams());
  }
  requestAnimationFrame(tick);
}

pauseButton.addEventListener("click", () => {
  playing = !playing;
  pauseButton.textContent = playing ? "pause" : "play";
});
restartButton.addEventListener("click", () => {
  previewTime = 0;
});

// --- decant ------------------------------------------------------------------

let cancelDecant = false;

function setStatus(message: string, kind: "info" | "error" = "info"): void {
  statusLine.textContent = message;
  statusLine.className = kind;
}

async function decant(): Promise<void> {
  const durationS = values.get("duration") ?? 60;
  const bitrate = (values.get("mbps") ?? 4) * 1_000_000;
  const kfFrames = (values.get("kf") ?? 5) * FPS;
  const totalFrames = Math.round(durationS * FPS);
  const params = sceneParams();

  const config: VideoEncoderConfig = {
    codec: "vp09.00.10.08",
    width: WIDTH,
    height: HEIGHT,
    bitrate,
    framerate: FPS,
    hardwareAcceleration: "prefer-software",
    latencyMode: "quality",
  };
  const support = await VideoEncoder.isConfigSupported(config);
  if (!support.supported) {
    throw new Error(
      "VideoEncoder does not support vp09.00.10.08 prefer-software",
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
  encoder.configure(config);

  const started = performance.now();
  for (let frame = 0; frame < totalFrames; frame += 1) {
    if (cancelDecant || encodeError) {
      break;
    }
    scene.render(frame / FPS, params, 4);
    const videoFrame = new VideoFrame(canvas, {
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
    const elapsed = (performance.now() - started) / 1000;
    const fps = (frame + 1) / elapsed;
    setStatus(
      `decanting ${frame + 1}/${totalFrames} — ${fps.toFixed(1)} fps, ` +
        `eta ${((totalFrames - frame - 1) / Math.max(fps, 0.1)).toFixed(0)}s`,
    );
    await new Promise(requestAnimationFrame);
  }
  if (encodeError) {
    encoder.close();
    throw encodeError;
  }
  if (cancelDecant) {
    encoder.close();
    setStatus("decant cancelled");
    return;
  }
  await encoder.flush();
  encoder.close();

  const durationMs = Math.round((totalFrames * 1000) / FPS);
  const options = { width: WIDTH, height: HEIGHT, durationMs, packets };
  const webm = muxVp9Webm(options);
  verifyRoundtrip(webm, options);

  const keyframes = packets.filter((packet) => packet.keyframe).length;
  const mbps = (webm.byteLength * 8) / (durationS * 1_000_000);
  const seed = values.get("seed") ?? 0;
  const name = `citygen-${WIDTH}x${HEIGHT}p${FPS}-${durationS}s-seed${seed}.webm`;
  const blob = new Blob([webm.buffer as ArrayBuffer], { type: "video/webm" });
  const url = URL.createObjectURL(blob);

  resultVideo.src = url;
  resultVideo.hidden = false;
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();

  setStatus(
    `${name}: ${(webm.byteLength / 1_000_000).toFixed(1)} MB, ${packets.length} packets, ` +
      `${keyframes} keyframes, ${mbps.toFixed(2)} Mbps — roundtrip parse ok, downloaded`,
  );
}

decantButton.addEventListener("click", () => {
  if (decanting) {
    cancelDecant = true;
    return;
  }
  decanting = true;
  cancelDecant = false;
  decantButton.textContent = "cancel";
  decant()
    .catch((error) => {
      setStatus(String(error), "error");
    })
    .finally(() => {
      decanting = false;
      decantButton.textContent = "decant webm";
      lastTick = undefined;
    });
});

hashToValues();
buildControls();
valuesToHash();
requestAnimationFrame(tick);

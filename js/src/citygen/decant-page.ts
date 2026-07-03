import {
  ALL_SPECS,
  HEIGHT,
  WIDTH,
  applyUrlParams,
  defaultValues,
  encodeSettings,
  sceneParams,
} from "./config";
import { decantCitygen, type DecantProgress } from "./decant";
import { createScene, type WebGlInfo } from "./scene";

type DecantStatus = "starting" | "running" | "done" | "error";

type DecantState = {
  ok: boolean;
  status: DecantStatus;
  webgl?: WebGlInfo;
  progress?: DecantProgress;
  result?: {
    name: string;
    bytes: number;
    packets: number;
    keyframes: number;
    durationMs: number;
    mbps: number;
    elapsedS: number;
  };
  error?: string;
};

declare global {
  interface Window {
    __citygenDecantState?: DecantState;
    __citygenDecantBytes?: Uint8Array;
  }
}

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
const statePre = el<HTMLPreElement>("state");

const state: DecantState = { ok: false, status: "starting" };
setState({});

function setState(patch: Partial<DecantState>): void {
  Object.assign(state, patch);
  window.__citygenDecantState = state;
  document.title = `vip9r citygen decant: ${state.status}`;
  statePre.textContent = JSON.stringify(state, null, 2);
}

function searchNumber(key: string, fallback: number): number {
  const raw = new URLSearchParams(location.search).get(key);
  if (raw === null) {
    return fallback;
  }
  const value = Number(raw);
  return Number.isFinite(value) ? value : fallback;
}

function hardwareAcceleration(): VideoEncoderConfig["hardwareAcceleration"] {
  const raw =
    new URLSearchParams(location.search).get("hardwareAcceleration") ??
    "prefer-software";
  if (
    raw === "no-preference" ||
    raw === "prefer-hardware" ||
    raw === "prefer-software"
  ) {
    return raw;
  }
  throw new Error(`invalid hardwareAcceleration: ${raw}`);
}

async function main(): Promise<void> {
  const values = defaultValues(ALL_SPECS);
  applyUrlParams(values, ALL_SPECS, new URLSearchParams(location.search));
  applyUrlParams(
    values,
    ALL_SPECS,
    new URLSearchParams(location.hash.slice(1)),
  );

  const scene = createScene(canvas);
  setState({ webgl: scene.info, status: "running" });

  const params = sceneParams(values);
  const result = await decantCitygen({
    ...encodeSettings(values),
    canvas,
    scene,
    params,
    samples: searchNumber("samples", 4),
    hardwareAcceleration: hardwareAcceleration(),
    onProgress: (progress) => setState({ progress }),
  });

  window.__citygenDecantBytes = result.webm;
  setState({
    ok: true,
    status: "done",
    result: {
      name: result.name,
      bytes: result.webm.byteLength,
      packets: result.packets,
      keyframes: result.keyframes,
      durationMs: result.durationMs,
      mbps: result.mbps,
      elapsedS: result.elapsedS,
    },
  });
}

main().catch((error: unknown) => {
  setState({
    ok: false,
    status: "error",
    error: error instanceof Error ? error.stack ?? error.message : String(error),
  });
});

import type { SceneParams } from "./scene";

export const WIDTH = 1280;
export const HEIGHT = 720;
export const FPS = 30;

export type ParamSpec<Key extends string = string> = {
  key: Key;
  label: string;
  min: number;
  max: number;
  step: number;
  def: number;
};

export const SCENE_SPECS: (ParamSpec<keyof SceneParams> & {
  key: keyof SceneParams;
})[] = [
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
  { key: "pitch", label: "pitch deg", min: 0, max: 45, step: 1, def: 29 },
  { key: "fov", label: "fov deg", min: 30, max: 100, step: 1, def: 62 },
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

export const ENCODE_SPECS: ParamSpec[] = [
  {
    key: "duration",
    label: "duration s",
    min: 1 / FPS,
    max: 180,
    step: 1,
    def: 60,
  },
  { key: "mbps", label: "bitrate Mbps", min: 0.5, max: 12, step: 0.5, def: 4 },
  { key: "kf", label: "keyframe s", min: 1, max: 10, step: 1, def: 5 },
];

export const ALL_SPECS: ParamSpec[] = [...SCENE_SPECS, ...ENCODE_SPECS];

export type EncodeSettings = {
  durationS: number;
  bitrate: number;
  keyframeIntervalS: number;
};

export function defaultValues(
  specs: readonly ParamSpec[],
): Map<string, number> {
  return new Map(specs.map((spec) => [spec.key, spec.def]));
}

export function applyUrlParams(
  values: Map<string, number>,
  specs: readonly ParamSpec[],
  params: URLSearchParams,
): void {
  for (const spec of specs) {
    const raw = params.get(spec.key);
    if (raw === null) {
      continue;
    }
    const value = Number(raw);
    if (Number.isFinite(value)) {
      values.set(spec.key, clamp(value, spec.min, spec.max));
    }
  }
}

export function valuesToSearchParams(
  values: ReadonlyMap<string, number>,
  specs: readonly ParamSpec[],
): URLSearchParams {
  const params = new URLSearchParams();
  for (const spec of specs) {
    const value = values.get(spec.key) ?? spec.def;
    if (value !== spec.def) {
      params.set(spec.key, String(value));
    }
  }
  return params;
}

export function sceneParams(values: ReadonlyMap<string, number>): SceneParams {
  return Object.fromEntries(
    SCENE_SPECS.map((spec) => [spec.key, values.get(spec.key) ?? spec.def]),
  ) as SceneParams;
}

export function encodeSettings(
  values: ReadonlyMap<string, number>,
): EncodeSettings {
  const get = (key: string): number => {
    const spec = ENCODE_SPECS.find((candidate) => candidate.key === key);
    if (spec === undefined) {
      throw new Error(`unknown encode setting: ${key}`);
    }
    return values.get(key) ?? spec.def;
  };
  return {
    durationS: get("duration"),
    bitrate: get("mbps") * 1_000_000,
    keyframeIntervalS: get("kf"),
  };
}

export function formatValue(spec: ParamSpec, value: number): string {
  const decimals = spec.step >= 1 ? 0 : spec.step >= 0.01 ? 2 : 3;
  return value.toFixed(decimals);
}

export function citygenFileName(
  params: SceneParams,
  durationS: number,
): string {
  return `citygen-${WIDTH}x${HEIGHT}p${FPS}-${durationS}s-seed${params.seed}.webm`;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

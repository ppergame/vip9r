import {
  HEIGHT,
  SCENE_SPECS,
  WIDTH,
  applyUrlParams,
  defaultValues,
  formatValue,
  sceneParams,
  valuesToSearchParams,
} from "./config";
import { createScene } from "./scene";

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

const values = defaultValues(SCENE_SPECS);

// --- controls, persisted in the URL hash so a look is shareable/reproducible

function hashToValues(): void {
  applyUrlParams(
    values,
    SCENE_SPECS,
    new URLSearchParams(location.hash.slice(1)),
  );
}

function valuesToHash(): void {
  const params = valuesToSearchParams(values, SCENE_SPECS);
  history.replaceState(
    null,
    "",
    params.size > 0 ? `#${params}` : location.pathname,
  );
}

function buildControls(): void {
  for (const spec of SCENE_SPECS) {
    const row = document.createElement("label");
    row.className = "control";

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

    slider.addEventListener("input", () => {
      const next = Number(slider.value);
      values.set(spec.key, next);
      value.textContent = formatValue(spec, next);
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
let lastTick: number | undefined;

function tick(now: number): void {
  if (playing && lastTick !== undefined) {
    previewTime += Math.min((now - lastTick) / 1000, 0.1);
  }
  lastTick = now;
  scene.render(previewTime, sceneParams(values));
  requestAnimationFrame(tick);
}

pauseButton.addEventListener("click", () => {
  playing = !playing;
  pauseButton.textContent = playing ? "pause" : "play";
});
restartButton.addEventListener("click", () => {
  previewTime = 0;
});

hashToValues();
buildControls();
valuesToHash();
requestAnimationFrame(tick);

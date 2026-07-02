import wasmUrl from "../../../rust/target/wasm32-unknown-unknown/release/vip9r.wasm?url";
import { Vp9Decoder } from "../wasm";
import { formatWasmLog, instanceMemory, makeVip9rImports } from "../wasm-driver/wasm-env";
import { startPlayback } from "./player";
import type { PlaybackHandle } from "./player";

type Mode = "play" | "race";

type CannedClip = { name: string; path: string; label: string };

const CANNED: CannedClip[] = [
  { name: "bear", path: "chromium/bear-vp9.ivf", label: "bear — 320p smoke clip" },
  {
    name: "jellyfish",
    path: "realworld/test-videos/jellyfish-720p30-1_68mbps.webm",
    label: "jellyfish — 720p30",
  },
  {
    name: "bbb",
    path: "realworld/wikimedia/big-buck-bunny-720p25-1_54mbps.webm",
    label: "big buck bunny — 720p25",
  },
  {
    name: "caminandes",
    path: "realworld/wikimedia/caminandes-gran-dillama-720p24-1_55mbps.webm",
    label: "caminandes — 720p24",
  },
  {
    name: "cosmos",
    path: "realworld/wikimedia/cosmos-laundromat-720p24-1_85mbps.webm",
    label: "cosmos laundromat — 720p24",
  },
  {
    name: "tears",
    path: "realworld/wikimedia/tears-of-steel-720p24-1_92mbps.webm",
    label: "tears of steel — 720p24",
  },
];

const CUSTOM = "custom";

function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (node === null) {
    throw new Error(`missing element: #${id}`);
  }
  return node as T;
}

const mediaSelect = el<HTMLSelectElement>("media-select");
const mediaUrl = el<HTMLInputElement>("media-url");
const copyLink = el<HTMLButtonElement>("copy-link");
const tabs: Record<Mode, HTMLButtonElement> = {
  play: el<HTMLButtonElement>("tab-play"),
  race: el<HTMLButtonElement>("tab-race"),
};
const sections: Record<Mode, HTMLElement> = {
  play: el<HTMLElement>("play"),
  race: el<HTMLElement>("race"),
};
const wcSelect = el<HTMLSelectElement>("wc");
const lanesSelect = el<HTMLSelectElement>("lanes");
const logPane = el<HTMLDivElement>("log");

function log(message: string, kind: "info" | "error" = "info"): void {
  const line = document.createElement("div");
  line.textContent = `${new Date().toISOString().slice(11, 19)} ${message}`;
  if (kind === "error") {
    line.className = "error";
  }
  logPane.append(line);
  logPane.scrollTop = logPane.scrollHeight;
}

window.addEventListener("error", (event) => {
  log(String(event.message), "error");
});
window.addEventListener("unhandledrejection", (event) => {
  log(String(event.reason), "error");
});

// All shareable state lives in the URL. Defaults are omitted from it.
const DEFAULTS: Record<string, string> = {
  mode: "play",
  media: CANNED[0].name,
  wc: "software",
  lanes: "both",
};

function setParam(key: string, value: string): void {
  const params = new URLSearchParams(location.search);
  if (value === DEFAULTS[key]) {
    params.delete(key);
  } else {
    params.set(key, value);
  }
  const query = params.toString();
  history.replaceState(null, "", query === "" ? location.pathname : `?${query}`);
}

function getParam(key: string): string {
  return new URLSearchParams(location.search).get(key) ?? DEFAULTS[key];
}

function setMode(mode: Mode): void {
  for (const m of ["play", "race"] as const) {
    tabs[m].classList.toggle("active", m === mode);
    sections[m].hidden = m !== mode;
  }
  setParam("mode", mode);
}

function syncMediaControls(): void {
  mediaUrl.hidden = mediaSelect.value !== CUSTOM;
  setParam("media", mediaSelect.value === CUSTOM ? mediaUrl.value.trim() : mediaSelect.value);
}

type MediaChoice = { label: string; url: string };

function selectedMedia(): MediaChoice | undefined {
  if (mediaSelect.value === CUSTOM) {
    const url = mediaUrl.value.trim();
    return url === "" ? undefined : { label: url, url };
  }
  const clip = CANNED.find((c) => c.name === mediaSelect.value);
  if (clip === undefined) {
    return undefined;
  }
  return { label: clip.name, url: `/media/${clip.path}` };
}

async function fetchMedia(): Promise<{ choice: MediaChoice; bytes: ArrayBuffer } | undefined> {
  const choice = selectedMedia();
  if (choice === undefined) {
    log("no media selected", "error");
    return undefined;
  }
  log(`fetching ${choice.label}`);
  const response = await fetch(choice.url);
  if (!response.ok) {
    log(`media fetch failed: ${response.status} ${response.statusText}`, "error");
    return undefined;
  }
  const bytes = await response.arrayBuffer();
  log(`media ready: ${choice.label}, ${bytes.byteLength} bytes`);
  return { choice, bytes };
}

let playback: PlaybackHandle | undefined;

async function startPlay(): Promise<void> {
  const media = await fetchMedia();
  if (media === undefined) {
    return;
  }
  playback?.stop();
  playback = startPlayback({
    media: media.bytes,
    canvas: el<HTMLCanvasElement>("play-canvas"),
    log,
    stats: (text) => {
      el<HTMLSpanElement>("play-stats").textContent = text;
    },
  });
}

async function startRace(): Promise<void> {
  if ((await fetchMedia()) === undefined) {
    return;
  }
  log(
    `race: lanes=${lanesSelect.value} webcodecs=${wcSelect.value} — not implemented yet`,
    "error",
  );
}

async function bootWasmCheck(): Promise<void> {
  let memory: WebAssembly.Memory | undefined;
  const imports = makeVip9rImports(
    () => {
      if (memory === undefined) {
        throw new Error("wasm logged before instantiation completed");
      }
      return memory;
    },
    (entry) => log(formatWasmLog(entry), "error"),
  );
  const { instance } = await WebAssembly.instantiateStreaming(fetch(wasmUrl), imports);
  memory = instanceMemory(instance);
  new Vp9Decoder(instance, 1280, 720);
  log(`wasm ok: ${wasmUrl.split("/").pop()}`);
}

function initControls(): void {
  for (const clip of CANNED) {
    const option = document.createElement("option");
    option.value = clip.name;
    option.textContent = clip.label;
    mediaSelect.append(option);
  }
  const custom = document.createElement("option");
  custom.value = CUSTOM;
  custom.textContent = "custom URL…";
  mediaSelect.append(custom);

  const media = getParam("media");
  if (CANNED.some((c) => c.name === media)) {
    mediaSelect.value = media;
  } else {
    mediaSelect.value = CUSTOM;
    mediaUrl.value = media;
    mediaUrl.hidden = false;
  }
  wcSelect.value = getParam("wc");
  lanesSelect.value = getParam("lanes");
  setMode(getParam("mode") === "race" ? "race" : "play");

  mediaSelect.addEventListener("change", syncMediaControls);
  mediaUrl.addEventListener("change", syncMediaControls);
  wcSelect.addEventListener("change", () => setParam("wc", wcSelect.value));
  lanesSelect.addEventListener("change", () => setParam("lanes", lanesSelect.value));
  tabs.play.addEventListener("click", () => setMode("play"));
  tabs.race.addEventListener("click", () => setMode("race"));
  el<HTMLButtonElement>("play-start").addEventListener("click", () => void startPlay());
  el<HTMLButtonElement>("race-start").addEventListener("click", () => void startRace());
  copyLink.addEventListener("click", () => {
    void navigator.clipboard.writeText(location.href).then(() => log(`link: ${location.href}`));
  });
}

async function main(): Promise<void> {
  initControls();
  await bootWasmCheck();
  if (getParam("autostart") === "1") {
    log("autostart");
    await (getParam("mode") === "race" ? startRace() : startPlay());
  }
}

void main();

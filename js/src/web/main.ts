import { Vp9Decoder } from "../wasm";
import { startBench } from "./bench";
import type { BenchHandle } from "./bench";
import { startPlayback } from "./player";
import type { PlaybackHandle } from "./player";
import { Sparkline } from "./sparkline";
import { instantiateVip9r, wasmUrl } from "./vip9r-instance";

type Mode = "play" | "bench";

type CannedClip = { name: string; path: string; label: string };

const CANNED: CannedClip[] = [
  {
    name: "citygen",
    path: "synthetic/citygen-720p30-1-5-mbps.webm",
    label: "citygen - 720p",
  },
  {
    name: "bear",
    path: "chromium/bear-vp9.ivf",
    label: "bear - 320p smoke clip",
  },
  {
    name: "jellyfish",
    path: "realworld/test-videos/jellyfish-720p30-1_68mbps.webm",
    label: "jellyfish - 720p30",
  },
  {
    name: "youtube-coral",
    path: "youtube/mN9_buCmKLE/mN9_buCmKLE-f247-720p30-vp9-rawprefix-0000-2000.webm",
    label: "youtube coral f247 - 720p30",
  },
  {
    name: "bbb",
    path: "realworld/wikimedia/big-buck-bunny-720p25-1_54mbps.webm",
    label: "big buck bunny - 720p25",
  },
  {
    name: "caminandes",
    path: "realworld/wikimedia/caminandes-gran-dillama-720p24-1_55mbps.webm",
    label: "caminandes - 720p24",
  },
  {
    name: "cosmos",
    path: "realworld/wikimedia/cosmos-laundromat-720p24-1_85mbps.webm",
    label: "cosmos laundromat - 720p24",
  },
  {
    name: "tears",
    path: "realworld/wikimedia/tears-of-steel-720p24-1_92mbps.webm",
    label: "tears of steel - 720p24",
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
  bench: el<HTMLButtonElement>("tab-bench"),
};
const sections: Record<Mode, HTMLElement> = {
  play: el<HTMLElement>("play"),
  bench: el<HTMLElement>("bench"),
};
const framesInput = el<HTMLInputElement>("frames");
const logPane = el<HTMLDivElement>("log");

function log(message: string, kind: "info" | "error" = "info"): void {
  (kind === "error" ? console.error : console.log)(message);
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
  frames: "300",
};

function setParam(key: string, value: string): void {
  const params = new URLSearchParams(location.search);
  if (value === DEFAULTS[key]) {
    params.delete(key);
  } else {
    params.set(key, value);
  }
  const query = params.toString();
  history.replaceState(
    null,
    "",
    query === "" ? location.pathname : `?${query}`,
  );
}

function getParam(key: string): string {
  return new URLSearchParams(location.search).get(key) ?? DEFAULTS[key];
}

function setMode(mode: Mode): void {
  for (const m of ["play", "bench"] as const) {
    tabs[m].classList.toggle("active", m === mode);
    sections[m].hidden = m !== mode;
  }
  setParam("mode", mode);
}

function onMediaChanged(): void {
  mediaUrl.hidden = mediaSelect.value !== CUSTOM;
  setParam(
    "media",
    mediaSelect.value === CUSTOM ? mediaUrl.value.trim() : mediaSelect.value,
  );
  // The page always autostarts; picking media restarts through a clean load.
  if (mediaSelect.value !== CUSTOM || mediaUrl.value.trim() !== "") {
    location.reload();
  }
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

let playback: PlaybackHandle | undefined;
let bench: BenchHandle | undefined;
const playSpark = new Sparkline(el<HTMLCanvasElement>("play-spark"));

function resetSessions(): void {
  playback?.stop();
  playback = undefined;
  bench?.stop();
  bench = undefined;
  el<HTMLSpanElement>("play-stats").textContent = "";
  playSpark.reset();
  const canvas = el<HTMLCanvasElement>("play-canvas");
  canvas.getContext("2d")?.clearRect(0, 0, canvas.width, canvas.height);
  el<HTMLPreElement>("bench-results").textContent = "";
}

function switchMode(mode: Mode): void {
  if (getParam("mode") === mode) {
    return;
  }
  resetSessions();
  setMode(mode);
  if (mode === "bench") {
    startBenchRun();
  } else {
    startPlay();
  }
}

function startPlay(): void {
  const choice = selectedMedia();
  if (choice === undefined) {
    log("no media selected", "error");
    return;
  }
  resetSessions();
  log(`streaming ${choice.label}`);
  // Only an explicit &frames=N caps playback; the frames input's default is
  // a bench-tab affair, and a plain page load plays the whole clip.
  const explicitFrames = new URLSearchParams(location.search).get("frames");
  playback = startPlayback({
    url: choice.url,
    frameLimit: Math.max(0, Number(explicitFrames) || 0),
    canvas: el<HTMLCanvasElement>("play-canvas"),
    log,
    stats: (text) => {
      el<HTMLSpanElement>("play-stats").textContent = text;
    },
    onFrameDecoded: (decodeMs, budgetMs) => playSpark.push(decodeMs, budgetMs),
  });
}

function startBenchRun(): void {
  const choice = selectedMedia();
  if (choice === undefined) {
    log("no media selected", "error");
    return;
  }
  resetSessions();
  log(`streaming ${choice.label}`);
  bench = startBench({
    url: choice.url,
    packetLimit: Math.max(0, Number(framesInput.value) || 0),
    log,
    results: (text) => {
      el<HTMLPreElement>("bench-results").textContent = text;
    },
  });
}

async function bootWasmCheck(): Promise<void> {
  const { instance } = await instantiateVip9r(
    { width: 1280, height: 720 },
    (message) => log(message, "error"),
  );
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
  framesInput.value = getParam("frames");
  setMode(getParam("mode") === "bench" ? "bench" : "play");

  mediaSelect.addEventListener("change", onMediaChanged);
  mediaUrl.addEventListener("change", onMediaChanged);
  framesInput.addEventListener("change", () =>
    setParam("frames", framesInput.value),
  );
  tabs.play.addEventListener("click", () => switchMode("play"));
  tabs.bench.addEventListener("click", () => switchMode("bench"));
  el<HTMLButtonElement>("play-start").addEventListener("click", () =>
    startPlay(),
  );
  el<HTMLButtonElement>("bench-start").addEventListener("click", () =>
    startBenchRun(),
  );
  copyLink.addEventListener("click", () => {
    void navigator.clipboard
      .writeText(location.href)
      .then(() => log(`link: ${location.href}`));
  });
}

async function main(): Promise<void> {
  initControls();
  await bootWasmCheck();
  if (getParam("mode") === "bench") {
    startBenchRun();
  } else {
    startPlay();
  }
}

void main();

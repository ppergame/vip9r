import { Vp9Decoder } from "../wasm";
import { startBench, type BenchRow } from "./bench";
import type { BenchHandle } from "./bench";
import { startPlayback } from "./player";
import type { PlaybackHandle } from "./player";
import { Sparkline } from "./sparkline";
import { instantiateVip9r, wasmUrl } from "./vip9r-instance";

type Mode = "play" | "bench";

type CannedClip = { name: string; path: string; label: string };

const CANNED: CannedClip[] = [
  {
    name: "youtube-coral",
    path: "youtube/mN9_buCmKLE/mN9_buCmKLE-f247-720p30-vp9-rawprefix-0000-2000.webm",
    label: "youtube coral - 720p30",
  },
  {
    name: "citygen",
    path: "synthetic/citygen-720p30-1-5-mbps.webm",
    label: "citygen - 720p30",
  },
  {
    name: "bear",
    path: "chromium/bear-vp9.ivf",
    label: "bear - 320p",
  },
  {
    name: "jellyfish",
    path: "realworld/test-videos/jellyfish-720p30-1_68mbps.webm",
    label: "jellyfish - 720p30",
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
const MEDIA_BASE_URL = import.meta.env.DEV
  ? "/media"
  : "https://media.xzrq.net/vip9r";

function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (node === null) {
    throw new Error(`missing element: #${id}`);
  }
  return node as T;
}

const mediaSelect = el<HTMLSelectElement>("media-select");
const mediaUrl = el<HTMLInputElement>("media-url");
const tabs: Record<Mode, HTMLButtonElement> = {
  play: el<HTMLButtonElement>("tab-play"),
  bench: el<HTMLButtonElement>("tab-bench"),
};
const sections: Record<Mode, HTMLElement> = {
  play: el<HTMLElement>("play"),
  bench: el<HTMLElement>("bench"),
};
const controls: Record<Mode, HTMLElement> = {
  play: el<HTMLElement>("play-controls"),
  bench: el<HTMLElement>("bench-controls"),
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
  frames: "100",
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
    controls[m].hidden = m !== mode;
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

// Steps through the canned clips only: the custom-URL entry needs a keyboard,
// so the remote skips it. From a custom URL, the first step lands on the
// nearest end of the canned list.
function cycleMedia(step: number): void {
  const index = CANNED.findIndex((c) => c.name === mediaSelect.value);
  const next =
    index === -1
      ? step > 0
        ? 0
        : CANNED.length - 1
      : (index + step + CANNED.length) % CANNED.length;
  mediaSelect.value = CANNED[next].name;
  onMediaChanged();
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
  return { label: clip.name, url: `${MEDIA_BASE_URL}/${clip.path}` };
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
  el<HTMLDivElement>("bench-results").innerHTML = "";
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
  // Perf-attribution probes, e.g. &probe=discard,prebuffer or &probe=nodraw.
  // "nospark" mutes the per-frame sparkline redraw; the rest are handled in
  // player.ts / decode-worker.ts.
  const probe = (new URLSearchParams(location.search).get("probe") ?? "")
    .split(",")
    .filter((flag) => flag !== "");
  if (probe.length > 0) {
    log(`probe: ${probe.join(",")}`);
  }
  playback = startPlayback({
    url: choice.url,
    frameLimit: Math.max(0, Number(explicitFrames) || 0),
    probe,
    canvas: el<HTMLCanvasElement>("play-canvas"),
    log,
    stats: (text) => {
      el<HTMLSpanElement>("play-stats").textContent = text;
    },
    onFrameDecoded: probe.includes("nospark")
      ? undefined
      : (decodeMs, budgetMs) => playSpark.push(decodeMs, budgetMs),
  });
}

function renderBenchResults(rows: BenchRow[], budgetMs: number): void {
  // Bars encode realtime speed, so longer reads as better at a glance. The
  // tick marks 1.0×; a lane that falls short of it draws all red.
  const maxRt = Math.max(...rows.map((r) => r.realtime ?? 0), 1);
  const lanes = rows.filter((r) => r.skipped === undefined);
  const wallDigits = Math.max(...lanes.map((r) => r.wallMs!.toFixed(0).length));
  const frameDigits = Math.max(...lanes.map((r) => String(r.frames).length));
  // Pad with figure spaces so "60 in 24 ms" lines up under "60 in 128 ms".
  const pad = (text: string, width: number) => text.padStart(width, " ");
  const tick =
    budgetMs > 0
      ? `<div class="tick" style="left:${((1 / maxRt) * 100).toFixed(2)}%"></div>`
      : "";
  const cells = rows.map((row) => {
    if (row.skipped !== undefined) {
      return `<tr><td>${row.label}</td><td class="skip" colspan="5">skipped — ${row.skipped}</td></tr>`;
    }
    const notes =
      row.decodeMsPerFrame === undefined
        ? ""
        : `decode ${row.decodeMsPerFrame.toFixed(1)} + VideoFrame ${row.videoFrameMsPerFrame!.toFixed(1)} ms`;
    const rt = row.realtime;
    const bar =
      rt === undefined
        ? ""
        : `<div class="bar${rt < 1 ? " under" : ""}" style="width:${((rt / maxRt) * 100).toFixed(2)}%"></div>`;
    return (
      `<tr><td>${row.label}</td>` +
      `<td class="num">${row.msPerFrame!.toFixed(1)}</td>` +
      `<td class="num">${rt === undefined ? "" : `${rt.toFixed(1)}×`}</td>` +
      `<td class="bar-cell"><div class="track">${bar}${tick}</div></td>` +
      `<td class="num">${pad(String(row.frames), frameDigits)} in ${pad(row.wallMs!.toFixed(0), wallDigits)} ms</td>` +
      `<td class="notes">${notes}</td></tr>`
    );
  });
  el<HTMLDivElement>("bench-results").innerHTML =
    `<table><thead><tr>` +
    `<th>decoder</th><th class="num">ms/frame</th><th class="num">realtime</th>` +
    `<th>${budgetMs > 0 ? "vs 1.0×" : ""}</th>` +
    `<th class="num">frames</th><th></th>` +
    `</tr></thead><tbody>${cells.join("")}</tbody></table>`;
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
    results: renderBenchResults,
  });
}

async function bootWasmCheck(): Promise<void> {
  const { instance } = await instantiateVip9r((message) =>
    log(message, "error"),
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
  framesInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter") startBenchRun();
  });
  tabs.play.addEventListener("click", () => switchMode("play"));
  tabs.bench.addEventListener("click", () => switchMode("bench"));
  el<HTMLButtonElement>("play-start").addEventListener("click", () =>
    startPlay(),
  );
  el<HTMLButtonElement>("bench-start").addEventListener("click", () =>
    startBenchRun(),
  );
  // Google TV remote: Android delivers the D-pad to web content as
  // Enter/arrow keys. When focus isn't on a control that owns those keys
  // itself, select starts the active tab, left/right toggles play/bench,
  // up/down steps through the canned clips.
  document.addEventListener("keydown", (event) => {
    const target = event.target;
    if (
      target instanceof HTMLInputElement ||
      target instanceof HTMLSelectElement ||
      target instanceof HTMLButtonElement
    ) {
      return;
    }
    switch (event.key) {
      case "Enter":
        if (getParam("mode") === "bench") {
          startBenchRun();
        } else {
          startPlay();
        }
        break;
      case "ArrowLeft":
      case "ArrowRight":
        switchMode(getParam("mode") === "bench" ? "play" : "bench");
        break;
      case "ArrowUp":
      case "ArrowDown":
        cycleMedia(event.key === "ArrowDown" ? 1 : -1);
        break;
      default:
        return;
    }
    event.preventDefault();
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

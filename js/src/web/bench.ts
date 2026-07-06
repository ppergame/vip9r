import type {
  BenchEvent,
  BenchInit,
  BenchLane,
  LaneResult,
} from "./bench-worker";

export type BenchHandle = {
  stop(): void;
};

export type BenchRow = {
  label: string;
  skipped?: string;
  msPerFrame?: number;
  realtime?: number;
  frames?: number;
  wallMs?: number;
  decodeMsPerFrame?: number;
  videoFrameMsPerFrame?: number;
};

export type BenchOptions = {
  url: string;
  packetLimit: number;
  log: (message: string, kind?: "info" | "error") => void;
  results: (rows: BenchRow[], budgetMs: number) => void;
};

const LANE_LABELS: Record<BenchLane, string> = {
  vip9r: "vip9r",
  ogv: "ogv.js",
  "wc-sw": "webcodecs software",
  "wc-hw": "webcodecs hardware",
};

export function startBench(options: BenchOptions): BenchHandle {
  const worker = new Worker(new URL("./bench-worker.ts", import.meta.url), {
    type: "module",
  });
  const rows: BenchRow[] = [];
  let budgetMs = 0;
  let stopped = false;

  const init: BenchInit = {
    url: options.url,
    packetLimit: options.packetLimit,
  };
  worker.postMessage(init);

  function addRow(row: BenchRow): void {
    rows.push(row);
    options.results(rows, budgetMs);
  }

  function laneRow(result: LaneResult): BenchRow {
    const msPerFrame = result.frames === 0 ? 0 : result.wallMs / result.frames;
    const row: BenchRow = {
      label: LANE_LABELS[result.lane],
      msPerFrame,
      frames: result.frames,
      wallMs: result.wallMs,
    };
    if (budgetMs > 0 && msPerFrame > 0) {
      row.realtime = budgetMs / msPerFrame;
    }
    if (
      result.decodeMs !== undefined &&
      result.videoFrameMs !== undefined &&
      result.frames > 0
    ) {
      row.decodeMsPerFrame = result.decodeMs / result.frames;
      row.videoFrameMsPerFrame = result.videoFrameMs / result.frames;
    }
    return row;
  }

  function finish(): void {
    if (stopped) {
      return;
    }
    stopped = true;
    worker.terminate();
  }

  worker.onmessage = (event: MessageEvent<BenchEvent>) => {
    const message = event.data;
    switch (message.type) {
      case "meta":
        budgetMs = message.budgetMs;
        options.log(
          `bench: ${message.container} ${message.width}×${message.height}, ` +
            `${message.packets} packets, budget ${budgetMs.toFixed(1)} ms/frame`,
        );
        break;
      case "log":
        options.log(message.message, message.error ? "error" : "info");
        break;
      case "lane":
        addRow(laneRow(message.result));
        break;
      case "lane-skipped":
        addRow({ label: LANE_LABELS[message.lane], skipped: message.reason });
        break;
      case "done":
        options.log("bench done");
        finish();
        break;
      case "error":
        options.log(`bench: ${message.message}`, "error");
        finish();
        break;
    }
  };
  worker.onerror = (event) => {
    options.log(`bench worker: ${event.message}`, "error");
    finish();
  };

  return { stop: finish };
}

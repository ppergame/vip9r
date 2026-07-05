import type {
  BenchEvent,
  BenchInit,
  BenchLane,
  LaneResult,
} from "./bench-worker";

export type BenchHandle = {
  stop(): void;
};

export type BenchOptions = {
  url: string;
  packetLimit: number;
  log: (message: string, kind?: "info" | "error") => void;
  results: (text: string) => void;
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
  const lines: string[] = [];
  let budgetMs = 0;
  let stopped = false;

  const init: BenchInit = {
    url: options.url,
    packetLimit: options.packetLimit,
  };
  worker.postMessage(init);

  function addLine(line: string): void {
    lines.push(line);
    options.results(lines.join("\n"));
  }

  function formatLane(result: LaneResult): string {
    const msPerFrame = result.frames === 0 ? 0 : result.wallMs / result.frames;
    let line =
      `${LANE_LABELS[result.lane].padEnd(19)} ${msPerFrame.toFixed(1).padStart(6)} ms/frame` +
      ` · ${result.frames} frames in ${result.wallMs.toFixed(0)} ms`;
    if (budgetMs > 0 && msPerFrame > 0) {
      line += ` · ${(budgetMs / msPerFrame).toFixed(1)}× realtime`;
    }
    if (
      result.decodeMs !== undefined &&
      result.videoFrameMs !== undefined &&
      result.frames > 0
    ) {
      line +=
        ` · decodeNext ${(result.decodeMs / result.frames).toFixed(1)}` +
        ` + VideoFrame ${(result.videoFrameMs / result.frames).toFixed(1)} ms/frame`;
    }
    return line;
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
        addLine(formatLane(message.result));
        break;
      case "lane-skipped":
        addLine(
          `${LANE_LABELS[message.lane].padEnd(19)} skipped — ${message.reason}`,
        );
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

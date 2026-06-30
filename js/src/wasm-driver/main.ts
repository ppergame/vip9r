import {
  benchmarkWasmGolden,
  compareWasmToGolden,
  DriverUsageError,
  extraCount,
  formatWasmLog,
  formatReport,
  formatProgress,
  matchedCount,
  missingCount,
  mismatchCount,
  parseDriverArgs,
  passes,
} from "./golden";
import type { DriverArgs } from "./golden";

declare const read: (path: string) => string;
declare const readbuffer: (path: string) => ArrayBuffer;
declare const print: (...values: unknown[]) => void;
declare const quit: (code?: number) => never;

type D8Global = typeof globalThis & {
  arguments?: string[];
  scriptArgs?: string[];
};

function main(args: string[]): void {
  if (args.includes("-h") || args.includes("--help")) {
    printUsage();
    quit(0);
  }

  let driverArgs: DriverArgs;
  try {
    driverArgs = parseDriverArgs(args);
  } catch (error) {
    if (error instanceof DriverUsageError) {
      printUsage();
      quit(2);
    }
    throw error;
  }

  if (driverArgs.bench !== undefined) {
    const wasmLogs: string[] = [];
    const report = benchmarkWasmGolden(driverArgs, {
      read,
      readbuffer,
      log(log) {
        wasmLogs.push(formatWasmLog(log));
      },
      now: nowMs,
    });
    print(JSON.stringify(wasmLogs.length === 0 ? report : { ...report, wasmLogs }));
    return;
  }

  const report = compareWasmToGolden(driverArgs, {
    read,
    readbuffer,
    log(log) {
      print(formatWasmLog(log));
    },
    progress(event) {
      print(formatProgress(event));
    },
  });
  for (const line of formatReport(report)) {
    print(line);
  }

  if (!passes(report, driverArgs.allowMismatch)) {
    throw new Error(
      `golden mismatch: ${matchedCount(report)} matched, ${mismatchCount(report)} mismatched, ${missingCount(report)} missing, ${extraCount(report)} extra`,
    );
  }
}

function printUsage(): void {
  print(
    "usage: wasm-golden [--allow-mismatch] [--progress-frames=N] [input.ivf|input.webm [input.md5]]",
  );
  print(
    "       wasm-golden --bench [--bench-frames START:LAST] [--bench-warmup-ms=N] [--bench-target-ms=N] [input.ivf|input.webm [input.md5]]",
  );
  print(
    "       d8 dist/wasm-driver/golden.js -- vip9r.wasm [same options and inputs]",
  );
}

function nowMs(): number {
  if (typeof performance === "object" && typeof performance.now === "function") {
    return performance.now();
  }
  return Date.now();
}

try {
  const d8 = globalThis as D8Global;
  main(d8.scriptArgs ?? d8.arguments ?? []);
} catch (error) {
  print(error instanceof Error && error.stack ? error.stack : String(error));
  quit(1);
}

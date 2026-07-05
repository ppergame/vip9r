import {
  benchmarkWasmGolden,
  compareWasmToGolden,
  DriverUsageError,
  extraCount,
  formatWasmLog,
  formatProgress,
  matchedCount,
  missingCount,
  mismatchCount,
  parseDriverArgs,
  passes,
} from "./golden";
import type {
  ComparisonReport,
  DriverArgs,
  FrameComparison,
  WasmLog,
} from "./golden";

declare const read: (path: string) => string;
declare const readbuffer: (path: string) => ArrayBuffer;
declare const print: (...values: unknown[]) => void;
declare const printErr: (...values: unknown[]) => void;
declare const quit: (code?: number) => never;

type D8Global = typeof globalThis & {
  arguments?: string[];
  scriptArgs?: string[];
};

type GoldenJsonReport = {
  mode: "golden";
  ok: boolean;
  input: string;
  golden: string;
  allowMismatch: boolean;
  pool: boolean;
  container: "ivf" | "webm";
  codec: "VP90" | "V_VP9";
  width: number;
  height: number;
  timebaseDenominator?: number;
  timebaseNumerator?: number;
  declaredFrameCount?: number;
  timestampScale?: number;
  packetCount: number;
  codedFrames: number;
  decodedOutputFrames: number;
  skippedOutputFrames: number;
  expectedCount: number;
  comparedFrames: number;
  matchedCount: number;
  mismatchedCount: number;
  missingCount: number;
  extraCount: number;
  mismatches: FrameComparison[];
  finalMemoryBytes?: number;
  wasmLogs?: WasmLog[];
};

const MAX_REPORTED_MISMATCHES = 10;

function main(args: string[]): void {
  if (args.includes("-h") || args.includes("--help")) {
    printUsage();
    quit(0);
  }

  const driverArgs = parseDriverArgs(args);

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
    print(
      JSON.stringify(wasmLogs.length === 0 ? report : { ...report, wasmLogs }),
    );
    return;
  }

  const wasmLogs: WasmLog[] = [];
  const report = compareWasmToGolden(driverArgs, {
    read,
    readbuffer,
    log(log) {
      wasmLogs.push(log);
      printStderr(formatWasmLog(log));
    },
    progress(event) {
      printStderr(formatProgress(event));
    },
  });
  const jsonReport = makeGoldenJsonReport(report, driverArgs, wasmLogs);
  print(JSON.stringify(jsonReport));

  if (!jsonReport.ok) {
    quit(1);
  }
}

function makeGoldenJsonReport(
  report: ComparisonReport,
  args: DriverArgs,
  wasmLogs: WasmLog[],
): GoldenJsonReport {
  const jsonReport: GoldenJsonReport = {
    mode: "golden",
    ok: passes(report, args.allowMismatch),
    input: report.inputPath,
    golden: report.goldenPath,
    allowMismatch: args.allowMismatch,
    pool: args.pool,
    container: report.container,
    codec: report.codec,
    width: report.width,
    height: report.height,
    packetCount: report.packetCount,
    codedFrames: report.codedFrames,
    decodedOutputFrames: report.decodedOutputFrames,
    skippedOutputFrames: report.skippedOutputFrames,
    expectedCount: report.expectedCount,
    comparedFrames: report.comparisons.length,
    matchedCount: matchedCount(report),
    mismatchedCount: mismatchCount(report),
    missingCount: missingCount(report),
    extraCount: extraCount(report),
    mismatches: report.comparisons
      .filter((comparison) => comparison.expectedMd5 !== comparison.actualMd5)
      .slice(0, MAX_REPORTED_MISMATCHES),
  };
  if (report.timebaseDenominator !== undefined) {
    jsonReport.timebaseDenominator = report.timebaseDenominator;
  }
  if (report.timebaseNumerator !== undefined) {
    jsonReport.timebaseNumerator = report.timebaseNumerator;
  }
  if (report.declaredFrameCount !== undefined) {
    jsonReport.declaredFrameCount = report.declaredFrameCount;
  }
  if (report.timestampScale !== undefined) {
    jsonReport.timestampScale = report.timestampScale;
  }
  if (report.finalMemoryBytes !== undefined) {
    jsonReport.finalMemoryBytes = report.finalMemoryBytes;
  }
  if (wasmLogs.length !== 0) {
    jsonReport.wasmLogs = wasmLogs;
  }
  return jsonReport;
}

function printUsage(out: (...values: unknown[]) => void = print): void {
  out(
    "usage: wasm-golden [--allow-mismatch] [--pool] [--frames START:LAST] [--progress-frames=N] [input.ivf|input.webm]",
  );
  out(
    "       wasm-golden --bench [--pool] [--frames START:LAST] [input.ivf|input.webm]",
  );
}

function printStderr(...values: unknown[]): void {
  if (typeof printErr === "function") {
    printErr(...values);
    return;
  }
  print(...values);
}

function nowMs(): number {
  if (
    typeof performance === "object" &&
    typeof performance.now === "function"
  ) {
    return performance.now();
  }
  return Date.now();
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.stack ?? error.message;
  }
  return String(error);
}

function modeFromArgs(args: string[]): "bench" | "golden" {
  return args.includes("--bench") ? "bench" : "golden";
}

try {
  const d8 = globalThis as D8Global;
  main(d8.scriptArgs ?? d8.arguments ?? []);
} catch (error) {
  const d8 = globalThis as D8Global;
  const args = d8.scriptArgs ?? d8.arguments ?? [];
  if (error instanceof DriverUsageError) {
    printUsage(printStderr);
    print(
      JSON.stringify({
        mode: modeFromArgs(args),
        ok: false,
        error: error.message,
      }),
    );
    quit(2);
  }
  print(
    JSON.stringify({
      mode: modeFromArgs(args),
      ok: false,
      error: errorMessage(error),
    }),
  );
  quit(1);
}

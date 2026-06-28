import {
  compareWasmToGolden,
  extraCount,
  formatReport,
  formatProgress,
  matchedCount,
  missingCount,
  mismatchCount,
  passes,
  formatWasmLog,
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
  const driverArgs = parseArgs(args);
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

function parseArgs(args: string[]): DriverArgs {
  let allowMismatch = false;
  let progressFrames: number | undefined;
  const paths: string[] = [];
  for (const arg of args) {
    if (arg === "-h" || arg === "--help") {
      printUsage();
      quit(0);
    }
    if (arg === "--allow-mismatch") {
      allowMismatch = true;
      continue;
    }
    if (arg.startsWith("--progress-frames=")) {
      progressFrames = parsePositiveInteger(arg.slice("--progress-frames=".length), "--progress-frames");
      continue;
    }
    if (arg.startsWith("-")) {
      throw new Error(`unknown argument: ${arg}`);
    }
    paths.push(arg);
  }

  if (paths.length < 2 || paths.length > 3) {
    printUsage();
    quit(2);
  }

  const [wasmPath, inputPath, goldenPath = `${inputPath}.md5`] = paths;
  return { allowMismatch, wasmPath, inputPath, goldenPath, progressFrames };
}

function printUsage(): void {
  print(
    "usage: d8 dist/wasm-driver/golden.js -- [--allow-mismatch] [--progress-frames=N] vip9r.wasm input.ivf|input.webm [input.md5]",
  );
}

function parsePositiveInteger(value: string, name: string): number {
  if (!/^[1-9]\d*$/.test(value)) {
    throw new Error(`${name} must be a positive integer`);
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed)) {
    throw new Error(`${name} is too large: ${value}`);
  }
  return parsed;
}

try {
  const d8 = globalThis as D8Global;
  main(d8.scriptArgs ?? d8.arguments ?? []);
} catch (error) {
  print(error instanceof Error && error.stack ? error.stack : String(error));
  quit(1);
}

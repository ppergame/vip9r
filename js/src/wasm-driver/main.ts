import {
  compareWasmToGolden,
  extraCount,
  formatReport,
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
  return { allowMismatch, wasmPath, inputPath, goldenPath };
}

function printUsage(): void {
  print(
    "usage: d8 dist/wasm-driver/golden.js -- [--allow-mismatch] vip9r.wasm input.ivf [input.ivf.md5]",
  );
}

try {
  const d8 = globalThis as D8Global;
  main(d8.scriptArgs ?? d8.arguments ?? []);
} catch (error) {
  print(error instanceof Error && error.stack ? error.stack : String(error));
  quit(1);
}

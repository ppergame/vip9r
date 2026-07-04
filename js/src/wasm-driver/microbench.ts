declare const readbuffer: (path: string) => ArrayBuffer;
declare const print: (...values: unknown[]) => void;
declare const printErr: (...values: unknown[]) => void;
declare const quit: (code?: number) => never;

import { createVip9rMemory, makeVip9rImports } from "./wasm-env";
import type { WasmLog } from "./wasm-env";

export {};

type D8Global = typeof globalThis & {
  arguments?: string[];
  scriptArgs?: string[];
};

type MicrobenchArgs = {
  wasmPath: string;
  slot: number;
};

type TimedMicrobenchPass = {
  targetMs: number;
  elapsedMs: number;
  batches: number;
  innerIters: number;
  totalInnerIters: number;
  nsPerInnerIter: number;
  innerItersPerSecond: number;
  sink: number;
};

type MicrobenchReport = {
  mode: "microbench";
  ok: true;
  slot: number;
  innerIters: number;
  warmupMs: number;
  targetMs: number;
  warmup: TimedMicrobenchPass;
  measurement: TimedMicrobenchPass;
  wasmLogs?: WasmLog[];
};

const DEFAULT_INNER_ITERS = 1000;
const DEFAULT_WARMUP_MS = 1000;
const DEFAULT_TARGET_MS = 5000;

function main(args: string[]): void {
  const parsed = parseArgs(args);
  const logs: WasmLog[] = [];
  const module = new WebAssembly.Module(readbuffer(parsed.wasmPath));
  // Microbench kernels run on small fixed scratch; 64 MiB is plenty.
  const memory = createVip9rMemory(1024);
  const instance = new WebAssembly.Instance(module, makeVip9rImports(memory, (log) => logs.push(log)));
  const bench = benchExport(instance);

  const now = nowMs;
  const warmup = runTimedMicrobench(bench, parsed.slot, DEFAULT_INNER_ITERS, DEFAULT_WARMUP_MS, now, false);
  const measurement = runTimedMicrobench(bench, parsed.slot, DEFAULT_INNER_ITERS, DEFAULT_TARGET_MS, now, true);
  const report: MicrobenchReport = {
    mode: "microbench",
    ok: true,
    slot: parsed.slot,
    innerIters: DEFAULT_INNER_ITERS,
    warmupMs: DEFAULT_WARMUP_MS,
    targetMs: DEFAULT_TARGET_MS,
    warmup,
    measurement,
  };
  if (logs.length !== 0) {
    report.wasmLogs = logs;
  }
  print(JSON.stringify(report));
}

function parseArgs(args: string[]): MicrobenchArgs {
  let slot: number | undefined;
  const paths: string[] = [];

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "-h" || arg === "--help") {
      printUsage();
      quit(0);
    }
    if (arg === "--slot") {
      const value = args[index + 1];
      if (value === undefined) {
        throw new UsageError("--slot requires an integer");
      }
      index += 1;
      slot = parseU32(value, "--slot");
      continue;
    }
    if (arg.startsWith("--slot=")) {
      slot = parseU32(arg.slice("--slot=".length), "--slot");
      continue;
    }
    if (arg.startsWith("-")) {
      throw new UsageError(`unknown argument: ${arg}`);
    }
    paths.push(arg);
  }

  if (paths.length !== 1) {
    throw new UsageError("expected exactly one wasm path");
  }
  if (slot === undefined) {
    throw new UsageError("--slot is required");
  }

  return {
    wasmPath: paths[0],
    slot,
  };
}

function printUsage(out: (...values: unknown[]) => void = print): void {
  out("usage: wasm-microbench --slot N");
}

function printStderr(...values: unknown[]): void {
  if (typeof printErr === "function") {
    printErr(...values);
    return;
  }
  print(...values);
}

function runTimedMicrobench(
  bench: (slot: number, innerIters: number) => number,
  slot: number,
  innerIters: number,
  targetMs: number,
  now: () => number,
  runAtLeastOnce: boolean,
): TimedMicrobenchPass {
  const start = now();
  let elapsedMs = 0;
  let batches = 0;
  let sink = 0;

  while ((runAtLeastOnce && batches === 0) || elapsedMs < targetMs) {
    sink = (sink ^ bench(slot, innerIters)) | 0;
    batches += 1;
    elapsedMs = now() - start;
    if (elapsedMs < 0) {
      throw new Error("microbenchmark clock moved backwards");
    }
  }

  if (batches === 0) {
    elapsedMs = now() - start;
    if (elapsedMs < 0) {
      throw new Error("microbenchmark clock moved backwards");
    }
  }

  const totalInnerIters = batches * innerIters;
  const rate = microbenchRate(totalInnerIters, elapsedMs);
  return {
    targetMs,
    elapsedMs,
    batches,
    innerIters,
    totalInnerIters,
    ...rate,
    sink,
  };
}

function benchExport(instance: WebAssembly.Instance): (slot: number, innerIters: number) => number {
  const bench = instance.exports.vip9r_bench_run;
  if (typeof bench !== "function") {
    throw new Error("missing wasm export: vip9r_bench_run");
  }
  return (slot, innerIters) => {
    const value = bench(slot, innerIters);
    if (typeof value !== "number") {
      throw new Error(`vip9r_bench_run returned ${String(value)}`);
    }
    return value | 0;
  };
}

function microbenchRate(
  totalInnerIters: number,
  elapsedMs: number,
): { nsPerInnerIter: number; innerItersPerSecond: number } {
  if (totalInnerIters === 0 || elapsedMs <= 0) {
    return { nsPerInnerIter: 0, innerItersPerSecond: 0 };
  }
  return {
    nsPerInnerIter: (elapsedMs * 1_000_000) / totalInnerIters,
    innerItersPerSecond: (totalInnerIters * 1000) / elapsedMs,
  };
}

function parseU32(value: string, name: string): number {
  const parsed = parseNonNegativeInteger(value, name);
  if (parsed > 0xffffffff) {
    throw new UsageError(`${name} must fit in u32`);
  }
  return parsed;
}

function parseNonNegativeInteger(value: string, name: string): number {
  if (!/^(0|[1-9]\d*)$/.test(value)) {
    throw new UsageError(`${name} must be a non-negative integer`);
  }
  return parseSafeInteger(value, name);
}

function parseSafeInteger(value: string, name: string): number {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed)) {
    throw new UsageError(`${name} is too large: ${value}`);
  }
  return parsed;
}

function nowMs(): number {
  if (typeof performance === "object" && typeof performance.now === "function") {
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

class UsageError extends Error {}

try {
  const d8 = globalThis as D8Global;
  main(d8.scriptArgs ?? d8.arguments ?? []);
} catch (error) {
  if (error instanceof UsageError) {
    printUsage(printStderr);
    print(JSON.stringify({ mode: "microbench", ok: false, error: error.message }));
    quit(2);
  }
  print(JSON.stringify({ mode: "microbench", ok: false, error: errorMessage(error) }));
  quit(1);
}

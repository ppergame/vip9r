declare const readbuffer: (path: string) => ArrayBuffer;
declare const print: (...values: unknown[]) => void;
declare const quit: (code?: number) => never;

import { createVip9rMemory, formatWasmLog, makeVip9rImports, WasmLogKind } from "../wasm-driver/wasm-env";
import type { WasmLog } from "../wasm-driver/wasm-env";
import { spawnWorkerPool } from "../wasm-driver/pool";
import type { WorkerPool } from "../wasm-driver/pool";

export {};

type D8Global = typeof globalThis & {
  arguments?: string[];
  scriptArgs?: string[];
};

type RunnerArgs = {
  wasmPath: string;
  json: boolean;
  testFilter?: string;
};

type TestResult =
  | { kind: "pass"; logs: WasmLog[] }
  | { kind: "fail"; message: string; logs: WasmLog[] };

type TestFailureReport = {
  name: string;
  message: string;
  logs: WasmLog[];
};

type TestReport = {
  mode: "tests";
  ok: boolean;
  filter?: string;
  discovered: number;
  selected: number;
  passed: number;
  failed: number;
  filtered: number;
  failures: TestFailureReport[];
  error?: string;
};

const TEST_PREFIX = "vip9r_test__";
const TEST_FAILURE = 1;
// Tests under the Rust pool module run against live workers; the runner owns
// spawn and teardown so the wasm side stays a pure protocol.
const POOL_TEST_MARKER = "::pool::";

function main(args: string[]): void {
  const { wasmPath, json, testFilter } = parseArgs(args);
  const module = new WebAssembly.Module(readbuffer(wasmPath));
  const testNames = discoverTests(module);
  const selectedTestNames = selectTests(testNames, testFilter);

  if (testFilter !== undefined) {
    const filtered = testNames.length - selectedTestNames.length;
    const filterLabel = JSON.stringify(testFilter);
    if (selectedTestNames.length === 0) {
      const report = emptyFilteredReport(testFilter, testNames.length);
      if (json) {
        print(JSON.stringify(report));
      } else {
        print(`no tests matched substring ${filterLabel} (${testNames.length} discovered)`);
      }
      quit(1);
    }
    if (!json) {
      print(
        `running ${selectedTestNames.length} of ${testNames.length} tests matching ${filterLabel} (${filtered} filtered out)`,
      );
    }
  }

  let passed = 0;
  let failed = 0;
  const failures: TestFailureReport[] = [];
  for (const testName of selectedTestNames) {
    const result = runTest(module, testName);
    if (result.kind === "pass") {
      passed += 1;
      if (!json) {
        printDiagnosticLogs(testName, result.logs);
        print(`test ${testName} ... ok`);
      }
    } else {
      failed += 1;
      failures.push({ name: testName, message: result.message, logs: result.logs });
      if (!json) {
        printDiagnosticLogs(testName, result.logs);
        print(`test ${testName} ... FAILED: ${result.message}`);
      }
    }
  }

  const report: TestReport = {
    mode: "tests",
    ok: failed === 0,
    discovered: testNames.length,
    selected: selectedTestNames.length,
    passed,
    failed,
    filtered: testNames.length - selectedTestNames.length,
    failures,
  };
  if (testFilter !== undefined) {
    report.filter = testFilter;
  }

  if (json) {
    print(JSON.stringify(report));
    if (!report.ok) {
      quit(1);
    }
    return;
  }

  if (failed === 0) {
    print(formatResult("ok", passed, failed, testNames.length - selectedTestNames.length));
    return;
  }

  print(formatResult("FAILED", passed, failed, testNames.length - selectedTestNames.length));
  quit(1);
}

function parseArgs(args: string[]): RunnerArgs {
  let json = false;
  const positional: string[] = [];
  for (const arg of args) {
    if (arg === "-h" || arg === "--help") {
      printUsage();
      quit(0);
    }
    if (arg === "--json") {
      json = true;
      continue;
    }
    if (arg.startsWith("-")) {
      throw new UsageError(`unknown argument: ${arg}`);
    }
    positional.push(arg);
  }

  if (positional.length < 1 || positional.length > 2) {
    throw new UsageError("expected wasm path and optional test substring");
  }
  const [wasmPath, testFilter] = positional;
  const runnerArgs: RunnerArgs = { wasmPath, json };
  if (testFilter !== undefined) {
    runnerArgs.testFilter = testFilter;
  }
  return runnerArgs;
}

function printUsage(): void {
  print("usage: wasm-tests [--json] [TEST_SUBSTRING]");
}

function discoverTests(module: WebAssembly.Module): string[] {
  const tests: string[] = [];
  for (const descriptor of WebAssembly.Module.exports(module)) {
    if (descriptor.kind === "function" && descriptor.name.startsWith(TEST_PREFIX)) {
      tests.push(descriptor.name);
    }
  }
  return tests;
}

function selectTests(testNames: string[], testFilter: string | undefined): string[] {
  if (testFilter === undefined) {
    return testNames;
  }
  return testNames.filter((testName) => testName.includes(testFilter));
}

function formatResult(status: "ok" | "FAILED", passed: number, failed: number, filtered: number): string {
  const base = `result: ${status}. ${passed} passed; ${failed} failed`;
  if (filtered === 0) {
    return base;
  }
  return `${base}; ${filtered} filtered out`;
}

function emptyFilteredReport(testFilter: string, discovered: number): TestReport {
  return {
    mode: "tests",
    ok: false,
    filter: testFilter,
    discovered,
    selected: 0,
    passed: 0,
    failed: 0,
    filtered: discovered,
    failures: [],
    error: `no tests matched substring ${JSON.stringify(testFilter)} (${discovered} discovered)`,
  };
}

function runTest(module: WebAssembly.Module, testName: string): TestResult {
  const logs: WasmLog[] = [];
  let pool: WorkerPool | undefined;
  try {
    // Unit tests init small decoder shapes; 64 MiB is plenty, and each test
    // gets a fresh memory alongside its fresh instance.
    const memory = createVip9rMemory(1024);
    const instance = new WebAssembly.Instance(module, makeVip9rImports(memory, (log) => logs.push(log)));
    const test = instance.exports[testName];
    if (typeof test !== "function") {
      return fail("export is not callable", logs);
    }
    if (testName.includes(POOL_TEST_MARKER)) {
      pool = spawnWorkerPool(module, memory);
    }

    const value = test();
    if (value === undefined || value === 0) {
      return { kind: "pass", logs };
    }
    if (value === TEST_FAILURE) {
      return fail("failed", logs);
    }
    if (typeof value === "number") {
      return fail(`returned ${value}`, logs);
    }
    return fail(`returned ${String(value)}`, logs);
  } catch (error) {
    return fail(errorMessage(error), logs);
  } finally {
    pool?.terminate();
  }
}

function fail(fallback: string, logs: WasmLog[]): TestResult {
  return { kind: "fail", message: loggedFailureMessage(logs) ?? fallback, logs };
}

function loggedFailureMessage(logs: WasmLog[]): string | undefined {
  for (let index = logs.length - 1; index >= 0; index -= 1) {
    const log = logs[index];
    if (log.kind === WasmLogKind.Panic || log.kind === WasmLogKind.TestFailure) {
      return formatWasmLog(log);
    }
  }
  return undefined;
}

function printDiagnosticLogs(testName: string, logs: WasmLog[]): void {
  for (const log of logs) {
    if (log.kind === WasmLogKind.Panic || log.kind === WasmLogKind.TestFailure) {
      continue;
    }
    print(`test ${testName} ${formatWasmLog(log)}`);
  }
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
  const args = d8.scriptArgs ?? d8.arguments ?? [];
  main(args);
} catch (error) {
  const d8 = globalThis as D8Global;
  const args = d8.scriptArgs ?? d8.arguments ?? [];
  const json = args.includes("--json");
  if (error instanceof UsageError) {
    if (json) {
      print(JSON.stringify({ mode: "tests", ok: false, error: error.message }));
    } else {
      printUsage();
      print(error.message);
    }
    quit(2);
  }
  if (json) {
    print(JSON.stringify({ mode: "tests", ok: false, error: errorMessage(error) }));
  } else {
    print(errorMessage(error));
  }
  quit(1);
}

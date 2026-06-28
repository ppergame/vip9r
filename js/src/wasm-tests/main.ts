declare const readbuffer: (path: string) => ArrayBuffer;
declare const print: (...values: unknown[]) => void;
declare const quit: (code?: number) => never;

const WasmLogKind = {
  Diagnostic: 0,
  TestFailure: 1,
  Panic: 2,
} as const;

type WasmLog = {
  kind: number;
  message: string;
};

type WasmLogSink = (log: WasmLog) => void;

type D8Global = typeof globalThis & {
  arguments?: string[];
  scriptArgs?: string[];
};

type RunnerArgs = {
  wasmPath: string;
  testFilter?: string;
};

type TestResult =
  | { kind: "pass"; logs: WasmLog[] }
  | { kind: "fail"; message: string; logs: WasmLog[] };

const TEST_PREFIX = "vip9r_test__";
const TEST_FAILURE = 1;

function main(args: string[]): void {
  const { wasmPath, testFilter } = parseArgs(args);
  const module = new WebAssembly.Module(readbuffer(wasmPath));
  const testNames = discoverTests(module);
  const selectedTestNames = selectTests(testNames, testFilter);

  if (testFilter !== undefined) {
    const filtered = testNames.length - selectedTestNames.length;
    const filterLabel = JSON.stringify(testFilter);
    if (selectedTestNames.length === 0) {
      print(`no tests matched substring ${filterLabel} (${testNames.length} discovered)`);
      quit(1);
    }
    print(
      `running ${selectedTestNames.length} of ${testNames.length} tests matching ${filterLabel} (${filtered} filtered out)`,
    );
  }

  let passed = 0;
  let failed = 0;
  for (const testName of selectedTestNames) {
    const result = runTest(module, testName);
    printDiagnosticLogs(testName, result.logs);
    if (result.kind === "pass") {
      passed += 1;
      print(`test ${testName} ... ok`);
    } else {
      failed += 1;
      print(`test ${testName} ... FAILED: ${result.message}`);
    }
  }

  if (failed === 0) {
    print(formatResult("ok", passed, failed, testNames.length - selectedTestNames.length));
    return;
  }

  print(formatResult("FAILED", passed, failed, testNames.length - selectedTestNames.length));
  quit(1);
}

function parseArgs(args: string[]): RunnerArgs {
  if (args.length === 1 && (args[0] === "-h" || args[0] === "--help")) {
    printUsage();
    quit(0);
  }
  if (args.length < 1 || args.length > 2) {
    printUsage();
    throw new UsageError("expected wasm path and optional test substring");
  }
  const [wasmPath, testFilter] = args;
  if (wasmPath.startsWith("-")) {
    printUsage();
    throw new UsageError(`unknown argument: ${wasmPath}`);
  }
  const runnerArgs: RunnerArgs = { wasmPath };
  if (testFilter !== undefined) {
    runnerArgs.testFilter = testFilter;
  }
  return runnerArgs;
}

function printUsage(): void {
  print("usage: d8 dist/wasm-driver/tests.js -- vip9r.wasm [TEST_SUBSTRING]");
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

function runTest(module: WebAssembly.Module, testName: string): TestResult {
  const logs: WasmLog[] = [];
  try {
    let instance: WebAssembly.Instance | undefined;
    const imports = makeVip9rImports(() => {
      if (instance === undefined) {
        throw new Error("vip9r_log called before wasm instance was assigned");
      }
      return instanceMemory(instance);
    }, (log) => logs.push(log));
    instance = new WebAssembly.Instance(module, imports);
    const test = instance.exports[testName];
    if (typeof test !== "function") {
      return fail("export is not callable", logs);
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

const utf8Decoder = typeof TextDecoder === "function" ? new TextDecoder("utf-8") : undefined;

function makeVip9rImports(
  getMemory: () => WebAssembly.Memory,
  sink: WasmLogSink,
): WebAssembly.Imports {
  return {
    env: {
      vip9r_log(kind: number, ptr: number, len: number): void {
        const memory = getMemory();
        if (!Number.isInteger(ptr) || !Number.isInteger(len) || ptr < 0 || len < 0) {
          throw new Error(`invalid wasm log span: ptr=${ptr} len=${len}`);
        }
        if (ptr + len > memory.buffer.byteLength) {
          throw new Error(`wasm log span out of bounds: ptr=${ptr} len=${len}`);
        }
        const bytes = new Uint8Array(memory.buffer, ptr, len);
        sink({ kind, message: decodeUtf8(bytes) });
      },
    },
  };
}

function instanceMemory(instance: WebAssembly.Instance): WebAssembly.Memory {
  const memory = instance.exports.memory;
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("missing wasm export: memory");
  }
  return memory;
}

function formatWasmLog(log: WasmLog): string {
  switch (log.kind) {
    case WasmLogKind.Diagnostic:
      return `wasm diag: ${log.message}`;
    case WasmLogKind.TestFailure:
      return `wasm test failure: ${log.message}`;
    case WasmLogKind.Panic:
      return `wasm panic: ${log.message}`;
    default:
      return `wasm log ${log.kind}: ${log.message}`;
  }
}

function decodeUtf8(bytes: Uint8Array): string {
  if (utf8Decoder !== undefined) {
    return utf8Decoder.decode(bytes);
  }

  let text = "";
  for (const byte of bytes) {
    text += String.fromCharCode(byte);
  }
  return text;
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
    print(error.message);
    quit(2);
  }
  print(error instanceof Error && error.stack ? error.stack : String(error));
  quit(1);
}

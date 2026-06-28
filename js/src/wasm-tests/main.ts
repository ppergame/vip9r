declare const readbuffer: (path: string) => ArrayBuffer;
declare const print: (...values: unknown[]) => void;
declare const quit: (code?: number) => never;

type D8Global = typeof globalThis & {
  arguments?: string[];
  scriptArgs?: string[];
};

type RunnerArgs = {
  wasmPath: string;
  testFilter?: string;
};

type TestResult =
  | { kind: "pass" }
  | { kind: "fail"; message: string };

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
  try {
    const instance = new WebAssembly.Instance(module, {});
    const test = instance.exports[testName];
    if (typeof test !== "function") {
      return { kind: "fail", message: "export is not callable" };
    }

    const value = test();
    if (value === undefined || value === 0) {
      return { kind: "pass" };
    }
    if (value === TEST_FAILURE) {
      return { kind: "fail", message: "failed" };
    }
    if (typeof value === "number") {
      return { kind: "fail", message: `returned ${value}` };
    }
    return { kind: "fail", message: `returned ${String(value)}` };
  } catch (error) {
    return { kind: "fail", message: errorMessage(error) };
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
  main(d8.scriptArgs ?? d8.arguments ?? []);
} catch (error) {
  if (error instanceof UsageError) {
    print(error.message);
    quit(2);
  }
  print(error instanceof Error && error.stack ? error.stack : String(error));
  quit(1);
}

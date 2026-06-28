declare const readbuffer: (path: string) => ArrayBuffer;
declare const print: (...values: unknown[]) => void;
declare const quit: (code?: number) => never;

type D8Global = typeof globalThis & {
  arguments?: string[];
  scriptArgs?: string[];
};

type RunnerArgs = {
  wasmPath: string;
};

type TestResult =
  | { kind: "pass" }
  | { kind: "fail"; message: string };

const TEST_PREFIX = "vip9r_test__";
const TEST_FAILURE = 1;

function main(args: string[]): void {
  const { wasmPath } = parseArgs(args);
  const module = new WebAssembly.Module(readbuffer(wasmPath));
  const testNames = discoverTests(module);

  let passed = 0;
  let failed = 0;
  for (const testName of testNames) {
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
    print(`result: ok. ${passed} passed; 0 failed`);
    return;
  }

  print(`result: FAILED. ${passed} passed; ${failed} failed`);
  quit(1);
}

function parseArgs(args: string[]): RunnerArgs {
  if (args.length === 1 && (args[0] === "-h" || args[0] === "--help")) {
    printUsage();
    quit(0);
  }
  if (args.length !== 1) {
    printUsage();
    throw new UsageError("expected wasm path");
  }
  const [wasmPath] = args;
  if (wasmPath.startsWith("-")) {
    printUsage();
    throw new UsageError(`unknown argument: ${wasmPath}`);
  }
  return { wasmPath };
}

function printUsage(): void {
  print("usage: d8 dist/wasm-driver/tests.js -- vip9r.wasm");
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

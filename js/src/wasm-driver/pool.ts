// d8 worker pool for the threaded module: three workers over the shared
// memory, each rebinding its fixed shadow-stack top before any wasm runs
// (layout and protocol in docs/design.md, Threads). d8-only: uses d8's
// string-source Worker; the web frontend ships its own worker file.

import {
  COORDINATOR_STACK_TOP,
  MIN_HEAP_BASE,
  WORKER_STACK_TOPS,
} from "./stack-layout";

type D8Worker = {
  postMessage(value: unknown): void;
  terminate(): void;
};
type D8WorkerConstructor = new (
  source: string,
  options: { type: "string" },
) => D8Worker;

// Reached through globalThis because lib.webworker declares a Worker with a
// different constructor shape.
const D8WorkerCtor = (globalThis as Record<string, unknown>)
  .Worker as D8WorkerConstructor;

// Runs in a fresh d8 isolate: self-contained, no module imports, so the
// stack-layout ABI constants are interpolated in. The two asserts are pure
// data reads on the not-yet-rebound instance: stack pointer still at the
// coordinator top (catches -zstack-size drift) and data pushed above the
// worker stack region (catches a dropped --global-base). vip9r_worker_main
// never returns, so onmessage never completes; teardown is terminate(), safe
// even while the worker is parked in a wait.
const WORKER_SOURCE = `
onmessage = function({ data }) {
  const { module, memory, workerIndex } = data;
  const instance = new WebAssembly.Instance(module, {
    env: {
      memory,
      vip9r_log(kind, ptr, len) {
        const bytes = new Uint8Array(memory.buffer, ptr, len);
        let text = "";
        for (let i = 0; i < len; i += 1) text += String.fromCharCode(bytes[i]);
        print("worker " + workerIndex + " wasm log " + kind + ": " + text);
      },
    },
  });
  const stackPointer = instance.exports.__stack_pointer;
  if (stackPointer.value !== ${COORDINATOR_STACK_TOP}) {
    throw new Error("fresh instance stack pointer " + stackPointer.value +
      " != ${COORDINATOR_STACK_TOP} (-zstack-size drift?)");
  }
  if (instance.exports.__heap_base.value < ${MIN_HEAP_BASE}) {
    throw new Error("__heap_base " + instance.exports.__heap_base.value +
      " below the worker stack region (--global-base missing?)");
  }
  stackPointer.value = [${WORKER_STACK_TOPS.join(", ")}][workerIndex];
  instance.exports.vip9r_worker_main(workerIndex);
};
`;

export type WorkerPool = {
  terminate(): void;
};

// The coordinator instance must be instantiated to completion before this is
// called, so worker instances hit the __wasm_init_memory flag=2 skip path
// instead of blocking in its once-guard. After spawning, the caller activates
// the pool via the coordinator's vip9r_pool_activate export.
export function spawnWorkerPool(
  module: WebAssembly.Module,
  memory: WebAssembly.Memory,
): WorkerPool {
  const workers = WORKER_STACK_TOPS.map((_stackTop, workerIndex) => {
    const worker = new D8WorkerCtor(WORKER_SOURCE, { type: "string" });
    worker.postMessage({ module, memory, workerIndex });
    return worker;
  });
  return {
    terminate() {
      for (const worker of workers) {
        worker.terminate();
      }
    },
  };
}

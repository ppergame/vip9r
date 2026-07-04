// d8 worker pool for the threaded module: three workers over the shared
// memory, each rebinding its fixed shadow-stack top before any wasm runs
// (layout and protocol in docs/design.md, Threads). d8-only: uses d8's
// string-source Worker; the web frontend will ship its own worker file.

// ABI constants of the fixed-address stack layout (rust/.cargo/config.toml).
const COORDINATOR_STACK_TOP = 0x100000;
const WORKER_STACK_TOPS = [0x200000, 0x300000, 0x400000];
const MIN_HEAP_BASE = 0x400000;

type D8Worker = {
  postMessage(value: unknown): void;
  terminate(): void;
};
type D8WorkerConstructor = new (source: string, options: { type: "string" }) => D8Worker;

// Reached through globalThis because lib.webworker declares a Worker with a
// different constructor shape.
const D8WorkerCtor = (globalThis as Record<string, unknown>).Worker as D8WorkerConstructor;

// Runs in a fresh d8 isolate: self-contained, no module imports. The two
// asserts are pure data reads on the not-yet-rebound instance: stack pointer
// still at the coordinator top (catches -zstack-size drift) and data pushed
// above the worker stack region (catches a dropped --global-base).
// vip9r_worker_main never returns, so onmessage never completes; teardown is
// terminate(), safe even while the worker is parked in a wait.
const WORKER_SOURCE = `
onmessage = function({ data }) {
  const { module, memory, workerIndex, stackTop, coordinatorStackTop, minHeapBase } = data;
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
  if (stackPointer.value !== coordinatorStackTop) {
    throw new Error("fresh instance stack pointer " + stackPointer.value +
      " != " + coordinatorStackTop + " (-zstack-size drift?)");
  }
  if (instance.exports.__heap_base.value < minHeapBase) {
    throw new Error("__heap_base " + instance.exports.__heap_base.value +
      " below the worker stack region (--global-base missing?)");
  }
  stackPointer.value = stackTop;
  instance.exports.vip9r_worker_main(workerIndex);
};
`;

export type WorkerPool = {
  terminate(): void;
};

// The coordinator instance must be instantiated to completion before this is
// called, so worker instances hit the __wasm_init_memory flag=2 skip path
// instead of blocking in its once-guard.
export function spawnWorkerPool(
  module: WebAssembly.Module,
  memory: WebAssembly.Memory,
): WorkerPool {
  const workers = WORKER_STACK_TOPS.map((stackTop, workerIndex) => {
    const worker = new D8WorkerCtor(WORKER_SOURCE, { type: "string" });
    worker.postMessage({
      module,
      memory,
      workerIndex,
      stackTop,
      coordinatorStackTop: COORDINATOR_STACK_TOP,
      minHeapBase: MIN_HEAP_BASE,
    });
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

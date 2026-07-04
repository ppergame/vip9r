// Web worker pool for the threaded module: three workers spawned from the
// decode worker (nested workers) over the shared memory, each rebinding its
// fixed shadow-stack top before any wasm runs (layout and protocol in
// docs/design.md, Threads).

import { WORKER_STACK_TOPS } from "../wasm-driver/stack-layout";

export type PoolWorkerInit = {
  module: WebAssembly.Module;
  memory: WebAssembly.Memory;
  workerIndex: number;
};

export type WorkerPool = {
  terminate(): void;
};

// The coordinator instance must be instantiated to completion before this is
// called, so worker instances hit the __wasm_init_memory flag=2 skip path
// instead of blocking in its once-guard. After spawning, the caller activates
// the pool via the coordinator's vip9r_pool_activate export. The workers die
// with their spawning worker; terminate() exists for callers that outlive a
// pool.
export function spawnWorkerPool(
  module: WebAssembly.Module,
  memory: WebAssembly.Memory,
  onError: (message: string) => void,
): WorkerPool {
  const workers = WORKER_STACK_TOPS.map((_stackTop, workerIndex) => {
    const worker = new Worker(new URL("./pool-worker.ts", import.meta.url), {
      type: "module",
    });
    worker.onerror = (event) => onError(`pool worker ${workerIndex}: ${event.message}`);
    const init: PoolWorkerInit = { module, memory, workerIndex };
    worker.postMessage(init);
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

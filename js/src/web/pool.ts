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

export type PoolWorkerEvent = {
  type: "ready";
  workerIndex: number;
};

export type WorkerPool = {
  ready: Promise<void>;
  terminate(): void;
};

// Spawn the pool, wait for every worker to park in vip9r_worker_main, then
// activate tile-parallel dispatch on the coordinator instance. The await is
// load-bearing: once activated, the coordinator blocks in a wasm atomic wait
// during multi-tile frames, and that starves nested-worker startup — spawned
// worker scripts never even evaluate while their parent is blocked (probed
// on Chrome). The workers die with their spawning worker; terminate() exists
// for callers that outlive a pool.
export async function activateWorkerPool(
  instance: WebAssembly.Instance,
  module: WebAssembly.Module,
  memory: WebAssembly.Memory,
  onError: (message: string) => void,
): Promise<WorkerPool> {
  if (!sharedMemoryTransferable(memory)) {
    onError(
      "worker pool disabled: shared memory transfer blocked; decoding single-threaded",
    );
    return { ready: Promise.resolve(), terminate() {} };
  }
  const pool = spawnWorkerPool(module, memory, onError);
  await pool.ready;
  const activate = instance.exports.vip9r_pool_activate;
  if (typeof activate !== "function") {
    throw new Error("missing wasm export: vip9r_pool_activate");
  }
  activate();
  return pool;
}

// Whether shared memory survives postMessage cannot be predicted from
// self.crossOriginIsolated: Cobalt launched with
// --enable-blink-features=SharedArrayBuffer transfers fine while
// crossOriginIsolated stays false. Probe the actual serializer gate with a
// throwaway channel — same Blink check the worker postMessage would hit.
function sharedMemoryTransferable(memory: WebAssembly.Memory): boolean {
  const channel = new MessageChannel();
  try {
    channel.port1.postMessage(memory);
    return true;
  } catch {
    return false;
  } finally {
    channel.port1.close();
    channel.port2.close();
  }
}

// The coordinator instance must be instantiated to completion before this is
// called, so worker instances hit the __wasm_init_memory flag=2 skip path
// instead of blocking in its once-guard.
function spawnWorkerPool(
  module: WebAssembly.Module,
  memory: WebAssembly.Memory,
  onError: (message: string) => void,
): WorkerPool {
  const ready: Promise<void>[] = [];
  const workers = WORKER_STACK_TOPS.map((_stackTop, workerIndex) => {
    const worker = new Worker(new URL("./pool-worker.ts", import.meta.url), {
      type: "module",
    });
    ready.push(
      new Promise<void>((resolve, reject) => {
        worker.onmessage = (event: MessageEvent<PoolWorkerEvent>) => {
          if (event.data.type === "ready") {
            resolve();
          }
        };
        worker.onerror = (event) => {
          const message = `pool worker ${workerIndex}: ${event.message}`;
          onError(message);
          reject(new Error(message));
        };
      }),
    );
    const init: PoolWorkerInit = { module, memory, workerIndex };
    worker.postMessage(init);
    return worker;
  });
  return {
    ready: Promise.all(ready).then(() => undefined),
    terminate() {
      for (const worker of workers) {
        worker.terminate();
      }
    },
  };
}

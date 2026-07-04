// Pool worker body (web counterpart of the d8 string-source worker in
// wasm-driver/pool.ts): instantiate the threaded module over the shared
// memory, check the fixed-address stack layout, rebind this worker's shadow
// stack, and park in vip9r_worker_main. The entry never returns, so onmessage
// never completes; teardown is Worker.terminate(), safe mid-wait. Thrown
// errors (including the layout asserts) surface on the spawner's onerror.

import { COORDINATOR_STACK_TOP, MIN_HEAP_BASE, WORKER_STACK_TOPS } from "../wasm-driver/stack-layout";
import { formatWasmLog, makeVip9rImports } from "../wasm-driver/wasm-env";
import type { PoolWorkerInit } from "./pool";

self.onmessage = (event: MessageEvent<PoolWorkerInit>) => {
  const { module, memory, workerIndex } = event.data;
  // Sync instantiation: allowed off the main thread, and it keeps assert
  // failures synchronous in onmessage so they reach the spawner's onerror.
  const instance = new WebAssembly.Instance(
    module,
    makeVip9rImports(memory, (log) => console.error(`pool worker ${workerIndex} ${formatWasmLog(log)}`)),
  );
  const stackPointer = instance.exports.__stack_pointer as WebAssembly.Global;
  if (stackPointer.value !== COORDINATOR_STACK_TOP) {
    throw new Error(
      `fresh instance stack pointer ${stackPointer.value} != ${COORDINATOR_STACK_TOP} (-zstack-size drift?)`,
    );
  }
  const heapBase = instance.exports.__heap_base as WebAssembly.Global;
  if (heapBase.value < MIN_HEAP_BASE) {
    throw new Error(`__heap_base ${heapBase.value} below the worker stack region (--global-base missing?)`);
  }
  stackPointer.value = WORKER_STACK_TOPS[workerIndex];
  (instance.exports.vip9r_worker_main as (workerIndex: number) => never)(workerIndex);
};

import wasmUrl from "../../../rust/target/wasm32-unknown-unknown/release/vip9r.wasm?url";
import {
  createVip9rMemory,
  formatWasmLog,
  makeVip9rImports,
  type WasmLog,
} from "../wasm-driver/wasm-env";

export { wasmUrl };

export async function instantiateVip9r(
  onLog: (message: string) => void,
): Promise<{
  instance: WebAssembly.Instance;
  module: WebAssembly.Module;
  memory: WebAssembly.Memory;
}> {
  const sink = (entry: WasmLog) => onLog(formatWasmLog(entry));
  const module = await WebAssembly.compileStreaming(fetch(wasmUrl));
  const memory = createVip9rMemory();
  // Async instantiation: Chrome rejects sync instantiation of large modules
  // on the main thread.
  const instance = await WebAssembly.instantiate(
    module,
    makeVip9rImports(memory, sink),
  );
  return { instance, module, memory };
}

import wasmUrl from "../../../rust/target/wasm32-unknown-unknown/release/vip9r.wasm?url";
import {
  createScratchVip9rMemory,
  createVip9rMemory,
  formatWasmLog,
  makeVip9rImports,
  sessionMaxPages,
  type WasmLog,
} from "../wasm-driver/wasm-env";

export { wasmUrl };

export async function instantiateVip9r(
  dims: { width: number; height: number },
  onLog: (message: string) => void,
): Promise<{
  instance: WebAssembly.Instance;
  module: WebAssembly.Module;
  memory: WebAssembly.Memory;
}> {
  const sink = (entry: WasmLog) => onLog(formatWasmLog(entry));
  const module = await WebAssembly.compileStreaming(fetch(wasmUrl));
  // Async instantiation both times: Chrome rejects sync instantiation of
  // large modules on the main thread.
  const scratch = await WebAssembly.instantiate(
    module,
    makeVip9rImports(createScratchVip9rMemory(), sink),
  );
  const memory = createVip9rMemory(sessionMaxPages(scratch.exports, dims));
  const instance = await WebAssembly.instantiate(module, makeVip9rImports(memory, sink));
  return { instance, module, memory };
}

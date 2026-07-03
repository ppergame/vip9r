import wasmUrl from "../../../rust/target/wasm32-unknown-unknown/release/vip9r.wasm?url";
import { formatWasmLog, instanceMemory, makeVip9rImports } from "../wasm-driver/wasm-env";

export { wasmUrl };

export async function instantiateVip9r(
  onLog: (message: string) => void,
): Promise<{ instance: WebAssembly.Instance; memory: WebAssembly.Memory }> {
  let memory: WebAssembly.Memory | undefined;
  const imports = makeVip9rImports(
    () => {
      if (memory === undefined) {
        throw new Error("wasm logged before instantiation completed");
      }
      return memory;
    },
    (entry) => onLog(formatWasmLog(entry)),
  );
  const { instance } = await WebAssembly.instantiateStreaming(fetch(wasmUrl), imports);
  memory = instanceMemory(instance);
  return { instance, memory };
}

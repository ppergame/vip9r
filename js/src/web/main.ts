import wasmUrl from "../../../rust/target/wasm32-unknown-unknown/release/vip9r.wasm?url";
import { Vp9Decoder } from "../wasm";
import { formatWasmLog, instanceMemory, makeVip9rImports } from "../wasm-driver/wasm-env";

const app = document.querySelector<HTMLDivElement>("#app")!;

function line(text: string): void {
  const div = document.createElement("div");
  div.textContent = text;
  app.append(div);
}

async function main(): Promise<void> {
  let memory: WebAssembly.Memory | undefined;
  const imports = makeVip9rImports(
    () => {
      if (memory === undefined) {
        throw new Error("wasm logged before instantiation completed");
      }
      return memory;
    },
    (log) => line(formatWasmLog(log)),
  );
  const { instance } = await WebAssembly.instantiateStreaming(fetch(wasmUrl), imports);
  memory = instanceMemory(instance);
  new Vp9Decoder(instance, 1280, 720);
  line(`wasm ok: ${wasmUrl}`);

  const mediaResponse = await fetch("/media/chromium/bear-vp9.ivf");
  if (!mediaResponse.ok) {
    throw new Error(`media fetch failed: ${mediaResponse.status}`);
  }
  const media = await mediaResponse.arrayBuffer();
  line(`media ok: bear-vp9.ivf, ${media.byteLength} bytes`);
}

main().catch((error) => {
  line(String(error));
});

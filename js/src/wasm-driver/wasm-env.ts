export const WasmLogKind = {
  Diagnostic: 0,
  TestFailure: 1,
  Panic: 2,
} as const;

export type WasmLog = {
  kind: number;
  message: string;
};

export type WasmLogSink = (log: WasmLog) => void;

const utf8Decoder = typeof TextDecoder === "function" ? new TextDecoder("utf-8") : undefined;

export function makeVip9rImports(
  getMemory: () => WebAssembly.Memory,
  sink: WasmLogSink,
): WebAssembly.Imports {
  return {
    env: {
      vip9r_log(kind: number, ptr: number, len: number): void {
        const memory = getMemory();
        if (!Number.isInteger(ptr) || !Number.isInteger(len) || ptr < 0 || len < 0) {
          throw new Error(`invalid wasm log span: ptr=${ptr} len=${len}`);
        }
        if (ptr + len > memory.buffer.byteLength) {
          throw new Error(`wasm log span out of bounds: ptr=${ptr} len=${len}`);
        }
        const bytes = new Uint8Array(memory.buffer, ptr, len);
        sink({ kind, message: decodeUtf8(bytes) });
      },
    },
  };
}

export function instanceMemory(instance: WebAssembly.Instance): WebAssembly.Memory {
  const memory = instance.exports.memory;
  if (!(memory instanceof WebAssembly.Memory)) {
    throw new Error("missing wasm export: memory");
  }
  return memory;
}

export function formatWasmLog(log: WasmLog): string {
  switch (log.kind) {
    case WasmLogKind.Diagnostic:
      return `wasm diag: ${log.message}`;
    case WasmLogKind.TestFailure:
      return `wasm test failure: ${log.message}`;
    case WasmLogKind.Panic:
      return `wasm panic: ${log.message}`;
    default:
      return `wasm log ${log.kind}: ${log.message}`;
  }
}

function decodeUtf8(bytes: Uint8Array): string {
  if (utf8Decoder !== undefined) {
    return utf8Decoder.decode(bytes);
  }

  let text = "";
  for (const byte of bytes) {
    text += String.fromCharCode(byte);
  }
  return text;
}

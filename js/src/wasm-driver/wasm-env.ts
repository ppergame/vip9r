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

const utf8Decoder =
  typeof TextDecoder === "function" ? new TextDecoder("utf-8") : undefined;

// Fixed session memory, matching the linker's --initial-memory ==
// --max-memory (rust/.cargo/config.toml). The declared min == max limits are
// what let V8 cache the shared-memory size, and they match only a memory of
// exactly this shape — drift between this constant and the linker value fails
// every instantiation loudly. The wasm side still sizes itself dynamically;
// 1024 pages is policy, pinned by the corpus_ceiling_fits_fixed_memory wasm
// test: the 1080p ceiling needs 541 pages, plus an 8 MiB packet-tail budget
// (anchors: largest 1080p corpus packet 504 KiB, bbb_1920x1080 tile_1x4; VP9
// level 4.1 CPB cap 3 MiB; lossless keyframe near raw 4:2:0, 3.1 MiB — a
// multi-frame lossless superframe is the accepted fail-loud RESOURCE_LIMIT
// case, hit when reserve_input can't grow the non-growable memory). V8
// reserves the full 64 MiB upfront for shared memories, but physical pages
// materialize on first touch, so untouched headroom is address space only.
// A few libvpx resize vectors exceed the 1080p shape; the d8 golden driver
// runs those over a limits-patched module (expanded-memory.ts) by passing an
// expanded page count here. Web frontends always use the default.
export const VIP9R_MEMORY_PAGES = 1024;

export function createVip9rMemory(
  pages: number = VIP9R_MEMORY_PAGES,
): WebAssembly.Memory {
  return new WebAssembly.Memory({
    initial: pages,
    maximum: pages,
    shared: true,
  });
}

export function makeVip9rImports(
  memory: WebAssembly.Memory,
  sink: WasmLogSink,
): WebAssembly.Imports {
  return {
    env: {
      memory,
      vip9r_log(kind: number, ptr: number, len: number): void {
        if (
          !Number.isInteger(ptr) ||
          !Number.isInteger(len) ||
          ptr < 0 ||
          len < 0
        ) {
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
    // Copy: TextDecoder rejects SharedArrayBuffer-backed views.
    return utf8Decoder.decode(new Uint8Array(bytes));
  }

  let text = "";
  for (const byte of bytes) {
    text += String.fromCharCode(byte);
  }
  return text;
}

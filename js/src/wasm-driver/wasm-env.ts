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

export const WASM_PAGE_BYTES = 65536;

// Must cover the module's declared import minimum (shadow stacks 0..4 MiB —
// coordinator + 3 fixed worker regions — then data pushed to 4 MiB by
// --global-base, ~4.05 MiB total today); instantiation fails loudly if the
// module ever outgrows it.
const INITIAL_PAGES = 80;

// The module decodes into an imported shared memory (threaded build). The
// provided maximum must not exceed the module's declared --max-memory (4 GiB).
// V8 reserves the provided maximum upfront for shared memories (growth is
// in-place), but the reservation is address space only: physical pages and
// page tables materialize on first touch (measured on Linux x64: 8 untouched
// 1GiB-maximum shared memories cost ~0 RSS). Size the maximum for the arm32
// VA budget, not for RAM. initial === maximum buys nothing: V8 does not
// constant-fold the size of a shared memory even when it is non-growable
// (measured null on the A55; it reloads the size per bounds-check region
// regardless — see the 2026-07-03 spike log entry).
export function createVip9rMemory(maximumPages: number): WebAssembly.Memory {
  return new WebAssembly.Memory({
    initial: INITIAL_PAGES,
    maximum: Math.max(maximumPages, INITIAL_PAGES),
    shared: true,
  });
}

// Minimal memory for a throwaway instance whose only job is answering
// vip9r_required_pages before the real memory's maximum is fixed (memory is
// imported, so sizing must precede the real instantiation).
export function createScratchVip9rMemory(): WebAssembly.Memory {
  return createVip9rMemory(INITIAL_PAGES);
}

// Packet-tail budget above the module-reported static requirement. 8 MiB
// anchors at 1080p: largest corpus packet 504 KiB (bbb_1920x1080 tile_1x4),
// VP9 level 4.1 caps the CPB at 3 MiB, and a lossless keyframe tops out near
// raw 4:2:0 (3.1 MiB). A multi-frame lossless superframe can legally exceed
// this — that is the fail-loud RESOURCE_LIMIT case. The budget is address
// space only until reserve_input actually grows into it.
const PACKET_TAIL_PAGES = (8 << 20) / WASM_PAGE_BYTES;

// Shared-memory maximum for a decoder session with the given max dimensions:
// the exact static layout (data + shadow stack + workspace arena) reported by
// vip9r_required_pages, plus the packet-tail budget. `exports` may come from
// a scratch instance over createScratchVip9rMemory().
export function sessionMaxPages(
  exports: WebAssembly.Exports,
  dims: { width: number; height: number },
): number {
  const requiredPages = exports.vip9r_required_pages;
  if (typeof requiredPages !== "function") {
    throw new Error("missing wasm export: vip9r_required_pages");
  }
  const pages = requiredPages(dims.width, dims.height) as number;
  if (!Number.isInteger(pages) || pages <= 0) {
    throw new Error(
      `vip9r_required_pages(${dims.width}x${dims.height}) failed: ${pages}`,
    );
  }
  return pages + PACKET_TAIL_PAGES;
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

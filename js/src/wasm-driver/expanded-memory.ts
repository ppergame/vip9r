// d8-only escape hatch from the fixed 64 MiB session shape (wasm-env.ts).
// A few libvpx resize vectors climb to 4096x2304 (2230 required pages),
// far past the 1080p shape the shipped policy is sized for. The golden
// driver rewrites the module's memory-import limits in place and creates a
// matching memory, so those vectors stay covered without raising the web
// ceiling. Only the two LEB128 limit fields change; the code section is
// byte-identical, and the declared min == max shape (what V8 keys its
// shared-memory codegen on) is preserved.

import { VIP9R_MEMORY_PAGES } from "./wasm-env";

// Packet-tail budget granted above vip9r_required_pages, mirroring the
// 8 MiB anchor baked into the fixed-memory policy (wasm-env.ts).
export const INPUT_TAIL_PAGES = 128;

// kind=memory, flags=has_max|shared, min=LEB(1024), max=LEB(1024) — the
// exact limits encoding the linker pins (rust/.cargo/config.toml).
const STOCK_LIMITS = Uint8Array.of(0x02, 0x03, 0x80, 0x08, 0x80, 0x08);

const IMPORT_SECTION_ID = 2;

// Copy of the module bytes with the memory import's min == max limits
// rewritten to `pages`. Same-length splice: the replacement LEB128 must be
// 2 bytes like the stock one, capping pages at 16383 (1 GiB) — plenty above
// the corpus maximum, and a loud throw if a future vector outgrows it.
export function expandedVip9rModuleBytes(
  bytes: Uint8Array<ArrayBuffer>,
  pages: number,
): Uint8Array<ArrayBuffer> {
  if (
    !Number.isInteger(pages) ||
    pages <= VIP9R_MEMORY_PAGES ||
    pages > 0x3fff
  ) {
    throw new Error(`expanded page count out of range: ${pages}`);
  }

  const section = importSectionRange(bytes);
  const matches = [];
  for (let at = section.start; at + STOCK_LIMITS.length <= section.end; at++) {
    if (limitsAt(bytes, at)) {
      matches.push(at);
    }
  }
  if (matches.length !== 1) {
    throw new Error(
      `expected exactly one stock memory-limits encoding in the import section, found ${matches.length}`,
    );
  }

  const leb = Uint8Array.of((pages & 0x7f) | 0x80, pages >> 7);
  const patched = bytes.slice();
  patched.set(leb, matches[0] + 2); // min
  patched.set(leb, matches[0] + 4); // max
  return patched;
}

function limitsAt(bytes: Uint8Array, at: number): boolean {
  for (let i = 0; i < STOCK_LIMITS.length; i++) {
    if (bytes[at + i] !== STOCK_LIMITS[i]) {
      return false;
    }
  }
  return true;
}

function importSectionRange(bytes: Uint8Array): {
  start: number;
  end: number;
} {
  if (bytes.length < 8) {
    throw new Error("wasm module too short");
  }
  let offset = 8; // magic + version
  while (offset < bytes.length) {
    const id = bytes[offset];
    offset += 1;
    // section size, LEB128
    let size = 0;
    let shift = 0;
    let byte;
    do {
      byte = bytes[offset];
      offset += 1;
      size |= (byte & 0x7f) << shift;
      shift += 7;
    } while (byte & 0x80);
    if (id === IMPORT_SECTION_ID) {
      return { start: offset, end: offset + size };
    }
    offset += size;
  }
  throw new Error("wasm module has no import section");
}

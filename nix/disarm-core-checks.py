# Disarm core's slice/array bounds-check panics (see unchecked-rustc.nix).
#
# Anchor-based, not a context diff: each edit matches a single identifier
# line and appends a replacement function at the end of the file, so upstream
# body/comment/attribute churn doesn't break it. An anchor that doesn't match
# exactly once fails the sysroot derivation — a toolchain bump can never
# silently build stock.
#
# Loud failure modes if upstream reshapes further: a signature change makes
# the appended replacement mismatch the lang-item ABI or the callsite arity,
# failing the core build. Dead stock functions are fine: cargo compiles
# build-std crates with --cap-lints allow.
import sys
from pathlib import Path

lib = Path(sys.argv[1])


def edit(path: str, anchor: str, replacement: str, append: str) -> None:
    p = lib / path
    text = p.read_text()
    n = text.count(anchor)
    assert n == 1, f"{path}: expected exactly 1 match of {anchor!r}, found {n}"
    p.write_text(text.replace(anchor, replacement) + append)


# Codegen locates panic_bounds_check by lang attribute, not name: detach the
# attribute from the stock function and hang it on the replacement.
edit(
    "core/src/panicking.rs",
    '#[lang = "panic_bounds_check"]',
    "// vip9r: lang item detached; the replacement at the end of this file"
    " owns it.",
    '''
// vip9r: slice/array OOB is unreachable-by-fiat; the wasm sandbox supersedes
// the check. inline(always) + unreachable_unchecked lets LLVM delete every
// bounds-check branch module-wide. #[track_caller] must stay: codegen passes
// a caller Location argument to this lang item.
#[inline(always)]
#[track_caller]
#[lang = "panic_bounds_check"]
fn vip9r_panic_bounds_check(_index: usize, _len: usize) -> ! {
    unsafe { crate::hint::unreachable_unchecked() }
}
''',
)

# slice_index_fail is a plain function called by name within this file:
# rename the stock definition out of the way and let the callsites resolve to
# the replacement.
edit(
    "core/src/slice/index.rs",
    "fn slice_index_fail(",
    "fn stock_slice_index_fail(",
    '''
// vip9r: OOB is unreachable-by-fiat; see panic_bounds_check in panicking.rs.
// The stock definition was renamed to stock_slice_index_fail; every callsite
// in this file resolves here instead.
#[inline(always)]
#[track_caller]
const fn slice_index_fail(_start: usize, _end: usize, _len: usize) -> ! {
    unsafe { crate::hint::unreachable_unchecked() }
}
''',
)

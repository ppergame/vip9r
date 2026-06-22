# vip9r — design

Working notes on constraints and mechanics. requirements.md holds the project
framing, roles, and durable decisions; this file should stay small enough to
guide the next session without pretending the decoder architecture is settled.

## Stable constraints

- Target is a freestanding `wasm32-unknown-unknown` module with manual bindings,
  a small export surface, and streaming I/O at frame granularity.
- Decoder work is Rust-first and spec-first. libvpx is a correctness/debugging
  oracle, not an implementation template.
- Unsafe Rust is allowed where integration or measured performance requires it;
  safe Rust remains the default.
- minih264 is used for the showcase path. It is not the current design center.
- SIMD, relaxed SIMD, workers, allocator shape, and internal boundaries are
  choices to earn with evidence, not decisions to pre-bake into the docs.

## Correctness

Bit-exact per-frame output against libvpx md5 is the eventual bar, reached in
two phases rather than gated incrementally:

- **Code-complete first (M1).** Build the whole profile 0 / 8-bit decode path
  from the spec. Almost nothing produces a correct full frame until the entire
  pipeline (entropy → dequant → inverse transform → prediction → reconstruction
  → loop filter) exists, so per-frame goldens cannot gate early work. M1
  verification is builds, `clippy`, targeted unit tests, engineering judgment,
  and the golden harness running end to end with output allowed to be wrong.
- **Bring-up second (M2).** Drive the golden harness green. The first frame
  whose md5 matches is the first-bit-exact milestone; from there it is
  debugging.

libvpx-generated outputs are useful when the spec or test vectors are not
enough. Intermediate checks may be added when they make failures easier to
localize, but the tap points and granularity should follow the implementation we
actually have. Do not commit the project to libvpx component boundaries just
because they are available to instrument.

### Golden harness

A host-side harness decodes a vector frame by frame and compares per-frame md5
against the `.md5` golden. The libvpx md5 protocol has sharp edges:

- The hash is over the raw I420 frame: Y (visible `d_w`×`d_h`), then U, then V
  at chroma dims (`⌈w/2⌉`×`⌈h/2⌉`), visible dimensions only — no stride padding.
- Only _shown_ frames produce a line, in display order. A superframe packs
  several coded frames into one IVF packet but usually shows one;
  `show_existing_frame` re-emits a stored frame and gets its own line. So output
  frame count ≤ coded frame count.
- Golden format is one line per shown frame: `<md5hex>  <name>.i420`. Compare
  positionally.

Start target is `bear-vp9.ivf` (320×240, 82 frames) — IVF, so no webm demux is
needed to begin.

## Measurement

The trusted harness answers two questions:

- does this revision decode correctly?
- is this revision faster on the target path?

Correctness runs natively against the core's host `std` build — the cheapest
loop. d8 covers the wasm build: host d8 for cheap parity and smoke, device d8 as
the performance ground truth, run A/B interleaved against the current baseline
with enough repetition to report a credible delta. Targeted microbenchmarks are
allowed when a full-decode result points at a hotspot; their wins only count
after reconfirming full-decode wall time.

Only rev-built runs — built by the trusted harness from a VCS revision — are
citable in `docs/log.md`. Ad-hoc blobs are for exploration.

### V8 artifacts

The devshell pins Google-published V8 canary bundles. Operational details for
bumping those pins, running Android `d8` under qemu user emulation, and
extracting ARM Wasm assembly live in [`docs/d8.md`](d8.md).

## Work shape

- The interactive session owns intent, trusted harness work, and review.
- Implementation agents should receive narrow, reviewable tasks with the needed
  spec excerpts, tests, and oracle handle.
- Rust implementation state lives in the self-contained Cargo workspace under
  `rust/`. Keep maintainer/orchestrator tooling outside that tree unless the
  implementor needs it for the task.
- Keep task boundaries provisional. Split by whatever makes correctness,
  measurement, and review easiest at the time.
- Do not turn temporary boundaries into architecture unless they survive contact
  with implementation and measurement.

## Record

- `docs/tracker.md` is the ordered backlog and current frontier.
- `docs/log.md` is the human-facing implementation/optimization record. Entries
  should cite a revision and either a first-correctness milestone or a measured
  performance delta.
- Add extra notes only when they prevent rediscovering a real decision or
  failure mode. Avoid ledgers for speculative module pieces.

## Corpus

Test media is mapped at `/bulk/vip9r` on the host (`/media` inside the grinder
sandbox). Each vector has a `.md5` golden.

- **Correctness:** `libvpx/` conformance vectors (profile 0 / 8-bit subset) plus
  `chromium/bear-vp9.ivf` as the IVF bring-up target.
- **Performance:** `realworld/` 720p clips with distinct character (high-motion,
  film grain, screen content, talking head). Keep at least one held out for
  review.

## Open design questions

- Minimal wasm export surface and JS bindings for the d8 parity check and the
  demo.
- Which intermediate checks, if any, are worth adding after the first failures.
- Device-time and token budget per optimization pass.

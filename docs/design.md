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
  and the host golden harness running end to end with output allowed to be
  wrong.
- **Bring-up second (M2).** Drive frame-md5 correctness green. The fast host
  harness is the primary debugging loop; the wasm driver is the shipping-path
  parity check. The first frame whose md5 matches is the first-bit-exact
  milestone; from there it is debugging.

libvpx-generated outputs are useful when the spec or test vectors are not
enough. Intermediate checks may be added when they make failures easier to
localize, but the tap points and granularity should follow the implementation we
actually have. Do not commit the project to libvpx component boundaries just
because they are available to instrument.

Spec note: the local VP9 v0.7 draft's partition probability prose appears to
reverse the `FrameIsIntra` condition. The decoder uses the fixed
`kf_partition_probs` table for key/intra frame partition syntax, matching the
table naming and key-frame syntax context.

### Golden harnesses

Golden harnesses decode a vector frame by frame and compare per-frame md5
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

The host harness runs `vip9r-core` directly. It is the cheap core bring-up and
debug loop, not the final shipping-path gate:

```sh
cd rust
cargo run -p vip9r-tools -- golden /bulk/vip9r/chromium/bear-vp9.ivf
```

Inside grinder sandboxes, use `/media/chromium/bear-vp9.ivf`. The harness
defaults the golden path to the `.md5` sidecar. It exits non-zero on decode
errors, missing/extra shown frames, or md5 mismatches; `--allow-mismatch` only
permits wrong frame hashes for code-complete smoke runs. The current decoder is
still expected to stop at `Unimplemented`.

The wasm driver exercises the manual boundary under d8 with the same md5
protocol. It is the wasm/shipping-path parity gate, but grinder implementors do
not get it by default while current tasks are Rust/core-focused:

```sh
cd rust
cargo build -p vip9r-wasm --target wasm32-unknown-unknown
cd ../js
pnpm build:wasm-driver
$D8_LINUX64 dist/wasm-driver/main.js -- \
  ../rust/target/wasm32-unknown-unknown/debug/vip9r_wasm.wasm \
  /bulk/vip9r/chromium/bear-vp9.ivf
```

Until reconstruction exists, the expected wasm-driver failure is
`vip9r_decode_next: unimplemented (-8)`. A failure earlier than
`decode_next` means the JS/wasm wrapper, exported ABI, input copying, or packet
setup regressed. The driver also accepts `--allow-mismatch`; like the host
harness, this only permits wrong frame hashes, not missing or extra shown
frames.

### Decode API

The primary core API is packet splitting plus one-coded-frame decode. A demuxed
VP9 packet may be a superframe; `split_packet` returns up to 8 coded-frame byte
ranges. `Decoder::decode_coded_frame` consumes one range with an explicit
`DecodeWorkspace` and returns either no output or one shown frame. Packet-level
helpers are adapters, not the architecture.

`Decoder` owns VP9 semantic session state. Geometry-sized storage belongs to
the supplied workspace. `WorkspaceLayout` currently defines a fixed arena with
one current reconstruction frame slot plus 8 reference frame slots, each using
simple 4:2:0 byte capacity derived from instance max dimensions. Additional maps
and scratch should enter the layout only when implementation code actually
consumes them.

Shown-frame output borrows from the supplied workspace and is invalid after the
next decode or packet transition. Core output is an `I420Frame`: visible Y, U,
and V planes with a shared `PlaneShape` for width, height, and stride, plus a
backing byte slice. Compact I420 in libvpx-md5 order is a tools/harness
serialization, not the core output model.

The wasm boundary uses the same shape: one instance is one decoder session,
`begin_packet` stages packet ranges, and `decode_next` advances exactly one
coded frame. There is no reset API; JS recreates the instance on stream changes
or decode errors.

## Measurement

The trusted harness answers two questions:

- does this revision decode correctly?
- is this revision faster on the target path?

Correctness bring-up runs natively against the core's host `std` build because
that is the cheapest debugging loop. Wasm/d8 runs cover the exported ABI and the
shipping build: host d8 for cheap parity and smoke, device d8 as the performance
ground truth, run A/B interleaved against the current baseline with enough
repetition to report a credible delta. Targeted microbenchmarks are allowed when
a full-decode result points at a hotspot; their wins only count after
reconfirming full-decode wall time.

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
- Grinder sandboxes make `rust/` the repo root, while the main checkout's VCS
  root is the project root. When importing sandbox diffs manually, apply them
  from the project root with `git apply --directory=rust` and verify `jj status`
  or `git status` afterward. Do not trust a quiet patch command alone.
- Keep task boundaries provisional. Split by whatever makes correctness,
  measurement, and review easiest at the time.
- Prefer grinder tasks smaller than the first packet front-end handoff. That
  bootstrapped useful structure but landed about 1k LOC; ordinary implementor
  tasks should be easier to review in isolation.
- Good grinder packets tend to change one kind of thing: one parser primitive,
  one syntax-table slice, one data model needed by the next slice, or one
  measured optimization. Borderline packets mix modeling choices, semantic
  changes, and fixture churn. Split before a task asks the implementor to both
  invent a shape and consume it broadly.
- Size by review risk, not lines. A few hundred lines of mechanical tables or
  local tests may be fine; a small diff can still be too large if it commits the
  decoder to a hard-to-unwind interpretation of the spec.
- Parallel grinders are for disjoint write sets or independent investigations.
  Sequential is the default when tasks share parser state, probability tables,
  frame storage, or other files where merge conflicts would hide review issues.
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

- Minimal wasm export surface and JS bindings for the wasm parity check and the
  demo.
- Which intermediate checks, if any, are worth adding after the first failures.
- Device-time and token budget per optimization pass.

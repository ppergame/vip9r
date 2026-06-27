# vip9r — tracker

The "what's next" surface. Co-maintained by the user and the agent. Keep this as
an ordered backlog, not an architecture commitment.

## How to edit this

- Prefix a task with `user:` only when it needs external input or a decision
  from the user.
- Move bullets to change priority. `Now` is the default work queue.

## Now

M0 — decode API and the fast host golden harness.

- [x] user: VP9 profile 0 / 8-bit spec source
- [x] user: conformance vectors for the profile 0 / 8-bit subset
- [x] prebuilt d8 binaries for host and ARM
- [x] freestanding `no_std` wasm skeleton with a minimal decode API
- [x] finalize the core decode API for host harness frame output
- [x] fast host golden harness: IVF demux, per-frame md5 vs the `.md5` golden,
      on `bear-vp9.ivf`
- [x] first implementor handoff packet

## Waiting

Deferrable external inputs for M3 performance work.

- [ ] user: root the Pixel 9a
- [x] user: 720p performance corpus with a held-out clip

## Later

### M0 device path — gates M3

- [ ] device oracle that serializes access to adb and hardware
- [ ] host and ARM d8 paths for wasm inspection and timing, JIT tier control
- [ ] repeatable device timing protocol with CPU control and confidence checks

### M1 — Decode, code-complete (implement campaign)

The whole decode path, code-complete; correctness is M2's job. Gated on the M0
harness.

- [x] packet front-end parser: superframe splitting and uncompressed header
      parsing
- [x] tile payload layout validation
- [x] boolean decoder primitive
- [x] retain uncompressed-header state needed by compressed-header parsing
- [x] intra compressed-header parser and default tx/skip/coef probability state
- [x] key/intra tile partition and mode-info syntax through residual handoff
- [x] key/intra residual traversal and coefficient token syntax parse-only
- [x] inter compressed-header parser and default non-coef/MV probability state
- [x] inter tile partition/mode-info/MV syntax and residual parse-only
- [x] syntax counts and adaptive probability refresh
- [x] resolve `bear-vp9.ivf` packet 17 inter tile syntax
      `InvalidBitstream` after probability refresh
- [x] resolve `bear-vp9.ivf` packet 25 inter tile syntax
      `InvalidBitstream` after previous-frame MV candidates
- [x] move previous-frame MV mode-history storage from the std-only host parser
      path into workspace/no-std state before relying on wasm parity
- [x] shaped frame-pool/reference output plumbing with neutral pixels, so host
      and wasm smoke runs have shown-frame cadence instead of missing output
- [x] decompose the remaining pixel path into first-pass reviewable blocks
- [x] residual coefficient storage and dequantization data model
- [x] inverse transform kernels for the profile 0 / 8-bit subset
- [x] intra prediction and reconstruction into the current frame
- [x] inter prediction from reference frame slots
- [x] loop filter and final reconstructed reference/output pixel path

### M2R — Runner engineering

Make wasm/d8 the canonical correctness and unit-test frontend before expanding
M2 beyond `bear-vp9.ivf`. Gated on M1; gates M2.

- [x] collapse the Rust decode implementation and wasm ABI into one implementation
      crate; keep boundaries as modules, not a native-vs-wasm crate split
- [ ] add a d8 wasm unit-test runner that discovers specially named test exports,
      runs tests under fresh-enough wasm instances, and reports pass/fail/ignored
      results without wasmtime
- [ ] add a `wasm-tests` feature and test-export macro shape; prefer `macro_rules!`
      first, add a proc-macro crate only if attribute syntax earns its cost
- [ ] add wasm-test print support: static bounded formatting buffer, imported JS
      sink function, `test_println!`, and a panic handler that prints `PanicInfo`
      before trapping
- [ ] migrate the `bear-vp9.ivf` strict md5 check to the wasm/d8 frontend as the
      canonical golden path
- [ ] remove `vip9r-tools`; do not add WebM or new correctness surface area to the
      native runner
- [ ] update `docs/design.md` and `scripts/grinder-system-prompt.md` so future
      tasks use wasm/d8 for full-decode correctness and wasm unit tests for
      target-exact implementation checks

### M2 — Decode, correct (bring-up campaign)

Drive frame-md5 correctness green. Gated on M2R.

- [x] first bit-exact frame
- [x] `bear-vp9.ivf` strict md5 green on host and wasm
- [ ] conformance subset green on full frames
- [ ] webm demux + corpus clips green

### M3 — Decode, fast (optimize campaign)

Sustained 720p30 on the pinned big core. Gated on M2 + the M0 device path.

- [ ] optimize measured full-decode hotspots; confirm wins against full-decode
      wall time

### M4 — Encode

minih264 in, MSE-playable H.264 out. Fitness gains a VMAF/size floor.

- [ ] integrate minih264 behind the required wasm/libc boundary
- [ ] establish the VMAF/size floor

### M5 — Chrome demo

Real browser on the phone, MSE player page, full VP9→H.264 loop.

- [ ] demo page: vite/TS, WebM demux + ISO BMFF mux, MSE append

## M6 — Stretch

relaxed-simd · threads · little-core (Cortex-A520) · 1080p

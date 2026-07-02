# vip9r — tracker

The "what's next" surface. Co-maintained by the user and the agent. Keep this as
an ordered backlog, not an architecture commitment.

## How to edit this

- Prefix a task with `user:` only when it needs external input or a decision
  from the user.
- Move bullets to change priority. `Now` is the default work queue.

## Now

M0 — decode API and the first frame-md5 golden harness.

- [x] user: VP9 profile 0 / 8-bit spec source
- [x] user: conformance vectors for the profile 0 / 8-bit subset
- [x] prebuilt d8 binaries for host and ARM
- [x] freestanding `no_std` wasm skeleton with a minimal decode API
- [x] finalize the core decode API for frame output
- [x] initial golden harness: IVF demux, per-frame md5 vs the `.md5` golden, on
      `bear-vp9.ivf`
- [x] first implementor handoff packet

## Waiting

Deferrable external inputs for M3 performance work.

- [x] user: root the Pixel 9a
- [x] user: 720p performance corpus
- [ ] user: collect the ffvp9 performance baseline: static ffmpeg (arm64 +
      armv7) run directly through adb, taskset-pinned, on the realworld 720p
      clips across Pixel 9a core types and the streamer. Deferred by the user
      2026-07. Strategically it bounds the A55 target: if ffvp9 can't do
      single-threaded 720p30 there, that's a threads/M6 scope decision, not an
      optimization gap

## Later

### M0 device path — gates M3

- [x] device oracle that serializes access to adb and hardware
- [x] host and ARM d8 paths for wasm inspection and timing, JIT tier control
- [x] repeatable device timing protocol with CPU control and confidence checks
- [x] return native (host / ARM) assembly for the wasm module from d8 back to
      the grinder to close the codegen feedback loop
- [x] surface perf-run profile traces (simpleperf / d8 wasm samples) back to the
      orchestrator and grinder, so M3 hotspot attribution is measured rather
      than guessed
- [x] systemize wasm runner JSON output; JSON stdout is the default for
      `wasm-golden` validation, `wasm-golden --bench`, and `wasm-microbench`;
      `wasm-tests` keeps explicit `--json` because its human output is useful

### Perf oracle hardening — supports M3+

Motivated by the 2026-07 scalar campaign: A/B position bias (~2% cool-start,
~13% heat-soaked) made sub-5% deltas unjudgeable, and daemon-held baselines
forced a restart after every merge.

- [x] counterbalanced bench: one submission runs B, C, C, B so both sides
      average the same position and monotone thermal drift cancels
- [x] position-corrected reporting: response carries the corrected delta
      `mean(C2,C3)/mean(B1,B4) - 1` as the headline number, the four raw
      per-run measurements, and the `B1` vs `B4` spread as a built-in no-op
      control / error-bar signal so callers can tell when a run was too
      drifty to conclude anything. 2026-07-02: both host and device benches
      run B1,C2,C3,B4 and report `bench.corrected_delta` /
      `bench.baseline_spread` over `measurement.msPerFrame`
- [x] baseline moves from daemon state to request payload: `serve` no longer
      builds a baseline, so merges stop forcing daemon restarts; device
      pushes cached by blob hash
- [x] `serve` takes a list of devices (`--serial` repeated), one queue and
      worker per device; pin moved entirely to request config (timed kinds
      require an explicit pin, validate/tests default `any`)
- [x] `vip9r-perf-submit` takes a baseline wasm path and a device index;
      both are required for device benches (no quiet fallbacks), with
      `--no-op-control` as the explicit way to A/B a build against itself
- [x] grinder spawn script: `grinder run TASK --baseline WASM --device
      INDEX[:PIN]` passes sandbox-wide submit defaults via
      `VIP9R_PERF_BASELINE` / `VIP9R_PERF_DEVICE`
- [x] trim full device summary in every perf-submit response: responses
      carry index/serial/model; full topology stays in `probe`

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
- [x] resolve `bear-vp9.ivf` packet 17 inter tile syntax `InvalidBitstream`
      after probability refresh
- [x] resolve `bear-vp9.ivf` packet 25 inter tile syntax `InvalidBitstream`
      after previous-frame MV candidates
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

- [x] collapse the Rust decode implementation and wasm ABI into one
      implementation crate; keep boundaries as modules, not a native-vs-wasm
      crate split
- [x] add a d8 wasm unit-test runner that discovers specially named test
      exports, runs tests under fresh-enough wasm instances, and reports
      pass/fail results without wasmtime
- [x] add a `wasm-tests` feature and proc-macro `#[wasm_tests]` module shape
      that exports inline `#[test]` functions for the d8 runner
- [x] add wasm diagnostic print support: bounded stack formatting buffer,
      imported kinded JS sink function, `diag!`, `Result::Err` test messages,
      and a panic handler that prints `PanicInfo` before trapping
- [x] migrate the `bear-vp9.ivf` strict md5 check to the wasm/d8 frontend as the
      canonical golden path
- [x] remove `vip9r-tools`; do not add WebM or new correctness surface area in
      this migration
- [x] update `docs/design.md` and `scripts/grinder-system-prompt.md` so future
      tasks use wasm/d8 for full-decode correctness and wasm unit tests for
      target-exact implementation checks

### M2 — Decode, correct (bring-up campaign)

Drive frame-md5 correctness green. Gated on M2R.

- [x] first bit-exact frame
- [x] `bear-vp9.ivf` strict md5 green on host and wasm
- [x] strict wasm-golden green for `vp90-2-12-droppable_{1,2,3}.ivf`
- [x] fix `vp90-2-05-resize.ivf`: strict wasm-golden green after MI-rounded
      reconstruction extents
- [x] fix `vp90-2-09-subpixel-00.ivf`: strict wasm-golden green after MSB-first
      `f(32)` tile-size parsing
- [x] fix `vp90-2-09-aq2.webm`: strict wasm-golden green after segment-map
      parsing and ALT_Q dequantization
- [x] fix `vp90-2-18-resize.ivf`: 55 frame-md5 mismatches after successful full
      decode
- [x] fix `vp90-2-13-largescaling.webm`: strict wasm-golden green after
      expanding fixed tile-context column storage for 19200px+ frames
- [x] fix `vp90-2-19-skip-02.webm`: strict wasm-golden green after preserving
      `PrevSegmentIds` across frames that do not update the segment map
- [x] fix `vp90-2-22-svc_1280x720_3.ivf`: strict wasm-golden green after
      deriving zero IVF dimensions from md5 sidecar names and comparing the top
      spatial layer
- [x] local md5-backed corpus green: strict wasm-golden passes every current
      `/bulk/vip9r` md5-backed media file, 330/330 total, covering the chromium
      smoke vector, libvpx conformance/perf vectors, and local realworld WebM
      clips
- [x] exercise WebM demux on YouTube VOD tracks; classify Track/Cluster
      ordering, init/media segment split, `SeekHead`, and unknown-size element
      cases before widening parser scope

### M3 — Decode, fast (optimize campaign)

Sustained 720p30 on the pinned big core. Gated on M2 + the M0 device path.

Starting evidence (2026-07): per-core baselines are in `docs/design.md` (X4 ~147
ms/frame vs the 33.3 ms budget). The only profile so far — bear on the X4 —
splits `predict_inter` 58% / `loop_filter_frame` 19% / `decode_block` 9% / IDCT
~7%; entropy and memmove shares should grow on 720p content. The pass order
below follows that evidence plus two stacking arguments: algorithmic reshaping
must precede SIMD (per-pixel spec-literal loops can't be vectorized), and the
multipliers compound. It is a suggested order, not a commitment — re-profile
between passes and reorder on what you see. One measured pass per grinder task;
`scripts/vip9r-corpus-golden.py` (compliance set, ~1 min) gates each merge and
`--all` runs at pass boundaries; log measured device deltas.

- [x] refresh hotspot attribution on 720p realworld clips (Pixel + streamer);
      measure a bare `+simd128` flag flip (autovectorization only) so the flag
      is part of the baseline before any hand-written kernels. 2026-07-02:
      X4 jellyfish 51% predict_inter / 25% loop_filter / 10% decode_block /
      4% IDCT; A55 27% loop_filter / 25% predict_inter / 16% decode_block /
      6% IDCT (2-frame keyframe-weighted window). Flag flip neutral on both
      (X4 within noise, A55 dead even); kept in the baseline
- [x] inter prediction: block-level two-pass separable convolution, unscaled
      fast path, integer-MV copy, per-block clamped edge gather (no
      `WorkspaceLayout` change needed). X4 jellyfish: 143.6 → 77.3 ms/frame
- [x] loop filter: strength LUTs, per-SB decision cache, span filtering,
      sliding-sum wide filter, scalar. X4: 76.2 → 43.5 ms/frame
- [x] bool decoder wide window: measured negative (≈ +0.4% after position-bias
      control), not merged. Under V8 the refill is not the bottleneck;
      remaining `decode_block` cost is token/syntax structure above the
      primitive — a different, deeper reshape if entropy stays hot
- [x] IDCT: dequant nonzero-row tracking + zero-row skip, DC-only DCT_DCT
      path, unchecked hot butterflies (documented malformed-stream behavior
      change). X4: 42.6 → 24.2 ms/frame
- [x] coefficient token loop hoisting: block-invariant probability/counts
      rows, const scan/band slices, const-shaped token tree. ~2% X4, at the
      credibility line; kept as hot-loop simplification. Wide-window bool
      decoder, unrolled tree, neighbor tables, direct coef writes all
      measured worse and were rejected
simd128 kernel campaign (scoped 2026-07-02). Framing: portable simd128, not
single-target tuning — A/B on the X4 for iteration speed, log an A55
confirmation bench per merged pass. simd cannot close the A55 budget gap
(~9x over, and decode_block's ~30% is serial entropy); the campaign goal is
the portable win plus A55 data for the later threads decision (M6). One
kernel per grinder task, sequential (everything touches `tile_syntax/mod.rs`);
reorder on refreshed profiles. Starting X4 profile (jellyfish 0:15, 23.1
ms/frame): inter subpel 33%, decode_block 30%, loop filter 15%, IDCT 7%.

- [x] pass 0: post-reshape attribution on X4 *and* A55. 2026-07-02: X4
      jellyfish 0:15 at 23.2 ms/frame — subpel 33% / decode_block 31% /
      loop_filter 15% / IDCT 6%; A55 0:5 at 275 ms/frame — subpel 24% /
      decode_block 24% / loop_filter 9% / IDCT 6% (+ ~19% d8/libc). Loop
      filter no longer outranks convolution on the A55; tracker order
      holds on both devices. `--all` corpus rerun green (337/337; 338
      sidecars minus one excluded yt raw-prefix clip)
- [x] inter subpel convolution simd — per-tap widening MAC on i32 lanes,
      saturating narrow; scalar kept as test reference/tails. X4 −16.7%
      (23.8 → 20.1 ms/frame), A55 −14.5% (272 → 233). Exposed and fixed a
      harness hole: cargo builds outside `rust/` dropped
      `.cargo/config.toml` target features; harness cargo invocations now
      pin cwd to the workspace
- [x] loop filter simd — measured negative, not merged. Best variant
      (pass-1 Tx4x4 narrow only) −0.6/−0.7% vs 1.1/0.8% spread on X4;
      fuller variants slower. The scalar span reshape already amortized
      decisions; pass-0 transpose and wide-filter blends cost more than
      lanes save under V8. Relaxed-simd/threads-era retry candidate
- [x] IDCT simd — DCT rows/columns in 4-wide i32 lanes, i64x2 extmul
      butterflies with wrapping-narrow packs reproducing scalar
      bit-exactly; ADST/WHT stay scalar. X4 −13.0% (20.1 → 17.5 ms/frame),
      A55 −9.6% (233 → 211). Overshoot vs the 6% profile share is the 2-D
      driver work that inlining had attributed to decode_block
- [x] conditional tail, by post-IDCT profile: reference slot remap merged
      (9 physical buffers + slot table, refresh is a table write; perf at
      the noise line, taken for structure — kills the per-frame copy and
      the "current precedes refs" constraint ahead of threads). Compound
      average already covered by the convolution Average path; intra
      prediction <0.3% of samples and counts accumulation not separately
      attributable — both skipped. Stride normalization dropped: no
      profile evidence
- [x] end-of-campaign: both-device margin table in log.md (2026-07-02).
      Campaign total X4 −29.5% jellyfish / −24.3% BBB; A55 −22.8% /
      −9.2%. X4 sits ~2x under the realtime budget; A55 is 6.3x over on
      jellyfish-class 720p30 — even ideal 4-core scaling leaves it 1.6x
      over, which bounds the threads / ffvp9-baseline scope decision (M6)

### M4 — Encode

minih264 in, MSE-playable H.264 out. Fitness gains a VMAF/size floor.

- [ ] integrate minih264 behind the required wasm/libc boundary
- [ ] establish the VMAF/size floor

### M5 — Chrome demo

Real browser on the phone, MSE player page, full VP9→H.264 loop.

- [ ] demo page: vite/TS, WebM demux + ISO BMFF mux, MSE append

## M6 — Stretch

relaxed-simd · threads · little-core (Cortex-A520) · 1080p

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
- [x] user: collect the ffvp9 performance baseline: static ffmpeg (arm64 +
      armv7) run directly through adb, taskset-pinned, on the realworld 720p
      clips across Pixel 9a core types and the streamer. 2026-07-02: done via
      `scripts/ffvp9-baseline.py`; ffvp9 does 720p30 single-threaded on the
      A55 with ~30% margin on the worst clip, so the streamer target is an
      optimization gap (~9x), not a scope decision. Table in log.md

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

A55 freeform campaign (scoped 2026-07-02). Primary A/B target is the A55
streamer (device 1, pin cpu:0); X4 confirmation secondary. Constraint: keep
changes principled — plausibly beneficial under future V8 versions and in the
browser, not tuned to one d8 build. Evidence base: 2026-07-02 A55 profiles
(temp/vip9r-profiles/c599be6a-*) — decode_block 26/35% (jellyfish/BBB), d8
binary 15%, subpel 13/4%, loop_filter 10/5%, libc 9/15% (93% memcpy/memset);
detailed findings with file:line evidence in temp/orchestrator-notes.md.
Sequential grinder passes (everything touches tile_syntax/); corpus golden
gates each merge, `--all` at campaign boundaries; BBB is the sensitive clip
for residual/copy passes, jellyfish for subpel.

- [x] H1 — measurement-window-only profiling: profile reports now window
      samples to the final measurement.elapsedMs of the recording
      (daemon-side sample-time filter, no driver change). Clean A55
      attribution — jellyfish: decode_block 38% / subpel 18% /
      loop_filter 15% / libc 11%; BBB: decode_block 50% / libc 20% /
      loop_filter 7% / subpel 6%. Real d8 share during decode is 1-2%
- [x] H2 — resolved, no hot memory.copy trampoline exists: the 0xea79xx
      cluster is Builtin:DoubleToI (md5 validation JS, correctly
      attributed all along; the exploration session's raw histogram
      double-counted map-covered samples). Remaining unattributed d8
      samples are diffuse V8 C++ (max 64B bucket ~0.5% of total), mostly
      excluded by the H1 window. libc memcpy/memset share is real decode
      cost, per the windowed profiles above
- [x] P1 (+P6) — residual path restructure: parser-owned ResidualBuffers,
      scan-prefix-scoped clears, eob-bounded dequant in scan order,
      eob==0 skips dequant/IDCT/reconstruct, no by-value struct moves,
      shift/mask in coefficient_token_context. A55 −16.1% BBB / −14.2%
      jellyfish (spread ~0.5%). Parse→dequant fusion not re-tested
      (deferred; clean comparison now possible inside new structure)
Post-P1 A55 re-attribution (inline(never) split of decode_block, BBB):
tokens 18% / decode_residual traversal glue 16% / predict_intra 10% /
decode_partition 4.5% / add_residual 4.3% / mode-info syntax ~6% /
dequantize 2.4%. libc collapsed to 6%/4% (BBB/jelly) after P1, so P3
shrank; traversal glue and intra prediction were promoted.

- [x] P1b — residual traversal restructure: measured negative, not
      merged. Three correct hoist/slice/unsafe variants all *regressed*
      A55 (+1.6..3.1%); the skip-block context-fill fast path alone was
      exactly noise on both clips. TurboFan already elides the checked
      ceremony; the traversal's 16% attribution is intrinsic loop work +
      code layout, not removable branches. Feeds the P5 icache/layout
      question — treat decode_block-region layout as fragile
- [x] P2 — simd `add_residual_block`: interior v128
      widen/add/saturating-narrow rows (clip1-exact), scalar edge
      fallback, simd-vs-scalar sweep test. A55 −2.6% BBB / −3.4%
      jellyfish (spreads ~0.4%)
- [x] P-intra — predict_intra fast path: persistent pred buffer (1KB
      zero-fill dropped), interior row-slice write-out with fixed-width
      stores, DC/V/H/TM row/simd kernels + direct-to-plane for interior
      blocks, directional modes stay scalar. A55 −2.6% BBB / −3.1%
      jellyfish (no-op anchor −0.7%); buffer+write-out alone carries
      most of BBB, mode kernels add on jellyfish. Edge gather untouched
      (not worth a follow-up at current attribution)
- [x] P4 — convolution kernel v2: merged extmul MACs + interp-buffer
      zero-fill removal, A55 −1.5% jellyfish / −0.5% BBB (memset removal
      is the main win). Negatives with asm evidence: V8 arm32 lowers
      shuffles to VTBL not VEXT (shuffle tap-gather dead), vertical
      sliding window spills (V8 regalloc), extmul does not fuse to
      VMLAL, pure-i16 impossible (coeff sum 182×255 > i16). Follow-up
      landed: MaybeUninit buffer replaced with parser-owned persistent
      `[u8]` (zero unsafe, −342 lines, simd/scalar helpers unified under
      const generic), perf-neutral on both clips
- [x] P3 (demoted, closed) — small-copy elimination in prediction
      paths: the measurable part shipped as P3-lite (persistent intra
      edges); post-P-intra profile had libc at 6.2%/2.8% (BBB/jelly)
      with no single remaining site worth a grinder pass. Campaign
      closed without revisiting
- [x] P8 — loop filter simd retry (post-P-intra profile: 18.9% jelly /
      8.6% BBB): pass-1 (horizontal edges) Tx4x4 narrow filter in 8 u8
      lanes, transpose-free; pass 0 and wide filters stay scalar.
      Grinder's 8-lane kernel failed device-only; orchestrator bisect
      found a real V8 arm32 codegen bug — i16x8_bitmask lowering leaks
      its powers-of-two lane constant into the aliased high D-half of a
      live Q register under pressure (lanes 4..7 corrupted). Fixed by
      v128_any_true for the early-out. A55 jellyfish −2.1% (grinder,
      equivalent kernel) / −1.5%, −0.6% confirm runs; BBB neutral.
      Precheck-only variants measured positive (slower), dropped
- [x] P5 (resolved by direct PMU measurement, no microbench needed):
      A55 jellyfish decode has stalled-cycles-frontend 13.1%, L1I
      refills 68.2M vs L1D 13.8M over 5s (5:1, 0.82/100 instr) —
      icache thrash confirmed as the monolith cost; spawned the
      residual-icache layout task on this evidence. P7 closed
      without work: entropy payload is bitrate-bounded (~56
      Kbit/frame jelly), refill cannot account for the monolith —
      X4-negative stands for A55. P6 closed: coefficient_token_context
      already shift/mask, token_cache clear already scan-scoped
      (absorbed by P1)
- [x] P-adst — simd 2D transform path generalized from DCT_DCT-only
      to per-pass DCT/ADST selection (DctAdst/AdstDct/AdstAdst):
      inverse_adst4/8/16 in i32x4 lanes with i64x2-extmul sb/sh
      mirroring scalar exactly, same 4-lane row/column groups and
      persistent scratch, scalar leftover-row tail kept. A55
      jellyfish −2.5% grinder / −3.1% confirm (spreads ≤0.7%), BBB
      −2.3% (spread 0.4%), anchors null. Scalar adst symbol gone
      from profile. Residue: adst kernels re-introduce small zeroed
      scratch copies (rare path, ≤0.3% total, candidate micro)
- [x] P-i16dct — i16-domain DCT_DCT inverse transform, 8 lanes per
      group. Spec-licensed: bitstream conformance requires all
      T-array values (and H's v/w) to fit 8+BitDepth bits
      (spec:4362/4391/4426/4442) = i16 at depth 8. Butterflies keep
      exact single-rounding via i32x4_extmul accumulation (q15mulr
      double-rounding explicitly rejected); saturating narrows only
      reachable off-conformance; ADST/mixed/lossless untouched (S
      array is spec-"higher precision"). 8-lane groups + 4-lane i16
      half-group tail + scalar 1-3-row tail. A55 jellyfish −5.4%
      grinder / −4.0% confirm (spreads ≤0.8%), BBB neutral. Sweep
      DctDct range scoped to conforming magnitudes; corpus
      (incl. quantizer-63) owns high-range coverage
- [x] icache layout experiment — measured null, NOT merged: the one
      clean cold-outline shrank decode_residual 140.7→135.0KB arm32,
      bench noise on both clips. L1I cost is per-block phase cycling
      through hot code, not cold-code pollution; only P9-scale phase
      batching could move it. Evidence in log
- [x] P10 + micro sweep — merged on mechanism, timing null: per-frame
      1.4MB neutral fill → per-slot first-use fill (reader audit:
      intra edges use explicit defaults, inter clamps to reference
      dims, loop filter stays in decoded extent; validates incl.
      resize/18x34/show-existing); ADST simd+scalar scratch made
      persistent (MAX_ADST_WIDTH=16, staleness-safe by write-before-
      read within each call); memory.fill sites 33 → 24 in the dump.
      A55 timing legs ranged −1.1%..+0.2% across sessions — noise
      floor; merged for the traffic reduction, not the stopwatch
- [x] P9 SB-row-fused loop filtering: declined — L2D refill only
      5.5M/s ≈ 0.9GB/s (PMU-measured), too weak to justify the
      structural cost; icache experiment already showed the phase-
      cycling cost needs batching that intra dependencies entangle
- [x] P3-lite — persistent IntraPredictionEdges + slice above-row
      gather: in-place fill with explicit missing-edge defaults
      (127/129 spans only), interior above row copied as fixed-width
      chunks, overhang keeps clamped path. Kills the per-intra-block
      96B struct copy + init fills (verified gone from wasm dump).
      A55 BBB −3.6% (1080p libvpx clip, spread 0.3%) / −1.2% (720p
      wikimedia, spread 0.3%); jellyfish noise-level as expected
- [x] P-idct-scratch — dead scratch traffic in the simd inverse DCT:
      gathers write directly into bit-reversed `brev(n, i)` slots
      (permutation pass deleted; brev is an involution so the scalar
      path uses in-place disjoint swaps), and the per-group 512B
      zero-init becomes persistent scratch on DequantizedCoefficients.
      All four 512B fill/copy sites gone from the wasm dump;
      decode_residual memory ops 18 → 7. A55 BBB −5.1% grinder /
      −4.4% confirm (spread 0.3-1.0%), jellyfish −1.1% (spread 0.2%)
- [x] P8-wide — pass-1 wide loop filter simd (Tx8x8 wide3 kernel,
      Tx16x16/Tx32x32 wide3/wide4 kernel), same transpose-free 8-lane
      structure as the narrow kernel, disjoint regime masks blended
      via bitselect, v128_any_true early-outs (no bitmask ops).
      Segment-mix histogram justified both kernels (jelly pass-1
      interior: 18k Tx4 / 23k Tx8 / 31k Tx16). A55 jellyfish −3.7%
      (grinder and confirm, spreads 0.4-0.8%), BBB −1.2/−1.4%.
      Direct simd-vs-scalar sweep test; device tests green
- [x] pass-0 loop filter simd — measured, NOT merged: transpose
      lowering is good (8/12 vzip, no vtbl; probe via asm --arch
      arm32) but Tx4 narrow is noise-level and Tx8 wide3 is a real
      loss (+0.96% jelly). Pass 0 stays scalar on in-order arm32;
      finding recorded in log + notes
- [x] end-of-campaign: margin table in log.md (A55 jelly −31.7%
      211→144 ms/f, BBB −28.8% 153→109; X4 jelly −34.1% 16.4→11.0,
      BBB −22.4% 15.1→11.8 vs campaign start 3e27aa5), `--all`
      corpus green, notes close-out, baselines pruned to
      campaign-start + final
- [x] parse→dequant fusion (deferred from P1): dequantize each
      coefficient as its token is decoded, write directly into the
      transform input; quantized i16 buffer, second scan walk, and
      dirty-prefix clear deleted. A55 BBB −4.5% (spread 0.9%) /
      jellyfish −1.5% (0.2%); X4 BBB −1.6% (credibility line),
      jellyfish noise. Compliance 307/307
- [x] branchless bool-decoder bit decision — measured null, NOT
      merged: underflow-mask rewrite achieved straight-line arm32
      asm (took four source shapes; LLVM/V8 kept re-deriving the
      branch from compare/select forms) and timed as exact noise on
      both clips stacked on the fusion (BBB −0.002%, jelly +0.3% at
      0.7% spread). The bit-decision branch was never the cost;
      branchy source kept for clarity

### A55 asm-audit backlog — candidate micro-passes (2026-07-03)

From the generated-code audit (docs/analysis/, offsets there). Ordered
by expected value; each needs the usual asm + counterbalanced bench
gate. Competes with M6 threads for effort.

- [ ] fused-dequant divide specialization: `(coef * quant) / dq_denom`
      carries a dynamic denominator into the token loop; V8 emits a
      per-nonzero-coefficient frame reload + div-by-zero and
      INT_MIN/−1 trap guards + serializing `sdiv` (verified,
      023 @ 0x1224c-0x12278). `dq_denom` is const {1,2} by tx_size —
      specialize (omit / arithmetic shift). Sits inside BBB's
      dominant region (0x10b00-0x12600 = 41% of decode_residual)
- [ ] loop_filter_is_block_edge modulo → mask test: divisors are
      8×num_8x8_{wide,high} — always powers of two — but arrive as
      table loads, so V8 emits zero-guard + `udiv` + `mls` per edge
      test (verified, 049 @ 0x2cc8-0x2d28; containing region is
      24-34% of loop_filter_frame)
- [ ] StoredModeInfo access specialization: mv candidate scan,
      loop-filter SB setup, and decode_block store path decode/encode
      the full 49-byte record where callers need a few fields (32-byte
      sub_mvs copied and dropped); specialized getters or field-split
      layout. Spread across mv_ref_candidate (81% of fn in one
      region), inter_block_mode_info, loop_filter setup, decode_block
- [ ] inverse_dct_simd_i16 size specialization: recursive schedule
      keeps dynamic brev/cos64 quadrant logic and indirect calls in
      hot code; straight-line per-n schedules trade code size (icache
      caution: monolith phase cycling is a known constraint)
- [ ] extend the narrow_i32_butterfly trust model to the remaining
      checked `narrow_i32(round2_i64(..))?` scalar ADST/shift paths
      (long adds/adc/cmn chains in transform tails)
- [ ] speculative, layout-fragile: register-pressure reduction in the
      inlined subpel convolution (coefficient Q-registers spill to
      the frame inside inner loops) and the token loop (~369 frame
      ld/st per 1728 instructions in the hot region). Prior traversal
      restructures measured negative; only attempt with asm-diff
      evidence that the spills actually leave
- Engine-level, recorded not actionable at source: redundant
      [0,255] clamp + GPR constant materialization before `vqmovun`
      (u8x16_narrow lowering), `v128.load64_zero` lowered as two
      `ldr` + lane `vmov`s, vector constant rematerialization in
      loop-filter kernels

### M5 — demo page: play + bench

Decode-and-play demo in desktop/device Chrome. Gated on M2 only; independent
of M3 and can run in parallel with optimization campaigns. Vanilla TS + DOM,
no web framework. Local dev only; hosting (VPS, COOP/COEP headers, CDN media)
is out of scope until it exists.

- [x] worker decode pipeline: vip9r in a dedicated worker (postMessage, no
      SAB/COOP/COEP needed); worker demuxes via `parseVp9Input`, constructs
      I420 `VideoFrame`s directly over wasm memory, and transfers them to
      main for canvas present; two clocks — worker-side decode ms/frame vs
      main-side presented fps and drop count — so slow blits on TV-class
      compositors can't pollute decoder numbers
- [x] playback mode: real-time pacing off IVF/WebM timestamps, credit-bounded
      frame queue (8 in flight), drop counter. 2026-07-02: jellyfish 720p30
      on the workstation: 300/300 presented @30 fps, 0 dropped, decode avg
      25.1 ms/frame
- [x] bench mode (pivoted from side-by-side race): sequential pure-decode
      benchmark as the Chrome honesty check against d8 oracle numbers. Lanes
      vip9r / WebCodecs prefer-software / prefer-hardware, run one at a time
      in a worker; per lane 60-packet untimed warmup then a timed pass over
      a packet prefix (`frames` param, default 300, 0 = whole clip). vip9r
      headline is summed `decodeNext` (same measurand as d8); VideoFrame
      construction cost measured and reported separately, not folded in.
      `lanes` param isolates one lane. hardwareAcceleration is a preference —
      unsupported configs are reported as skipped via `isConfigSupported`,
      but "software" still means "asked for software". 2026-07-02
      workstation, jellyfish 300: vip9r 25.0 ms/frame (VideoFrame +0.0),
      wc-software 0.4, wc-hardware 1.1
- [ ] spike, time-boxed: Cobalt on the streamer — can an arbitrary page load
      at all, and does it expose `VideoFrame` construction; sideloaded
      Chrome/WebView shell is the fallback device target
- Parked, explicit later decisions: optimization time-travel wasm-vintage
  selector (verify ABI stability across M3 merges first), live per-frame md5
  badge vs golden sidecars, X-ray overlay (needs decoder side-data exports —
  weigh against the no-unused-affordances rule)

### M6 — Threads (tile-parallel decode)

Scoped 2026-07-03; full evidence and mechanics in `docs/design.md` (Threads
section). Goal: close the A55 720p30 gap. Campaign-wrap numbers: jellyfish
144 ms/frame (4.3x over 33.3), BBB 109 (2.7x over 40); the post-wrap fusion
pass trimmed a few percent more, and the single-thread entropy levers are now
exhausted (fusion merged, branchless bool decision null). ffvp9 frame
threading measured ~2.9x on the A55 quad — the realistic scaling anchor. At
~2.7-3x, BBB lands under budget; jellyfish lands ~1.5x over, so a residual
gap likely survives and feeds the frame-parallel decision at the end.

Content evidence: the entire 720p perf corpus is coded with 4 tile columns
and `frame_parallel_decoding_mode=1`; youtube 720p tracks are 4 columns
(`frame_parallel=0`), 480p are 2; bear is single-tile. Tile columns
parallelize entropy decode — the cost share nothing else touches — and the
tile loop is already parallel-shaped (per-tile BoolDecoder and scratch,
tile-scoped availability/MV search, no cross-tile pixel reads).

- [ ] toolchain + ABI spike: `RUSTC_BOOTSTRAP=1` on the pinned stable
      toolchain, `-Zbuild-std=core`
      `-Zbuild-std-features=compiler-builtins-mem`, target features
      `+atomics,+bulk-memory`, link `--shared-memory --import-memory
      --max-memory=N`; JS frontends create and import the shared
      `WebAssembly.Memory`; golden stays green running single-threaded on
      the threaded build
- [ ] shadow-stack binding: every instance initializes `__stack_pointer` to
      the same linker address, so N instances over one shared memory alias
      one shadow stack. Reserve per-worker stack regions in the arena;
      export a raw `global.set __stack_pointer` shim (asm, `nostack` — it
      must not touch the stack it is replacing) called as the worker's
      first export; coordinator keeps the linker-default stack
      (`--stack-first` for overflow trapping); only the coordinator
      instance touches static `SESSION`; verify lld's `__wasm_init_memory`
      once-guard for passive segments
- [ ] worker pool runtime: JS spawns N−1 workers (d8 `Worker` / web
      `Worker`) over the shared memory; job dispatch via atomics +
      `memory.atomic.wait32`/`notify`; coordinator blocks only in its
      dedicated worker (blocking waits are illegal on the browser main
      thread; d8 allows them anywhere)
- [ ] harness: daemon pin-set support for timed kinds (e.g. `cpu:0-3`);
      driver detects shared-vs-plain memory per artifact
      (`WebAssembly.Module.imports`) so threaded candidates A/B against
      single-thread baselines under the existing counterbalanced protocol;
      characterize streamer thermals/frequency under sustained 4-core load
- [ ] tile-parallel tile decode: per-thread `SyntaxCounts` (merge after
      join; skip accumulation entirely when adaptation is off — the
      `frame_parallel=1` corpus makes it dead work even single-threaded),
      per-thread `TileModeContexts`, contained-unsafe disjoint column-band
      views of the current-frame planes and mode grid. Bit-exact by
      construction; corpus golden gates the merge as usual
- [ ] loop filter SB-row wavefront: `loop_filter_frame` is the serial
      remainder after tile parallelism (~10-19% A55); parallelize as its
      own measured pass
- [ ] demo: vite COOP/COEP headers for SAB; verify `VideoFrame`
      construction from SAB-backed views in Chrome; the Cobalt spike gains
      a SAB/cross-origin-isolation probe
- [ ] decision point: frame-parallel decode (per-row reference progress,
      per-frame state snapshots, +1 frame latency) only if the A55 gap
      survives tile + loop-filter parallelism

## M7 — Stretch

relaxed-simd · little-core (Cortex-A520)

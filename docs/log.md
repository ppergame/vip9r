# vip9r — implementation log

Human-facing narrative of decoder implementation and optimization progress.
Compact and legible. Record accepted implementation blocks, correctness
milestones, and measured optimization results.

## 2026-06-22 — VP9 packet front end

- Added the first grinder-produced decoder block: fixed-bit parsing,
  superframe splitting, and VP9 profile 0 / 8-bit uncompressed header parsing
  through `header_size_in_bytes`.
- `Decoder::decode_packet` now parses coded-frame packet structure and reference
  dimensions before stopping at the expected compressed-header/tile
  `Unimplemented` boundary.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-22 — VP9 tile payload layout

- Added allocation-free tile payload layout validation after uncompressed header
  parsing: MI bounds, raster tile descriptors, and non-final little-endian tile
  size prefixes.
- `Decoder::decode_packet` now validates all coded-frame headers and tile byte
  ranges against scratch parser state before committing persistent reference
  dimensions, then stops at the expected tile-decode `Unimplemented` boundary.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-22 — VP9 boolean decoder primitive

- Added the internal allocation-free VP9 boolean decoder primitive for future
  compressed-header and tile syntax parsing: init marker validation,
  probability-coded bools, literals, renormalization underflow checks, and exit
  padding validation.
- The primitive is intentionally not wired into packet decode yet, so decoder
  behavior is unchanged at the public boundary.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-22 — VP9 entropy header prerequisites

- Retained uncompressed-header fields needed by compressed-header parsing:
  frame-context flags/index, high-precision MV, interpolation filter,
  quantizer deltas, and `lossless`.
- The parser still does not parse compressed headers or tile contents; the
  public decode boundary remains unchanged.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-24 — VP9 intra compressed headers

- Added the first compressed-header block for key and intra-only frames:
  transform mode parsing, tx/skip/coef probability updates, subexponential
  probability remapping, and retained default frame probability state.
- `Decoder::decode_packet` now consumes valid intra compressed headers before
  validating tile layout and stopping at the expected tile-decode
  `Unimplemented` boundary. Inter compressed headers remain intentionally
  unparsed for a later block; prior tile-layout validation for inter frames is
  preserved.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-24 — VP9 intra tile mode info

- Added key/intra tile boolean syntax through the residual handoff: partition
  tree decoding, fixed key-frame partition/Y/UV probability tables, above/left
  partition and mode contexts, skip and transform-size parsing, and
  segmentation-disabled intra mode-info parsing.
- `Decoder::decode_packet` now enters key/intra tile payloads after tile-layout
  validation and stops at the expected residual `Unimplemented` boundary.
  Segmentation-enabled intra tile syntax is explicitly unimplemented rather than
  silently misparsed.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-24 — VP9 intra residual tokens

- Added parse-only residual traversal for segmentation-disabled key/intra
  frames: UV transform sizing, plane block sizing, scan selection, coefficient
  token parsing, extra coefficient bits, sign-bit consumption, and above/left
  nonzero contexts.
- `parse_intra_tiles` now consumes the full tile bool stream for the first
  `bear-vp9.ivf` key frame and returns to the packet-level
  `Unimplemented` boundary. Prediction, inverse transform, reconstruction, loop
  filter, reference storage, and output frames remain unimplemented.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-24 — VP9 inter compressed headers

- Added inter-frame compressed-header parsing through non-coef probability
  updates: inter mode, switchable interpolation filter, intra/inter, reference
  mode/reference probabilities, Y mode, partition, and MV probabilities.
- Extended frame probability state with the spec 10.5 default inter/non-coef/MV
  tables and wired `Decoder::decode_packet` to parse inter compressed headers
  before the still-unimplemented inter tile syntax boundary.
- Golden smoke on `bear-vp9.ivf` still stops at
  `decode packet 0 timestamp 0: Unimplemented`.

## 2026-06-27 — VP9 inter tile syntax

- Added parse-only inter-frame tile traversal: inter partition probabilities,
  mode-info syntax, reference-frame selection, interpolation filters, motion
  vector syntax/prediction scaffolding, intra blocks inside inter frames, and
  residual token parsing with the inter coefficient context.
- `Decoder::decode_coded_frame` now parses the first `bear-vp9.ivf` key frame
  and first inter frame tile syntax before stopping at the next honest boundary:
  non-frame-parallel inter probability adaptation needs syntax counts. Pixel
  prediction, reconstruction, loop filtering, reference pixels, and shown-frame
  output remain unwired.
- Golden smoke and strict runs on `bear-vp9.ivf` now stop at
  `decode packet 1 coded frame 0: Unimplemented`.

## 2026-06-27 — VP9 syntax counts and probability refresh

- Added syntax count accumulation for the parse-only tile path and VP9 backward
  probability refresh for coefficient, non-coefficient, and MV probability
  state.
- `Decoder::decode_coded_frame` now removes the prior non-frame-parallel inter
  adaptation `Unimplemented` boundary, tracks previous frame type for the
  coefficient update factor, and refreshes frame contexts after tile parsing.
- Golden smoke on `bear-vp9.ivf` advances past packet 1 and now stops at
  `decode packet 17 coded frame 0: InvalidBitstream`.

## 2026-06-27 — VP9 previous-frame MV candidates

- Added previous-frame mode/MV history for the host tile parser and wired
  `UsePrevFrameMvs` into inter-frame MV reference discovery.
- `Decoder::decode_coded_frame` now records the current frame's per-MI
  reference/MV state after successful tile parsing and supplies it to the next
  eligible inter frame.
- Golden smoke on `bear-vp9.ivf` advances past packet 17 and now stops at
  `decode packet 25 coded frame 0: InvalidBitstream`.

## 2026-06-27 — VP9 exact mode history for MV refs

- Replaced the 1-D above/left-context approximation for current-frame MV
  reference candidates with exact per-MI mode-history lookup when that grid is
  available, and retained sub-block MVs for sub-8x8 candidate extraction.
- Host golden smoke on `bear-vp9.ivf` now parses all 82 coded frames. It exits
  at the next expected M1 boundary: no shown frames are output yet, so the
  harness reports 82 missing frames. Wasm/no-std parity still waits on moving
  mode-history storage into workspace state.

## 2026-06-27 — VP9 mode history in workspace

- Moved previous/current frame mode-history storage out of std-only `Vec`s and
  into `DecodeWorkspace` as two packed byte slots, so host and no-std/wasm use
  the same previous-frame MV candidate path.
- Reduced tile above-context storage to a column cap sized for project targets
  instead of 8192 MI columns, fixing a wasm stack trap in the d8 smoke path.
- Host and wasm smoke runs on `bear-vp9.ivf` now both parse all 82 coded frames
  and exit at the expected M1 boundary: no shown frames are output yet, so both
  report 82 missing frames.

## 2026-06-27 — VP9 shaped frame output plumbing

- Added neutral I420 output plumbing backed by the existing workspace frame
  pool: current-frame fill, reference-slot refresh/copy, `show_existing_frame`
  output, and visible plane descriptors for host and wasm consumers.
- Host and wasm smoke runs on `bear-vp9.ivf` now emit all 82 shown frames.
  `--allow-mismatch` passes with 82 md5 mismatches and no missing/extra frames;
  strict md5 still fails because prediction, inverse transform, reconstruction,
  and loop filtering remain unimplemented.

## 2026-06-27 — VP9 residual coefficient/dequant model

- Added a transform-block residual data model that stores signed quantized
  coefficients in raster position order, retains transform metadata, and
  dequantizes profile 0 / 8-bit blocks with spec-derived DC/AC quant tables.
- `TileParser::tokens` now captures coefficient magnitudes/signs and
  `decode_residual` dequantizes each parsed non-skipped transform block before
  dropping it at the next M1 boundary. Neutral frame output remains intentionally
  md5-wrong until inverse transforms, prediction, reconstruction, and loop
  filtering land.
- Smoke still passes with 82 md5 mismatches; strict still fails with 82 md5
  mismatches and no missing/extra frames.

## 2026-06-27 — VP9 inverse transform kernels

- Added no-allocation inverse transform support for the profile 0 / 8-bit
  residual path: 4/8/16/32 IDCT, 4/8/16 IADST, and lossless 4x4 IWHT.
- `decode_residual` now dequantizes and inverse-transforms each parsed
  non-skipped transform block before dropping the local block result. Neutral
  frame output remains intentionally md5-wrong until prediction,
  reconstruction, inter reference pixels, and loop filtering land.
- Smoke still passes with 82 md5 mismatches; strict still fails with 82 md5
  mismatches and no missing/extra frames.

## 2026-06-27 — VP9 intra prediction and reconstruction

- Added current-frame intra prediction and reconstruction for profile 0 / 8-bit
  blocks: default current-frame initialization, mutable current-frame tile
  plumbing, all ten VP9 intra predictors, skipped-block prediction, and
  inverse-transformed residual add/clip for intra-coded transform blocks.
- Inter-coded blocks intentionally remain default-filled until the inter
  prediction/reference-sampling block lands, so full-frame md5 remains wrong.
- Host and wasm smoke pass with 82 shown frames and md5 mismatches only; strict
  host golden still fails with 82 md5 mismatches and no missing/extra frames.

## 2026-06-27 — VP9 inter prediction from references

- Added profile 0 / 8-bit inter prediction from workspace reference slots:
  logical LAST/GOLDEN/ALTREF mapping through `ref_frame_idx`, read-only
  reference-plane views, luma/chroma MV selection, MV clamping/scaling,
  separable subpel filtering, compound averaging, and prediction before
  residual reconstruction.
- Inter-coded blocks now contribute reference-derived pixels instead of the
  previous neutral fill. Loop filtering remains the final M1 pixel-path block,
  so reference/output frames are still intentionally unfiltered.
- Host and wasm smoke pass with 82 shown frames and md5 mismatches only; strict
  host golden still fails with 82 md5 mismatches and no missing/extra frames.

## 2026-06-27 — VP9 loop filter and first strict green vector

- Added profile 0 / 8-bit loop-filter header state, per-MI filter metadata, and
  the final in-loop filter pass before reference refresh/output.
- `bear-vp9.ivf` strict md5 now passes on both the host golden harness and the
  d8 wasm driver: 82 matched, 0 mismatched, 0 missing, 0 extra.
- M1 is code-complete for the current supported subset. M2 bring-up now moves to
  conformance vectors and corpus clips; segmentation remains outside the
  implemented tile subset and is still rejected before filtering.

## 2026-06-28 — VP9 MI-rounded reconstruction extents

- Changed the workspace frame-pool/current-frame view to reconstruct over the
  spec MI-rounded luma extent (`MiCols*8` by `MiRows*8`) and matching chroma
  extent, while keeping public I420 output and reference views clipped to the
  visible decoded frame dimensions.
- Strict wasm-golden now passes on the resize vectors that previously failed:
  `vp90-2-05-resize.ivf` is 10 matched / 0 mismatched, and
  `vp90-2-18-resize.ivf` is 100 matched / 0 mismatched. `bear-vp9.ivf` remains
  green at 82 matched / 0 mismatched.

## 2026-06-28 — VP9 tile-size byte order

- Fixed non-final VP9 tile-size parsing to treat the byte-aligned `f(32)` field
  as MSB-first instead of little-endian.
- `vp90-2-09-subpixel-00.ivf` now passes strict wasm-golden at 20 matched /
  0 mismatched. The sorted md5-backed corpus sweep then advances through 221
  media files and stops at the next frontier, `vp90-2-09-aq2.webm`, which
  returns `Unimplemented` at packet 0 coded frame 0 on the segmentation-enabled
  tile path.

## 2026-06-28 — VP9 segmentation map and ALT_Q

- Added persistent segmentation header state, segment-id tile syntax for
  intra/inter frames, temporal segment prediction contexts, ALT_Q per-segment
  dequantization, ALT_L loop-filter level adjustment, and REF_FRAME/SKIP
  syntax hooks.
- `vp90-2-09-aq2.webm` now passes strict wasm-golden at 100 matched /
  0 mismatched. The sorted md5-backed corpus sweep then advances through 232
  media files and stops at `vp90-2-13-largescaling.webm`, which returns
  `InvalidBitstream` at packet 0 coded frame 0. The next hypothesis is the
  current fixed 512-MI-column tile-context cap vs this vector's 19200px+ output
  frames.

## 2026-06-28 — VP9 large-scaling tile contexts

- Raised fixed tile above-context storage from 512 to 2560 MI columns, enough
  for `vp90-2-13-largescaling.webm`'s 20400px-wide frame after 64x64 partition
  rounding. Wider fixed-context overflow now reports `ResourceLimit` instead of
  `InvalidBitstream`.
- `vp90-2-13-largescaling.webm` now passes strict wasm-golden at 2 matched /
  0 mismatched. The sorted md5-backed corpus sweep advances through 275 media
  files and stops at `vp90-2-19-skip-02.webm`, which returns
  `InvalidBitstream` at packet 5 coded frame 0.

## 2026-06-28 — VP9 persistent segment map

- Separated current-frame `segment_id` from the saved `PrevSegmentIds` entry in
  packed mode history, so frames with segmentation disabled or
  `segmentation_update_map == false` preserve the prior segment map instead of
  overwriting it with current syntax IDs.
- `vp90-2-19-skip-02.webm` now passes strict wasm-golden at 12 matched /
  0 mismatched. The sorted md5-backed corpus sweep advances through 304 media
  files and stops at `vp90-2-22-svc_1280x720_3.ivf`, where the golden runner
  rejects IVF header dimensions `0x0` before decode.

## 2026-06-28 — VP9 zero-dimension SVC IVF harnessing

- Changed the wasm golden runner to allow IVF header dimensions `0x0` through
  demux metadata and derive decoder limits from dimensioned md5 sidecar frame
  names when the container cannot provide them.
- Added a narrow zero-dimension IVF comparison rule for SVC vectors whose md5
  sidecar covers only the top spatial layer: decoded outputs with dimensions
  absent from the sidecar are counted and skipped before positional md5
  comparison. The rule is not applied globally because some WebM sidecar
  filename dimensions are not the decoded frame dimensions.
- `vp90-2-22-svc_1280x720_3.ivf` now passes strict wasm-golden at 20 matched /
  0 mismatched after decoding 60 shown outputs and skipping 40 lower-layer
  outputs. `vp90-2-15-segkey_adpq.webm` remains green at 150 matched /
  0 mismatched, guarding against overbroad dimension filtering.

## 2026-06-28 — VP9 md5 corpus complete

- Added optional `wasm-golden --progress-frames=N` output for long corpus runs.
  Default golden output and pass/fail criteria are unchanged.
- Strict wasm-golden now passes every md5-backed media file currently present in
  `/bulk/vip9r`: 330/330 total, with no mismatched, missing, or extra shown
  frames. The set covers `chromium/bear-vp9.ivf`, 323 libvpx conformance/perf
  vectors and clips, and six realworld WebM clips.
- The final parallel validation batches completed the remaining TOS tail and
  realworld clips without decoder changes. Notable slow paths were
  `vp90-2-tos_1920x800_tile_1x4_fpm_2335kbps.webm` at 17,620 matched frames and
  `realworld/wikimedia/spring-original-2048x858p24-1_41mbps.webm` at 11,138
  matched frames.

## 2026-07-02 — M3 pre-optimization timing baselines

- Recorded full-decode wasm timings on `bear-vp9.ivf` (320x240) across all
  three perf targets before any optimization work. Same 16-output-frame window
  (`--frames 0:15`), release wasm, md5-validated, cool start, flat per-pass
  timings on all targets:
  - host workstation: 6.9 ms/frame
  - Pixel 9a Cortex-X4 (pinned cpu7, 3.105 GHz): 11.2 ms/frame
  - TV Streamer Cortex-A55 (pinned, 2.0 GHz): 129.7 ms/frame
- The window is small because the A55 cannot finish the full 82-frame bear
  validation pass inside the bench runner's 5 s per-pass deadline (26 frames
  in 5.1 s). Host and X4 full-clip runs measure 7.3 and 12.0 ms/frame.
- 720p per-core starting-line numbers against the 33.3 ms/frame 720p30 budget
  are in `docs/design.md`: X4 ~147, A720 ~207, A520 ~845, A55 ~1489 ms/frame.
## 2026-07-02 — M3 attribution refresh and simd128 baseline

- Fixed device profile reports: `simpleperf report` on device attributes JIT
  samples to stale file mappings (d8 mmaps input files, munmap is never
  recorded, V8 JITs into the freed ranges). The daemon now attributes raw
  `perf.data` sample IPs on the host with the V8 perf map taking precedence.
  The old reports had hidden a 25% `loop_filter_frame` share entirely.
- Refreshed 720p hotspot attribution on `jellyfish-720p30` (144.5 ms/frame on
  the X4, matching the starting line):
  - X4: `predict_inter` 51%, `loop_filter_frame` 25%, `decode_block` 10%,
    `inverse_dct` 4%, `residual::b` 3%
  - A55: `loop_filter_frame` 27%, `predict_inter` 25%, `decode_block` 16%,
    `inverse_dct` 6% (2-output-frame window, keyframe-weighted; d8/libc native
    overhead is ~16% on arm32 vs ~3% on arm64)
- Measured the bare `+simd128` flag flip (autovectorization only): X4
  145.4 → 147.0 ms/frame (min-pass +1.7%, below the 2% credibility line), A55
  1473.4 → 1473.5 (dead even). No autovectorization win on the spec-literal
  loops, as expected. The flag is now part of the baseline
  (`rust/.cargo/config.toml`) so later hand-written kernel deltas are not
  confounded with the flag; compliance corpus green with the flag (307/307).

## 2026-07-02 — VP9 inter prediction two-pass convolution

- Reshaped unscaled inter prediction from per-sample nested 8-tap filtering
  (64 MACs/pixel with per-sample clamped access and checked math) into a
  block-level two-pass separable convolution: constant per-block filter
  phase, horizontal pass into a local intermediate buffer, straight-line
  vertical pass, integer-MV block copy, and edge clamping hoisted to a
  per-block clamped gather. Scaled references keep the spec-literal
  per-sample path. Output is bit-exact (u8 intermediate math composition
  unchanged).
- Measured on `jellyfish-720p30` frames 0:15, Pixel 9a X4: 143.6 → 77.3
  ms/frame (-46%). Compliance corpus green (307/307, host wall time 61 → 31 s).

## 2026-07-02 — VP9 loop filter precompute and span processing

- Reshaped the loop filter: per-frame strength LUTs (64 levels plus
  segment/ref/mode lookup), a per-superblock MI decision cache, span
  processing of up to 8 filter positions sharing one MI's decisions, and a
  direct-index interior filter path with the clamped per-sample path kept as
  the frame-edge fallback. The wide filter's per-output window sum became a
  bit-identical sliding sum.
- Filter arithmetic is unchanged; all mask/strength/edge special cases
  (sharpness, ALT_L segments, odd-chroma guards) preserved and covered by
  targeted goldens plus the compliance corpus (307/307, host wall 31 → 17 s).
- Measured on `jellyfish-720p30` frames 0:15, Pixel 9a X4: 76.2 → 43.5
  ms/frame (-43%); `loop_filter_frame` fell from 47% to ~7% of decode
  cycles. First grinder attempt was lost to the socket-inode harness fault
  fixed earlier this session; its unvalidated tree was salvaged, validated,
  and refined by the relaunched session.

## 2026-07-02 — VP9 IDCT eob fast paths

- Added dequantization-time nonzero-row tracking and first-pass zero-row
  skipping (a 1-D transform of a zero row is exactly zero), plus a DC-only
  DCT_DCT fast path pinned to the general 2-D pipeline by a unit test across
  clamp-range DC values.
- Removed per-op checked narrowing from the hot `b()`/`h()` butterflies: the
  spec makes 32-bit intermediate representability a conformance requirement,
  so conformant output is unchanged; malformed streams that overflow now
  produce wrong pixels instead of a decode error (documented in code).
  Products stay i64 for exactness.
- Measured on `jellyfish-720p30` frames 0:15, Pixel 9a X4: 42.6 → 24.2
  ms/frame (-43%). Compliance corpus green (307/307). The X4 is now under
  the 33.3 ms/frame 720p30 budget on this clip.

## 2026-07-02 — bool decoder wide window: measured negative

- Rebuilt `BoolDecoder` around a 32-bit combined value/window with
  byte-granularity refill and clz renormalization (several variants: u64/u16
  side windows, branchless split, out-of-line refill, literal(1) fast path).
  All validated bit-exact, none beat the existing 16-bit bit-at-a-time
  decoder on the X4: net effect ≈ +0.4% after subtracting position bias.
  Not merged. Under V8/TurboFan the per-bit refill is not the bottleneck;
  `decode_block`'s cost sits in the token/syntax structure above the
  primitive.
- Measurement caveat discovered on the way: with the device heat-soaked by
  hours of continuous back-to-back runs, the A/B bench's second position
  reads ~13% slow (a no-op candidate reproduced it). Under normal cool-start
  conditions the position penalty is ~2%. Sub-2% deltas remain
  non-credible, and orchestrator-side controls (no-op candidate) are the
  cheap way to re-anchor when the device has been under sustained load.

## 2026-07-02 — coefficient token loop hoisting

- Hoisted block-invariant lookups out of the coefficient token loop: the
  probability/counts subarrays (previously a 6-level index chain re-derived
  per bool read), the scan and band tables (now per-size const slices), and
  the DC context. The token tree walk moved to a const-shaped
  `TokenTreeBranch` table, dropping per-node bounds checks and error
  mapping. Counts and syntax are bit-identical; adaptation is unaffected.
- Grinder-measured against same-position no-op controls: ~1.7-2.0% faster
  full decode on the X4 — at the measurement credibility threshold, kept
  for the hot-loop simplification as much as the delta. Rejected variants
  that measured worse: fully unrolled token tree, precomputed
  neighbor-context tables, direct coefficient writes, small-token coef
  fast path. Compliance corpus green (307/307).

## 2026-07-02 — simd128 subpel convolution + harness cargo-config fix

- First kernel of the simd128 campaign: the 8-tap subpel convolution
  (h-pass to-buffer and in-place, v-pass Store/Average, phase-0 Average
  rows) now runs on `core::arch::wasm32` lanes — per-tap u8→i32 widening
  multiply-accumulate, `(sum+64)>>7` on i32 lanes, saturating narrow
  reproducing `clip1`/`avg2` bit-exactly. Scalar kernels retained as the
  test reference and odd-width tails. An exhaustive wasm unit sweep (3
  filter banks × 16×16 phase pairs × widths 4-64 × Store/Average × interior
  and edge-clamped gathers) checks simd-vs-scalar equality on device and
  host; compliance corpus green (307/307).
- Measured with position-corrected A/B vs the pre-simd baseline,
  `jellyfish-720p30`: Pixel 9a X4 frames 0:15 −16.7% (23.8 → 20.1
  ms/frame, spread 2.8%); A55 streamer frames 0:5 −14.5% (272 → 233
  ms/frame, spread 0.2%). Grinder cross-check on `big-buck-bunny-720p25`
  −19.0%.
- Merge validation initially measured the "same" build 2.8x *slower* —
  root cause: cargo resolves `.cargo/config.toml` from the invoking cwd,
  not `--manifest-path`, so harness builds launched outside `rust/`
  silently dropped `target-feature=+simd128` and compiled every simd
  intrinsic as an outlined call. All harness cargo invocations
  (`vip9r-perf-submit`, corpus runner, wasm-tools wrappers) now pin cwd to
  the workspace. Grinder sandboxes were unaffected (they build from
  `/run/rust`), which is why the grinder's numbers were right all along.

## 2026-07-02 — loop filter simd128: measured negative

- Vectorized the span loop filter three ways: full simd128 for both passes
  (lane masks for hev/mask/flat/flat2, i8-saturating `filter4_clamp`,
  bitselect blends, load+transpose tiles for vertical edges), a
  pass-1-only variant, and a final narrowed pass-1 Tx4x4-narrow-only
  variant with the simd probe gated out of larger-filter dispatch. All
  bit-exact (85/85 wasm tests incl. a new segment sweep vs the scalar
  reference across passes, lengths 1..8, sizes, and mask-diverse data).
  Best case measured −0.6% (jellyfish) / −0.7% (big-buck-bunny) on the X4
  against spreads of 1.1%/0.8% — inside noise; broader variants were
  outright slower. Not merged.
- Why it doesn't pay here: the scalar reshape already amortized decisions
  over ≤8-position spans, so the simd upside is only the filter
  arithmetic; pass-0 (vertical edge) needs a gather/transpose per span,
  and wide filters need many blended variants, both of which cost more
  than the lanes save under V8. Candidate for a relaxed-simd or
  multi-thread era retry.

## 2026-07-02 — simd128 inverse DCT

- DCT_DCT 2-D inverse transforms now run 4 lanes wide: the row pass
  compacts set bits of `nonzero_row_mask` into groups of 4 (leftovers
  scalar), the column pass takes 4 contiguous columns per step, and the
  butterfly network (`b`/`h`) is a structural clone of the scalar
  recursion on `i32x4` state — `i64x2_extmul` products, `(+8192)>>14`
  rounding in i64 lanes, low-32 packing reproducing the scalar wrapping
  narrow bit-exactly. ADST/WHT stay scalar (~1.3% share didn't justify
  lanes). Sparse fast paths (eob 0/1, zero-row skip) preserved.
- The delta beats the profiler's 6-7% `inverse_dct` share because the 2-D
  driver's strided gather/scatter and final i64 rounding were inlined
  into `decode_block`'s attribution; the simd path captures those too.
- Measured vs post-convolution baseline, jellyfish: X4 0:15 −13.0% (20.1
  → 17.5 ms/frame, spread 1.1%); A55 0:5 −9.6% (233 → 211, spread 0.2%).
  Grinder cross-check big-buck-bunny −5.1%. simd-vs-scalar wasm test
  sweep across sizes/types/sparsity patterns; compliance corpus 307/307.

## 2026-07-02 — reference slot remap

- Reference refresh is now a slot-table write instead of a full-frame
  copy: the frame pool is 9 equal physical buffers plus a decoder-owned
  map of {current, ref 0..7} → buffer. 9 buffers are necessary (8 live
  refs + writable current) and sufficient (refreshing current into a ref
  frees that ref's old buffer). Refs may alias one buffer; current is
  never referenced by construction, and the ref table only advances after
  a successful decode. This also drops the old "current precedes all
  references" layout constraint.
- Measured on the A55 (the memory-bound target): −0.40% jellyfish /
  −0.33% big-buck-bunny with no-op anchors of similar magnitude — at the
  noise line, as the tracker's ~1-2% X4 headroom estimate predicted. A
  ~1.4 MB memcpy per refreshed slot is simply small against a 211 ms A55
  frame. Merged anyway: consistent positive direction on both clips, and
  the indirection is the structure the threads milestone needs (no copy
  serialization), with alias/free-selection unit tests and the
  resize/svc/skip vectors green. Compliance corpus 307/307.

## 2026-07-02 — simd128 campaign wrap: both-device margins

Whole-campaign position-corrected A/B, final tree vs the pre-simd scalar
baseline (49bac22), X4 = Pixel 9a cpu7, A55 = TV streamer cpu0:

| device | clip                  | before → after ms/frame | delta  | budget    | margin      |
|--------|-----------------------|-------------------------|--------|-----------|-------------|
| X4     | jellyfish-720p30 0:15 | 24.2 → 17.1             | −29.5% | 33.3 ms   | 1.95x under |
| X4     | big-buck-bunny 0:15   | 20.0 → 15.2             | −24.3% | 40 ms @25 | 2.6x under  |
| A55    | jellyfish-720p30 0:5  | 272 → 210               | −22.8% | 33.3 ms   | 6.3x over   |
| A55    | big-buck-bunny 0:5    | 168 → 152.5             | −9.2%  | 40 ms @25 | 3.8x over   |

- Campaign ledger: subpel convolution simd merged (−16.7% X4), loop
  filter simd measured negative and not merged, inverse DCT simd merged
  (−13.0% X4), reference slot remap merged at the noise line for
  structure. Compliance corpus green at every merge; `--all` green at
  every pass boundary (337/337).
- Post-campaign X4 profile (jellyfish): decode_block 38% / loop_filter
  20% / subpel 17% / IDCT-simd 4.3% / libc 5%. The residual is dominated
  by serial entropy decode — not simd-shaped; a decode_block structural
  reshape is the next distinct campaign if the X4 margin needs to grow.
- A55 residue for the threads/scope decision (M6): perfect 4-core scaling
  would put jellyfish at ~52 ms/frame — still 1.6x over the 720p30
  budget — and BBB-class content just under its 25 fps budget. Threads
  alone cannot close jellyfish-class 720p30 on the streamer; that bounds
  what the deferred ffvp9 baseline needs to answer (whether *any*
  single-device software decode fits the streamer, or the A55 target
  moves to M6 stretch scope).

## 2026-07-02 — ffvp9 baseline: both devices, all core types

Static NEON ffmpeg (ffvp9 8.1) run directly through adb by the new
`scripts/ffvp9-baseline.py` (armv7 static build added to the flake next to
the existing arm64/linux64). 600-frame windows, taskset-pinned, best-of-2,
ffmpeg `-benchmark` rtime, so numbers include process startup and demux.
Full matrix with per-run telemetry:
`temp/perf/ffvp9-baseline-20260702T222432Z.json`. ms/frame:

| clip (720p unless noted) | A55 x1t | A55 x4t | A520 x1t | A720 x1t | X4 x1t |
|--------------------------|---------|---------|----------|----------|--------|
| jellyfish p30            | 23.1    | 7.9     | 13.3     | 4.4      | 2.7    |
| big-buck-bunny p25       | 17.5    | 5.8     | 9.6      | 3.2      | 2.1    |
| caminandes p24           | 22.9    | 8.4     | 13.2     | 4.5      | 2.9    |
| cosmos-laundromat p24    | 9.5     | 3.4     | 4.6      | 1.6      | 1.0    |
| spring 2048×858 p24      | 27.2    | 9.7     | 14.3     | 4.7      | 2.8    |
| tears-of-steel p24       | 15.0    | 5.4     | 8.2      | 2.8      | 1.7    |

- The M6 scope question is answered: ffvp9 decodes every clip inside its
  realtime budget **single-threaded on the A55** (worst 720p30 clip 23.1
  vs 33.3 ms, ~30% margin, cool start). Single-device software decode
  fits the streamer without threads; the A55 target is an optimization
  gap, not a hardware bound.
- Gap to vip9r post-simd128: A55 jellyfish 210 vs 23.1 ms/frame (~9x),
  X4 17.1 vs 2.7 (~6.4x). Part of that is native NEON vs V8 wasm
  codegen; the rest is decoder maturity.
- Frame threading (`-threads N`) scales ~2.9x on the homogeneous A55
  quad. On the Pixel's heterogeneous clusters multithreaded numbers
  mislead — A720 x3t is slower than x1t and all-cores x8t is gated by
  the little cores — so the single-core columns are the meaningful
  bound there.
- Host (x86-64, `-threads 1`): 0.7–2.1 ms/frame, collected under
  concurrent build load; rough numbers only.

## 2026-07-02 — A55 campaign: measurement-window profiling + residual path restructure

A55-first optimization campaign opened (streamer is the decision device;
X4 confirmation secondary; changes stay portable-V8-principled).

- Harness first: device profile reports are now windowed to the bench
  driver's measurement passes (daemon filters samples by time against
  `measurement.elapsedMs`). The md5-validation/warmup/startup head was
  ~40% of samples and had inflated the JS/d8/libc rows; the suspected
  "hot d8 memory.copy builtin" from the exploration notes dissolved
  under scrutiny (it was `Builtin:DoubleToI` from validation md5,
  double-counted by an offline histogram). Real V8-runtime cost during
  decode is 1–2%. Clean A55 measurement-only shares — jellyfish:
  decode_block 38 / subpel 18 / loop_filter 15 / libc 11; BBB:
  decode_block 50 / libc 20 / loop_filter 7 / subpel 6.
- Residual path restructure (decode_block's data churn): parser-owned
  persistent coefficient/dequant/token-cache buffers with
  scan-prefix-scoped clears, dequantization bounded by eob in scan
  order, eob==0 blocks skip dequant/IDCT/reconstruct entirely, no more
  KB-sized by-value struct returns; plus shift/mask in
  `coefficient_token_context` (real udivs on arm32). A stale-workspace
  regression test pins the clear discipline.
- Measured on the A55: **−16.1% big-buck-bunny** (residual-heavy clip),
  **−14.2% jellyfish** (spreads 0.5/0.3%). Host validations across
  quantizer extremes/aq2/tiny-frame vectors and 88/88 wasm tests green;
  compliance corpus 307/307.

## 2026-07-03 — A55 campaign: traversal negative, residual add simd

- Post-P1 re-attribution with an `#[inline(never)]`-instrumented device
  profile split the decode_block monolith: on big-buck-bunny the entropy
  token loop is 18%, the residual traversal glue 16%, intra prediction
  10%, the residual add 4.3%, dequantize 2.4%.
- Traversal glue restructure measured **negative and was not merged**:
  three correct hoist/slice variants regressed the A55 +1.6..3.1%, and a
  skip-block context-fill fast path alone was exactly noise. TurboFan
  already elides the checked-arithmetic ceremony the profile appeared to
  charge; that 16% is intrinsic loop work and code layout, not removable
  branches. Lesson recorded: decode_block-region layout is fragile,
  scalar "cleanup" passes there are not worth grinder time.
- `add_residual_block` interior simd merged: rows fully inside the
  visible plane take a widen→add→two-stage-saturating-narrow v128 path
  (7 ALU ops + 3 loads + 1 store per 8 pixels; 4-wide rows use 32-bit
  lanes); frame-edge overhang keeps the checked scalar path. Saturating
  u8 narrow reproduces scalar `clip1` bit-exactly, pinned by a
  simd-vs-scalar sweep over tx sizes/overhangs/extreme residuals.
  Measured on the A55: **−2.6% big-buck-bunny, −3.4% jellyfish**
  (spreads ~0.4%). Compliance corpus 307/307.

## 2026-07-03 — A55 campaign: convolution v2, interp buffer de-uninit

- Subpel convolution v2 merged: MAC helpers moved to
  `i32x4_extmul_low/high_i16x8` and the per-block 5041-byte interp
  buffer zero-fill removed. **−1.5% jellyfish, −0.5% BBB** on the A55 —
  the memset removal is most of the win. The grinder produced asm-dump
  evidence for three negatives worth remembering: V8 arm32 lowers
  `i8x16_shuffle` to VTBL (not VEXT), so shuffle-based tap gather is
  dead on arrival; extmul does not fuse with add into VMLAL (lowers as
  vmovl+vmul+add); a vertical sliding-register window spills under V8's
  regalloc. Pure-i16 accumulation is impossible for the 8-tap filters
  (worst-case coefficient sum × 255 exceeds i16).
- The zero-fill removal initially shipped as a `MaybeUninit` buffer
  (~30 `assume_init` sites, duplicated `*_to_uninit` helper twins). A
  follow-up refactor replaced it with a parser-owned persistent
  `[u8; MAX_INTERP_BUFFER]` — same ResidualBuffers ownership pattern,
  stale-read violations become deterministic md5 bugs instead of UB.
  Zero unsafe remains in the interp path; the simd/scalar helper twins
  were unified under a `const USE_SIMD: bool` generic (net −342 lines).
  Gate: perf-neutral on both clips (jellyfish −0.27% inside a 0.54%
  spread, BBB +0.01%), 89/89 tests host+device, compliance 307/307.

## 2026-07-03 — A55 campaign: intra prediction fast paths

- Intra prediction got the residual-path treatment: parser-owned
  persistent pred buffer (drops a 1KB zero-fill per predicted tx-block),
  interior write-out as row slices with fixed-width 4/8/16/32-byte
  stores instead of per-pixel checked `set_visible`, and row-oriented
  kernels for the common modes — DC/V/H as fills/row stores, TM as
  widened i16 add + saturating u8 narrow (clip1-exact, same trick as
  add_residual). Fully-interior DC/V/H/TM blocks predict directly into
  the plane and skip the pred buffer entirely; directional modes and
  frame-edge blocks keep the scalar reference path, which the sweep
  tests pin against.
- Measured on the A55: **−2.6% BBB, −3.1% jellyfish** against a −0.7%
  no-op anchor. The A/B split shows the buffer + chunked write-out
  carries nearly all of the BBB win; the mode kernels add ~1% more on
  jellyfish. Compliance corpus 307/307, 91/91 tests host+device.

## 2026-07-03 — A55 campaign: loop filter simd + a V8 arm32 codegen bug

- Post-P-intra profiles put `loop_filter_frame` at 18.9% (jellyfish) /
  8.6% (BBB), the largest discrete target left. The retry avoided the
  X4 attempt's failure mode by construction: pass-1 (horizontal edges)
  filters adjacent columns with shared decisions, so the Tx4x4 narrow
  filter vectorizes over 8 u8 lanes with zero transposes; pass 0 and
  the wide filters stay scalar. Vector decision prechecks alone
  measured *positive* (slower) on both clips and were dropped.
- The grinder's 8-lane kernel passed the sweep test on host but failed
  on device, and it shipped a half-width 4+4 workaround that gave up
  most of the win after blaming i16 shifts and high-half extends. An
  orchestrator repro proved those ops correct on device; a lane-uniform
  bisect of the full kernel then produced the fingerprint: output lanes
  4..7 corrupted to 128+{16,32,64,128} — the powers-of-two lane
  constant that **V8's arm32 `i16x8_bitmask` lowering materializes,
  leaking into the aliased high D-half of a live Q register under
  register pressure**. Same wasm is correct on x64. Replacing the
  early-out with `v128_any_true` (no lane constant, and semantically
  what the check wants) fixes the kernel at full width; the noted
  constraint lives next to the code.
- Merged the corrected 8-lane kernel: A55 jellyfish −2.1% (grinder
  measurement of the equivalent kernel) with −1.5%/−0.6% confirm runs
  at ~0.7-1.0% spreads, BBB neutral. Compliance 307/307, 93/93 tests
  host+device, validates incl. tile-4x1 and 66x66. Portable-simd
  lesson for the campaign: prefer `v128_any_true`/`v128_all_true` over
  `*_bitmask` for emptiness checks in register-heavy arm32 kernels.

## 2026-07-03 — A55 campaign: persistent intra edges

- The post-P-intra profile put libc at 6.2% of BBB decode (~62% of it
  memcpy), and a wasm dump attributed a per-intra-block **96-byte
  `memory.copy`** to `intra_prediction_edges` returning its edge
  struct by value, plus the `[127; _]`/`[129; _]` init fills. Fix:
  `IntraPredictionEdges` becomes a persistent field on the
  parser-owned intra buffers, filled in place; missing-edge defaults
  (127/129) are written explicitly only over the `size`/`2·size`
  spans that consumers read, and the `have_above` interior gather
  copies the contiguous plane row as fixed-width 4/8/16/32/64-byte
  chunks (per-sample clamped path kept for the frame-right overhang
  and the `not_on_right && Tx4x4` extension). Both the copy and the
  fills verified gone from the release wasm dump.
- A55 BBB −3.6% on the 1080p libvpx clip (0.3% spread), −1.2% on the
  canonical 720p wikimedia clip (0.3% spread); jellyfish noise-level,
  as expected for an inter-heavy clip. Compliance 307/307, 93/93 tests
  host+device, validates incl. quantizer-00 (all-intra, hammers the
  missing-edge defaults), size-18x34, resize.
- Side finding from the same dump: the remaining per-block 512-byte
  copy+fill sites are simd inverse-DCT scratch — the zero-init of
  `[v128; MAX_TX_WIDTH]` and `inverse_dct_permutation_simd`'s
  `let copy_t = *t`. Queued as a candidate follow-up.

## 2026-07-03 — A55 campaign: inverse-DCT scratch elimination

- Follow-up on the edges pass's side finding: the simd DCT_DCT path
  paid ~1KB of dead stack traffic per 4-lane group — a 512B
  `memory.fill` (zero-init of `[i32x4_splat(0); MAX_TX_WIDTH]` in the
  row-group and column kernels) and a 512B `memory.copy`
  (`inverse_dct_permutation_simd`'s `let copy_t = *t`). Fix: the
  gather loops write directly into bit-reversed slots
  (`t[brev(n, col)] = …`), which is exactly gather-then-permute since
  `brev(n, ·)` is a bijection on `0..width`, so the permutation pass
  disappears; the scratch array becomes a persistent field on
  `DequantizedCoefficients` (zeros were never read — every entry the
  DCT touches is overwritten each group). The scalar
  `inverse_dct_permutation` becomes in-place disjoint swaps (brev is
  an involution), with a unit test against the copy-based reference.
- All four 512B fill/copy sites verified gone from the release wasm
  dump; decode_residual memory ops dropped 18 → 7. A55: BBB **−5.1%**
  grinder / **−4.4%** orchestrator confirm (spreads 0.3-1.0%, no-op
  anchor null), jellyfish **−1.1%** (spread 0.2%). Largest single win
  since P1 — on an in-order core with per-access wasm bounds checks,
  libc round-trips inside the hottest loop were pure tax. Compliance
  307/307, 94/94 tests host+device, validates incl. quantizer-00 and
  resize.

## 2026-07-03 — A55 campaign: wide loop filter simd

- Refreshed profiles showed `loop_filter_frame` still at 18.4% of
  jellyfish after the narrow kernel — the P8 pass only covered
  pass-1 Tx4x4, and a segment-mix histogram showed Tx8x8/Tx16x16
  edges outnumber Tx4x4 nearly 3:1 on jellyfish. Extended the pass-1
  len==8 dispatch with a Tx8x8 kernel (narrow + wide3 regimes) and a
  Tx16x16/Tx32x32 kernel (narrow + wide3 + wide4), same transpose-free
  8-column structure: masks/hev/flat/flat2 in i16 lanes, wide
  smoothing as sliding-window sums (peak 4088 < i16::MAX, no widening
  needed), disjoint per-lane regime masks blended with bitselect so
  untouched lanes stay byte-exact. `v128_any_true` for all early-outs
  and regime skips — no bitmask ops, per the V8 arm32 clobber lesson;
  p3..p6/q3..q6 rows only stored when a wide4 lane exists, mirroring
  the scalar write set.
- A55 jellyfish **−3.7%** (grinder and orchestrator confirm,
  spreads 0.4-0.8%), BBB **−1.2/−1.4%**; no-op anchors null.
  Compliance 307/307, 95/95 tests host+device (incl. a direct
  simd-vs-scalar wide-kernel sweep with mixed-regime crafted
  patterns), validates incl. lf_deltas and resize. No host/device
  divergence this time.

## 2026-07-03 — A55 campaign: pass-0 loop filter — measured, not merged

- Staged attempt at the vertical-edge (pass-0) kernel, transpose-gated.
  Stage 0 answered the lowering question: V8 arm32 lowers the 3-stage
  8x8 byte transpose well — 8/12 shuffles become single `vzip.8`/
  `vzip.16`, the u32 stage becomes `vdup/vsri/vsli` (no vtbl, no
  constant-table pressure). Even so: Tx4x4 narrow measured
  jellyfish −0.50% at 0.45% spread with a −0.14% no-op anchor (BBB
  −0.10%) — noise-level; the Tx8x8 wide3 variant on the same
  transpose was a real loss (jellyfish +0.96%, BBB +0.76%) and was
  reverted. The transpose round-trip tax eats the lane-math win as
  soon as the kernel widens. Conclusion for the campaign: pass-0
  stays scalar on the A55; the pass-1 kernels were the recoverable
  part of the loop filter. Diff not merged (win does not clear
  spread); evidence preserved here and in the notes.

## 2026-07-03 — A55 campaign: icache layout experiment — null, not merged

- PMU counters (per-process simpleperf stat on the d8 pid) confirmed
  the monolith hypothesis directionally: stalled-cycles-frontend
  13.1%, L1I refills 5× L1D refills (68.2M vs 13.8M per 5s), 0.82
  L1I refills per 100 instructions. But the actionable version of the
  fix — outlining rarely-executed subtrees with
  `#[cold]`/`#[inline(never)]` — measured null: the one clean outline
  (scalar/non-DCT_DCT `inverse_transform_2d_scalar`) shrank
  decode_residual only 140.7KB → 135.0KB arm32 (V8 verified not to
  re-inline), jellyfish −0.57% at 0.53% spread, BBB noise below its
  own anchor. Two further outline batches (simd-DCT scalar tail,
  scaled-inter branch) were tried and discarded by the grinder.
- Conclusion: the L1I cost is inherent to per-block phase cycling
  through ~135KB of hot code, not to cold code polluting the cache.
  Only structural phase batching (decode an SB's tokens, then batch
  transforms/reconstruct) could shrink the working set, and that is
  P9-scale surgery with entangled intra dependencies. Not pursued;
  diff not merged.

## 2026-07-03 — A55 campaign: simd ADST transforms

- The last sizable non-structural target from the refreshed profile:
  ADST-involved transform types took the fully scalar 2D path
  (scalar `inverse_adst` + `inverse_dct` symbols ≈ 6% jellyfish,
  5.4% BBB). Generalized the 4-lane-group simd driver to per-pass
  DCT/ADST selection: `inverse_adst4/8/16_simd` are structural 1:1
  mirrors of the scalar functions (same butterfly order, same
  round-at-14 points) built on the existing i64x2-extmul primitives,
  with `sb_simd`/`sh_simd` splitting raw i64 MACs from the rounding
  exactly as scalar `sb`/`sh` do. DCT gathers keep fused
  bit-reversal; ADST gathers load naturally and permute in-kernel;
  scalar tail for leftover rows unchanged; lossless stays scalar.
- A55 jellyfish **−2.5%** grinder / **−3.1%** orchestrator confirm
  (spreads ≤0.7%), BBB **−2.3%** (spread 0.4%), no-op anchors null.
  Compliance 307/307, 95/95 tests host+device, validates incl.
  quantizer-00 and quantizer-63. Scalar `inverse_adst` disappeared
  from the device profile (`inverse_adst_simd` now ~1.1-1.4%).

## 2026-07-03 — A55 campaign: i16-domain DCT

- The last big idea in the tank, licensed by the spec rather than by
  libvpx folklore: the bitstream spec makes it a conformance
  requirement that every value written into the transform array T —
  and the H butterfly's unrounded v/w — fits 8 + BitDepth bits
  (spec md, lines 4362/4391/4426/4442), i.e. signed 16-bit for the
  8-bit streams vip9r decodes. So DCT_DCT moves from 4-lane i32
  groups to 8-lane i16 groups. Exactness preserved by construction:
  rotation butterflies widen i16→i32 with extmul, accumulate and
  round the *sum* once at 14 bits (q15mulr was rejected up front —
  it double-rounds per product and diverges ±1 from the scalar
  reference), H stays wrapping i16 add/sub, saturating narrows can
  only fire on non-conforming streams. ADST keeps the i32 path — its
  S array is spec-designated higher precision. Row tails: 4-lane i16
  half-group for 4..7 leftovers (a scalar-only tail measured as a
  BBB loss and was fixed), scalar below that.
- A55 jellyfish **−5.4%** grinder / **−4.0%** confirm (spreads
  ≤0.8%), BBB neutral (its 4x4-heavy mix lives in tails and token
  decode, not group throughput). Compliance 307/307 — including
  quantizer-63, which supplies the real high-magnitude coverage that
  random sweeps structurally cannot (arbitrary vectors at high
  magnitude violate the very conformance bound that licenses i16).
  Sweep test's DctDct input range scoped to conforming magnitudes
  accordingly; 95/95 tests host+device.

## 2026-07-03 — A55 campaign: fill-traffic micro sweep

- End-of-campaign bundle of memory-traffic hygiene. Item 1: the
  ~1.4MB/frame neutral fill of the current frame's Y/U/V planes now
  runs only on first use of a frame-pool slot (flag set only after a
  successful tile parse, so an errored decode refills next time).
  Reader audit for never-written bytes: intra edges are explicit
  defaults via `IntraPredictionEdges`, inter prediction clamps to
  the *reference* frame's decoded dimensions, loop filter stays in
  decoded extent, show-existing shows fully decoded frames, resizes
  reconstruct their own full dimensions. Validates include resize,
  18x34, and show-existing vectors. Items 2+3: simd ADST permutation
  copies and s_lo/s_hi MAC arrays plus the scalar ADST scratch moved
  to persistent `DequantizedCoefficients` fields sized
  `MAX_ADST_WIDTH = 16` (staleness-safe: every lane read is written
  earlier in the same call — audited for adst8 and both adst16
  waves). memory.fill sites in the release dump: 33 → 24.
- Timing is an honest null. Grinder legs: item 1 alone BBB −1.13%
  at 0.04% spread, combined BBB −0.53%/jelly −0.04% (spreads ≤0.9%);
  orchestrator guard on the merged tree BBB +0.22% at 0.11% spread.
  Cross-session range −1.1%..+0.2% = noise floor / code layout.
  Merged on mechanism per the neutral-cleanup precedent: strictly
  less work per frame, fill sites verifiably gone. Compliance
  307/307, 95/95 tests host+device.

## 2026-07-03 — A55 campaign wrap: both-device margins

Whole-campaign position-corrected A/B, final tree vs the campaign-start
baseline (3e27aa5, post-simd128 master), X4 = Pixel 9a cpu7, A55 = TV
streamer cpu0:

| device | clip                  | before → after ms/frame | delta  | budget    | margin      |
|--------|-----------------------|-------------------------|--------|-----------|-------------|
| X4     | jellyfish-720p30 0:15 | 16.4 → 11.0             | −34.1% | 33.3 ms   | 3.0x under  |
| X4     | big-buck-bunny 0:15   | 15.1 → 11.8             | −22.4% | 40 ms @25 | 3.4x under  |
| A55    | jellyfish-720p30 0:5  | 211 → 144               | −31.7% | 33.3 ms   | 4.3x over   |
| A55    | big-buck-bunny 0:5    | 152 → 109               | −28.8% | 40 ms @25 | 2.7x over   |

- Campaign ledger, merged (every merge corpus-gated 307/307): P1
  residual restructure (−16.1% BBB), P2 add_residual simd, P4
  convolution v2 + cleanup, P-intra fast paths, P3-lite persistent
  intra edges, P-idct-scratch, P8 + P8-wide pass-1 loop filter simd,
  P-adst, P-i16dct (spec-licensed 16-bit DCT domain), fill-traffic
  micro sweep. Full `--all` corpus green at wrap (337/337).
- Measured and declined, with durable reasons: P1b traversal glue
  (decode_block region is layout-fragile), pass-0 loop filter simd
  (transpose round-trip tax beats good vzip lowering on in-order
  arm32), icache cold-outlining (L1I cost is per-block phase cycling),
  P9 SB-row fusion (L2D only 0.9GB/s), P7 wide refill
  (bitrate-bounded), P6 (already absorbed by P1).
- V8-portability posture held: every kernel is plain wasm simd128
  shaped by measured V8 arm32 lowering (extmul MACs, vzip-friendly
  interleaves, no bitmask in register-heavy kernels — documented
  arm32 codegen bug — no per-engine branches), so the tweaks carry to
  future V8 including the browser.
- A55 residue: jellyfish 144 ms/f is 4.3x over 720p30 single-core;
  remaining profile mass is the serial-entropy decode_block monolith
  and its icache phase cycling — structural, not kernel-shaped. Next
  levers are threads (M6, ~2.9x per ffvp9 scaling) and relaxed-simd,
  not more simd128 passes. ffvp9 gap narrowed to ~6.2x (144 vs 23.1).

## 2026-07-03 — the arm32 safety-check tax, measured

- Post-campaign question: how much A55 decode time goes to V8's
  explicit wasm bounds checks? arm32 cannot use the 4GB guard-region
  trick, so every heap access carries a compare-and-branch — the asm
  dump of the 40-line standalone `read_bool` alone shows ~8 of them
  (1d32b952-arm32). d8 ships "performance testing only" switches:
  `--no-wasm-bounds-checks`, `--no-wasm-stack-checks`.
- Manual counterbalanced B/C/C/B runs replicating the daemon's pinned
  bench invocation (streamer cpu0, tree 1d32b952, frames 0:5,
  measurement msPerFrame, spreads ≤1%):

| flags dropped   | clip      | ms/frame      | delta  |
|-----------------|-----------|---------------|--------|
| bounds          | jellyfish | 144.5 → 124.7 | −13.7% |
| stack           | jellyfish | 144.6 → 141.9 | −1.9%  |
| bounds + stack  | jellyfish | 145.2 → 119.5 | −17.7% |
| bounds          | BBB       | 109.1 → 90.3  | −17.3% |

- Reading: safety checks are ~1/6 of A55 decode. Superadditive
  (−17.7% combined vs −15.6% sum) — dropping the check branches also
  improves TurboFan's code shape. The entropy-heavy clip pays more,
  consistent with the tax landing on branchy, load-dense scalar code
  rather than on simd kernels.
- Not shippable (unsound; the browser pays these checks too). Value
  is as a bound: of the ~6.3x single-core ffvp9 gap on the A55,
  ~1.2x is check overhead that no source-level change can remove on
  arm32, and that mostly vanishes on arm64 targets via guard-region
  elision. Calibrates expectations for entropy-side micro-passes
  (branchless read_bool, parse→dequant fusion — tracker) whose
  upside sits inside the remaining ~5x.
- While here, asm evidence for the tracker's branchless item: the
  `read_bool` bit decision is a br_if diamond already in the wasm
  (LLVM won't selectify across the state stores); V8 lowers it as a
  data-dependent branch but demonstrably can emit predicated arm32
  moves (`movhi` for the bool itself). Masked-arithmetic Rust would
  make the whole update straight-line.
## 2026-07-03 — parse→dequant fusion; branchless read_bool measured null

- The two entropy-side items left after the campaign, run as one
  grinder task with two independently judged commits.
- Fusion (merged): coefficients are dequantized as their tokens are
  decoded — `(coef * quant) / dq_denom` written straight into the
  i32 transform input with the row mask updated in place. The
  quantized i16 buffer, the second walk over the scan prefix, and
  its dirty-prefix clear are gone (`TransformCoefficients` deleted,
  −85 lines net). Buffer cleanliness moved to a dirty flag: cleared
  before reuse under the block extent it was written with, so error
  paths can't leak stale coefficients. A55 BBB **−4.5%** (spread
  0.9%), jellyfish **−1.5%** (0.2%); X4 confirm BBB −1.6% (1.0%,
  credibility line), jellyfish noise. Compliance 307/307.
- Branchless read_bool (not merged): the underflow-mask rewrite took
  four source shapes before LLVM+V8 stopped re-deriving a branch —
  the final arm32 asm is genuinely straight-line, and it bought
  nothing: BBB −0.002%, jellyfish +0.3% at 0.7% spread, stacked on
  the fusion. The safety-check-tax entry above calibrated the
  ceiling correctly; the bit-decision branch itself was never the
  cost. Null recorded, branchy source kept for clarity.

## 2026-07-03 — A55 generated-code audit (asm reconnaissance)

- Post-campaign audit pass: fresh cpu0 profiles on both clips, then
  offset-level sample attribution inside the hot functions (perf.data
  IPs bucketed against the arm32 asm dump — function sizes match
  byte-for-byte, so device offsets index the dump directly), then two
  parallel analysis grinders auditing the hot regions against the A55
  optimization guide. No code changes; reports in docs/analysis/,
  backlog distilled into the tracker.
- Attribution shift worth recording: TurboFan inlines the entire
  per-block pipeline (predict_inter, predict_intra, tokens+dequant,
  reconstruct) into wasm[23] decode_residual — 51.5% of jellyfish /
  56.8% of BBB decode as one 164KB function, while the standalone
  predict_inter/subpel copies compile but take zero samples. Profile
  reads that stop at function granularity are misleading here; the
  offset histograms are the real map.
- Verified findings (orchestrator re-checked source + asm):
  - Fused dequant keeps `/ dq_denom` dynamic in the token loop; per
    nonzero coefficient V8 emits a frame reload, div-by-zero and
    INT_MIN/−1 trap guards, and a serializing `sdiv` — for a
    denominator that is 1 or 2 by tx_size. Inside BBB's dominant
    region (41% of decode_residual). Clearest source-level item.
  - loop_filter_is_block_edge modulo: divisors are always powers of
    two but arrive as table loads, so each edge test pays zero-guard
    + `udiv` + `mls`; a mask test removes all of it.
  - StoredModeInfo round-trips: mv candidate scan, loop-filter SB
    setup, and decode_block store path decode/encode the full 49-byte
    record (including a 32-byte sub_mvs copy) where callers consume a
    few fields.
- Engine-level warts recorded (not source-fixable, worth knowing when
  reading dumps): u8x16_narrow lowering emits a redundant [0,255]
  clamp with GPR-materialized constants before the already-saturating
  `vqmovun`; `v128.load64_zero` lowers to two `ldr` + lane `vmov`s;
  loop-filter kernels rematerialize vector constants per iteration.
- Register-pressure observations (token loop ~369 frame ld/st per
  1728 instructions; subpel coefficient Q-registers spilled inside
  inner loops) filed as speculative — same territory where traversal
  restructures measured negative.

## 2026-07-03 — A55 asm-audit micro-passes 1-2: divide elimination

- Pass 1, fused dequant: `/ dq_denom` with denominator const {1,2} by
  tx_size becomes a truncating shift (`dq_shift`); the per-nonzero-
  coefficient `sdiv` and its div-by-zero / INT_MIN trap guards leave
  the token loop (asm-verified: zero s/udiv in decode_residual).
  A55 timing **null** — BBB +0.2% (spread 0.2%), jellyfish +0.4%
  (spread 0.3%). Merged on mechanism, P10 precedent: real
  serializing-instruction removal in the hottest region at zero
  complexity cost, but the region is evidently not divide-latency
  bound.
- Pass 2, loop-filter block edges: `is_multiple_of(8 * num_8x8_*)`
  becomes a mask test (divisors are always powers of two); zero-guard
  + `udiv` + `mls` leave the per-segment edge walk (asm-verified:
  zero udiv in loop_filter_frame). A55 jellyfish **−0.6%** (spread
  0.5%), BBB **−0.2%** (spread 0.2%) — at the credibility line,
  directionally consistent on both clips.
- Calibration note for the remaining backlog: the audit's two
  highest-confidence scalar findings (both verified in asm, both
  serializing divides in hot regions) bought ≤0.6%. Instruction-level
  wins are noise-dominated on this in-order core unless they remove
  memory traffic or whole code regions; rank the remaining items
  accordingly.

## 2026-07-03 — A55 asm-audit pass 3: StoredModeInfo access specialization

- Grinder pass on the audit's finding: MV candidate scans, loop-filter
  SB setup, and the decode_block store path all round-tripped the full
  49-byte StoredModeInfo record where callers need a few fields.
  Changes, layout-preserving (access paths only):
  - `ModeInfoView` grew specialized getters: candidate header
    (valid/y_mode/ref_frames/mvs — no sub_mvs copy, no tx/mi_size
    validation), lazy per-sub-block `sub_mv` read, loop-filter
    mini-record, one-byte `segment_map_id`.
  - Candidate sub-MVs became `CandidateSubMvs` (RepeatedMvs /
    StoredCurrent{index} / Unavailable); the `block < 0` path reads
    `mvs[ref_list]` directly, exact because stored `mvs` is defined as
    `sub_mvs[..][3]` at encode time.
  - `find_mv_refs` decodes the prev-frame candidate once and reuses it
    across the same-ref/diff-ref passes.
  - `update_current_frame_modes` encodes one 49-byte template and
    copies it per covered MI, patching only the segment_map_id byte.
- Asm (arm32): mv_ref_candidate 53.1 → 19.2KB with zero vld1/vst1
  (sub_mvs copy gone); find_mv_refs vld/vst sites 125 → 3;
  loop-filter record load reads exactly the needed bytes.
- A55 **jellyfish −4.4% / BBB −3.8%** (spreads 0.8/0.3%); X4 confirm
  **−5.4% / −3.5%** (0.2/1.1%). First clearly-real win of the
  asm-audit backlog — memory-traffic removal, where the session's two
  divide eliminations (instruction-level) were noise.

## 2026-07-03 — A55 asm-audit pass 4: i16 DCT size specialization

- The simd i16 DCT_DCT schedule was runtime-recursive on transform
  size with dynamic `brev` bit-reversal angles and branchy
  `cos64`/`sin64` quadrant lookups per butterfly. Replaced with four
  straight-line bodies (n=2..5, each calling the next smaller; one
  `match` at the entry) — an exact expansion of the generic schedule,
  orchestrator-verified stage by stage. `b_simd_i16`/`h_simd_i16`
  arithmetic untouched; i32 sibling untouched.
- Asm (arm32): trig-table `vld1.16` splats, brev mask arithmetic, and
  quadrant compares all zero after; constants are literal movw/vmov.
  Total code *shrank* — 20.8KB recursive body → 15.5KB across the
  five specialized functions (the generic-schedule control flow cost
  more than the unrolled straight-line ops it drove).
- A55 **jellyfish −1.8%** (spread 0.14%), BBB −0.5% (spread 0.7%,
  noise — BBB's DCT share is small post-P-idct-scratch). X4 confirm
  jellyfish −0.03% at 1.27% spread: pure noise, consistent with the
  out-of-order core hiding the schedule overhead the A55 pays for.

## 2026-07-03 — A55 asm-audit backlog: session wrap

- Four passes executed, two more declined on evidence (checked-narrow
  trust extension, register-pressure reduction — see tracker for
  rationale). Cumulative A55 vs session start, counterbalanced:
  **jellyfish −5.5%** (142 → ~134 ms/frame, spread 0.3%),
  **BBB −5.1%** (103 → ~98, spread 0.3%). Best samples 133.5 / 97.3.
- Margin: jellyfish ~4.0x over the 33.3 ms 720p30 budget (was 4.3x at
  campaign wrap), BBB ~2.4x over its 40 ms @25fps budget (was 2.7x).
  Still a threads-scale gap; the audit backlog is closed and M6
  remains the path.
- Session lesson, now twice-confirmed: on this in-order core under
  V8, instruction-level substitutions (even serializing divides)
  measure null; what moves the number is eliminating memory traffic
  (StoredModeInfo pass, −4%) and collapsing whole control-flow
  regions (DCT specialization, −1.8% jelly with *less* code).

## 2026-07-03 — M6 threads: toolchain + ABI spike

- The build is now threaded-wasm unconditionally: `+atomics,+bulk-memory`,
  `--shared-memory --import-memory --export-memory`, core rebuilt via
  build-std (`RUSTC_BOOTSTRAP=1` on the pinned stable toolchain). One ABI —
  every JS frontend creates and imports a shared `WebAssembly.Memory`,
  workload-sized because V8 reserves the provided maximum upfront. Old-ABI
  baselines are retired; this entry is the cross-ABI reference point.
- Two feared costs evaporated under test: `compiler-builtins-mem` is
  unnecessary (+bulk-memory lowers memcpy/memset natively; the artifact has
  no memcpy import), and build-std adds only ~2-3s of core compile to a
  cold build — grinder sandboxes always cold-build anyway.
- Single-threaded on the threaded build: compliance corpus 307/307, wasm
  tests 95/95. Perf vs the logged post-fusion numbers: X4 jellyfish 9.4
  ms/frame (vs ~10.4, no regression); A55 jellyfish 138.4 vs ~132 (**+5%**),
  BBB 101.5 vs ~93.5 (**+8.5%**).
- The A55 tax was run to ground (windowed profile + asm diff, no atomics
  emitted anywhere): V8 stops caching the memory *size* across control flow
  for shared memories — decode_block's `ldr [instance,#size]` count goes 65
  → 556 (asm +8.8% bytes, +19% bounds-trap branches), decode_residual 219 →
  999 — so scalar entropy code pays a serialized load→sub→cmp→bcs chain per
  guarded access that the in-order core cannot hide. Profile confirms the
  shape: decode_block +32% absolute, mode-info +18%, decode_residual +11%,
  while simd-dense loop filter/IDCT are ~flat and libc/copy cost did not
  move (the relaxed-memcpy theory was falsified). Ironically the memory
  *base* became a hoisted constant (82 reloads → 1) — shared memory never
  relocates. Engine-level, not source-fixable; V8 could legally cache the
  monotonic size even for shared memories, so a future V8 may hand it back.
  A non-growable memory (initial == maximum) measured null — V8 does not
  constant-fold the size for shared memories regardless. A55 first-pass
  compile grew to ~3.7s; A55 benches keep using `--frames` windows.

## 2026-07-03 — M6 threads: worker pool runtime

- First live multi-threaded wasm execution: three d8 workers instantiate the
  module over the coordinator's shared memory, rebind their fixed shadow
  stacks, and park in `memory.atomic.wait32` on a static control block.
  Dispatch is an initial wave plus coordinator mop-up — no job queue; one
  Release epoch bump publishes the job slots, one Acquire join load brings
  worker writes back.
- One protocol decision was load-bearing in review: the join counter counts
  worker *acknowledgements* (always 3), not assigned jobs. Counting jobs
  lets a join complete while an idle worker is still on its way to read its
  empty slot — racing the next wave's slot rewrite with a torn read.
- Smoke coverage runs as ordinary wasm tests: the d8 runner spawns live
  workers for tests under the pool module; 50 back-to-back waves with
  coordinator-side work between dispatch and join, plus a partial wave, all
  writing through job-slot pointers into the coordinator's stack region.
  The bounded test join turns missing or miswired workers into a failure
  instead of a hang. Worker teardown is `Worker.terminate()` — probed safe
  mid-wait and at process exit, so no wasm-side shutdown path exists.
- Suite: wasm tests 97/97 (95 + 2 pool), compliance corpus 307/307, vitest
  green. Decode path untouched.

## 2026-07-04 — M6 threads: harness pool flag, pin sets, 4-core thermals

- The perf harness now speaks threads. Pins take sets (`cpu:0-3`,
  `cpu:0,2,4-5`), verified against the taskset's actual `Cpus_allowed_list`
  like single pins always were. Validate, bench, and profile requests take an
  explicit `pool` flag (`--pool` on `vip9r-perf-submit`,
  `vip9r-corpus-golden.py`, and the wasm-golden runner itself): spawn the
  3-worker pool, `vip9r_pool_activate`, per decoder instance. Never inferred
  from the pin set — serial-on-4-cores and oversubscribed-on-3-cores stay
  expressible; other request kinds reject it. Responses echo `pool` so a run
  can't be misread. Bench mode terminates each pass's pool before the next
  pass instantiates: parked workers would pin every retired pass's
  shared-memory reservation, which arm32 address space cannot afford.
- Verified end-to-end with today's (pool-inert) decoder: pooled golden and
  bench on host d8, pooled validate through the daemon on the streamer under
  `--pin cpu:0-3` (mask f, verified 0-3), pool wasm tests green, vitest green.
- Streamer 4-core sustained-load characterization (10 min, one pinned decode
  loop per core): no frequency derating — 2.0 GHz held throughout; `soc_max`
  28 → 47 °C, still flattening, thermal status 0. Cross-core contention on a
  measured core is ~5% (jellyfish on cpu:0 vs three-core load: 193.8 vs 184.6
  ms/frame) plus ~1% extra spread. The M6 scaling budget is compute-bound:
  neither thermals nor DRAM bandwidth will eat the multiplier.

## 2026-07-04 — M6 threads: tile-parallel tile decode

- Tile columns now decode in parallel: a wave hands one column band each to
  up to three workers, the coordinator decodes the remaining columns between
  dispatch and join, and the loop filter stays frame-wide on the coordinator.
  The parallel unit is the column *band* — all tile rows of a column on one
  thread — because `clear_above_context()` is per frame, so above context
  carries across tile rows within a column; a fresh per-band
  `TileModeContexts` is then exactly the frame-level clear. Single-column
  clips and pool-off runs keep the untouched serial loop.
- The shared-mutable surface went per-thread or loud: per-worker
  `SyntaxCounts` live in a new workspace-arena region and merge after join;
  all counts work (accumulate, zero, merge) is skipped when adaptation is off
  (`error_resilient || frame_parallel_decoding_mode` — the whole realworld
  perf corpus), which also deletes dead work from serial decode. Current-frame
  planes and the mode grid cross the job boundary as raw parts rebuilt into
  band-restricted views: the unsafe is confined to the split/rebuild, and
  every accessor checks the band so a cross-band access is an
  `InvalidBitstream` decode failure instead of a data race.
- A55 streamer (`cpu:0-3`, 0:5 windows, corrected deltas, spreads ≤0.8%):
  jellyfish −54.6% (139.7 → 63.5 ms/frame), BBB −59.7% (104.5 → 42.0), and
  the adaptation-on flag=0 lane f247 −53.4% (132.3 → 61.7) — the per-worker
  counts merge is bit-exact and keeps the win. Serial no-pool decode is
  unchanged (−0.15% at 1.8% spread). 2.2–2.5x against the ffvp9 ~2.9x anchor
  with the loop filter (~10–19%) still serial — Amdahl-consistent. BBB now
  sits 5% over its 40 ms budget; jellyfish 1.9x over 33.3. The loop-filter
  SB-row wavefront is the next lever.
- Review caught one real bug in the grinder's dispatch path: an error return
  between dispatch and join would unwind the coordinator frame while workers
  still held pointers to its stack-resident result cells; errors now funnel
  through result aggregation so join always runs.
- Negative result, recorded: pooled decode on the Pixel measures 3.3x
  *slower* than serial X4 (12.2 → 41 ms/frame pinned `cpu:4-7`; worse under
  an all-cores pin). Telemetry shows the A720 policy parked at 578→357 MHz:
  the daemon controls affinity only, and schedutil/EAS never ramps for the
  futex-parked worker threads. Pixel pooled numbers are meaningless until
  the harness locks frequencies — and product-side, pool activation on a
  fast-serial big.LITTLE device can be a real pessimization.
- Suite: wasm tests 100/100 (band-view + pool coverage), compliance corpus
  307/307 pooled and serial, full corpus 339/339 pooled, vitest 43/43.

## 2026-07-04 — M6 threads: loop filter SB-row wavefront

- The loop filter — the serial remainder after tile-parallel decode — now
  runs as a wavefront over superblock rows on the same worker pool: one wave
  per filtered frame, participant p of 4 owns SB rows p, p+4, ..., and
  per-row atomic watermarks order the front. The lag rule is derived from
  filter reach, not copied: filtering SB (r, c) touches an 8-pixel apron
  into its left and above neighbors, and the above apron overlaps the left
  apron of the above-right neighbor, so row r may filter column c once row
  r-1 has completed column c+1. Under that rule every reorderable pair of
  superblocks has disjoint touch windows — bit-exact against serial raster
  order by commutation, and corpus-verified.
- Containment follows the tile-band philosophy: participants hold aliased
  full-plane views, but each view carries a per-superblock window (SB extent
  plus the 8px apron) and `loop_filter_segment` checks every segment's
  maximal touch rectangle against it before the raw kernel accesses — a
  reach bug becomes an `InvalidBitstream` failure instead of a data race. A
  failing participant marks its remaining rows complete without touching
  pixels, so errors aggregate after join (lowest SB raster index, matching
  serial reporting) instead of deadlocking the front.
- A55 streamer (`cpu:0-3`, 0:5 windows, corrected deltas vs tile-parallel
  baseline f8d54a7): jellyfish −28.3% (≈64.7 → 46.4 ms/frame, spread 3.1%),
  BBB −14.3% confirm run (47.6 → 40.8 warm, spread 1.8%; first read −11.2%
  at 4.5% was heat-drifty), f247 flag=0 lane −21.4% (63.2 → 49.7, spread
  2.3%). Deltas are Amdahl-consistent with the post-tile-parallel loop
  filter share (~42% jellyfish, ~21% BBB). Host pooled jellyfish −40.9%.
  Controls: A55 serial −1.6% at 2.3% spread, X4 serial +1.3% at 2.5%, host
  serial +0.15% — all noise.
- Budget position: BBB lands at its 40 ms budget — the first realworld 720p
  clip at budget on the A55. Jellyfish sits ~1.4x over 33.3 ms, matching the
  arithmetic that gates the decode_residual reshape spike (threads-era
  profiles next).
- Suite: wasm tests 100/100, compliance corpus 307/307 pooled and serial,
  full corpus 339/339 pooled.

## 2026-07-04 — decode_residual reshape spike: closed without a campaign

- The gated spike asked whether any source-level shape (phase batching,
  monolith split/merge, V8-respected inlining barriers) could move the L1I
  phase-cycling cost. The evidence gate answered it before a grinder run
  was justified: the mechanism's total pool is single-digit, below the
  pre-committed double-digit escalation bar.
- Threads-era attribution (A55 pooled `cpu:0-3`, 0:5 windows): the
  decode_residual monolith is 50.5% jellyfish / 54.2% BBB — same share as
  the pre-threads profiles, so the spike premise was worth re-measuring.
- PMU re-anchor (per-process simpleperf stat, serial `cpu:0` jellyfish
  full-clip validate, 2.0 GHz held, two clean 5 s samples): frontend
  stalls 8.6–9.0% of cycles (P5: 13.1%), L1I refills 0.48–0.51 per 100
  instructions (P5: 0.82), L1I:L1D refill ratio 4:1 (P5: 5:1), backend
  stalls 30%. The post-P5 passes — fusion, StoredModeInfo/divide/IDCT
  specializations — already removed ~40% of the per-instruction icache
  tax as a side effect; the P5 premise numbers no longer exist.
- Ceiling arithmetic: eliminating *every* frontend stall — impossible;
  any 135 KB hot loop has compulsory refills and entropy branches are
  data-dependent — buys ~9%, under the double-digit seam the tracker
  requires, and far from the ~28% single-CPU that jellyfish-class clips
  would need on top of threads.
- Directional evidence agrees: every probe at this seam measured
  null-to-negative (icache cold-outlining, P1b traversal restructure,
  branchless bool decision), and the one structural move that won —
  parse→dequant fusion, −4.5% BBB — won by *tightening* phase
  interleave, the opposite of phase batching.
- Decision per the tracker's rule: single-CPU work is closed. M6 threads
  own the remaining A55 gap; the jellyfish-class residual (~1.4x over
  33.3 ms pooled) stays accepted as scoped.

## 2026-07-05 — M6 threads: fused decode+filter wave

- One pool wave per frame instead of two: each participant decodes its tile
  column band, then falls into a shared loop-filter row pool. This spends
  the two idle pools the 2026-07-05 profiles measured — the 6-10 ms decode
  join tail (workers draining 2→1 behind the slowest band while the
  coordinator parks) and the filter wavefront ramp.
- Decode publishes one monotone watermark per band: completed global SB
  rows, stored after each SB row of `parse_tile` (tile rows stack
  vertically within a band, tile boundaries are SB-aligned). Filtering
  (r, c) waits — on top of the intra-wave lag rule — for decode row
  min(r+2, sb_rows) in every band intersecting SB columns c-1..c+1: intra
  prediction in decode row r+1 can read above/above-left/above-right, and
  (r, c+1) can read left, from pixels inside filter(r, c)'s touch window.
  Bands ending at column c-2 or earlier need no gate — their farthest
  above-right reach (32 px, the max transform width) falls 56 px short of
  the left apron. One row of decode lag makes every remaining reorderable
  decode/filter pair touch-disjoint: still bit-exact by commutation.
- Filter rows are claimed by a shared fetch_add counter instead of the
  static p, p+4 stride, so the last-finishing decode band no longer owns
  blocked rows; participants without a decode band (2-column clips) start
  claiming immediately. Deadlock-free by claim order: the lowest unfinished
  row's row-above is always complete, decode never waits, and both decode
  and filter error paths force-publish their watermarks before returning.
  The standalone wavefront (single-tile clips) adopts the same row claiming.
- A55 streamer (`cpu:0-3`, 0:9 windows, corrected deltas vs daca1ed):
  jellyfish −15.1% at 0.1% spread (grinder read; orchestrator confirms
  −10.8..−12.2% on heat-drifty 4% spreads, candidate ≈37.2 ms/frame), BBB
  −7.4% at 0.2% spread (33.9 → 31.4 ms/frame), f247 flag=0 lane −11.6% at
  0.2% spread (candidate ≈36.4 ms/frame). All at or above the −8..12%
  scoping estimate; BBB "less" prediction held until the row claim also
  absorbed its band imbalance.
- Budget position: BBB ~31 ms/frame sits well under its 40 ms budget;
  jellyfish ~37 vs 33.3 is ~1.1x over (was ~1.4x); f247 ~36 vs 33.3
  likewise. Bench windows overweight the keyframe 1/6 vs ~1/300 in real
  playback, so sustained playback sits below these numbers.
- Suite: wasm tests 100/100, compliance corpus 307/307 pooled and serial,
  full corpus 339/339 pooled, libvpx odd-size/resize/aq2/skip/segkey
  vectors green in both pool modes.

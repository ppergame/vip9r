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

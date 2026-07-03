# A55 secondary generated-code audit: loop filter and mode-info functions

Target/codegen assumptions used here: arm32 TurboFan asm from
`vip9r-perf-submit asm --arch arm32` on the current tree.  Offsets below are
function-relative.  Estimated costs are **hot-region budgets**, computed from
provided profile shares as `clip_ms * function_share * region_share`; they are
not claimed achievable savings.

## 1. MV candidate scans repeatedly decode/copy full 49-byte mode records, including sub-MVs that most passes do not use

- **Offsets:**
  - `mv_ref_candidate` wasm[26]: `0x00900-0x01100` (81.2% of function), especially `0x00944-0x00c18`.
  - `inter_block_mode_info` wasm[22]: inlined/repeated candidate paths in `0x03300-0x03f00`, `0x04600-0x05300`, `0x05a00-0x08300`.
- **Estimated hot-region budget:** about **6.4 ms/f jellyfish**, **3.8 ms/f BBB** for `mv_ref_candidate 0x900-0x1100` plus `inter_block_mode_info 0x5a00-0x8300`; the other inter regions add another ~2.8/2.0 ms/f of similar-looking code.
- **Representative asm:**

  ```asm
  ; mv_ref_candidate current_frame_modes path
   9c0: e3a08031  mov     r8, #49        ; STORED_MODE_INFO_BYTES
   9cc: e0825896  umull   r5, r2, r6, r8 ; index * 49
   a30: e5d5602d  ldrb    r6, [r5, #45]  ; tx_size validation
   a64: e5d5902f  ldrb    r9, [r5, #47]  ; mi_size validation
   b04: f4260a0f  vld1.8  {d0-d1}, [r6]  ; copy sub_mvs chunk
   b1c: f4060a0f  vst1.8  {d0-d1}, [r6]
   b5c: e5989004  ldr     r9, [r8, #4]   ; mvs/ref fields
   b84: e5d8102e  ldrb    r1, [r8, #46]  ; segment_id, unused here
   c00: e5d8202c  ldrb    r2, [r8, #44]  ; skip, unused here
  ```

- **Source attribution:** `TileParser::mv_ref_candidate` in
  `crates/vip9r/src/tile_syntax/mod.rs`, via
  `ModeInfoView::get(index)? -> decode_stored_mode_info(entry)` and then
  `CandidateModeInfo::from(info)`.  `find_mv_refs` scans candidates in multiple
  passes: first two candidates for mode context/sub-block MV, later candidates
  for same-ref MVs, and then (if any candidate was found) a second all-candidate
  pass for diff-ref MVs.
- **Why this is suboptimal on A55:** the candidate hot path is a branch/memory
  wall on an in-order core.  It decodes and validates `tx_size`, `mi_size`,
  `skip`, `segment_id`, and `segment_map_id` even though MV reference logic only
  needs `valid`, `y_mode`, `ref_frames`, `mvs`, and sometimes `sub_mvs`.  It also
  copies all 32 bytes of `sub_mvs` for candidates whose caller only uses
  `ref_frames`/`mvs`.  Re-scanning candidates repeats the same pointer chasing
  and 49-byte decode.
- **Plausible remedy:** **source-level.** Split candidate access into a small
  header decode (`valid/y_mode/ref_frames/mvs`) plus lazy `sub_mvs` decode only
  for the first two candidates when `get_sub_block_mv` is actually needed.
  Consider collecting the candidate headers once per `find_mv_refs` call and
  reusing them for the same-ref and diff-ref passes.  Longer-term, a field-split
  mode-info layout would make the candidate scan cheaper still.
- **Engine-level angle:** TurboFan would need aggressive scalar replacement and
  CSE across the multi-pass candidate scan to remove this; source restructuring
  is much more plausible.
- **Confidence:** **High.** The `#49` multiply and full-record copy/validation
  are directly visible in both the standalone function and inlined inter paths.

## 2. Loop-filter block-edge test lowers to serializing `UDIV` even though divisors are powers of two

- **Offsets:** `loop_filter_frame` wasm[49] `0x02500-0x03000`, especially
  `0x02cc8-0x02d28`.
- **Estimated hot-region budget:** **6.0 ms/f jellyfish**, **3.1 ms/f BBB** for
  the containing bucket.  The two divides are only part of that bucket, but they
  are in the per-segment edge walk.
- **Representative asm:**

  ```asm
   2cc8: e51b71c0  ldr     r7, [fp, #-448]    ; table base
   2cd8: e7d25005  ldrb    r5, [r2, r5]
   2cdc: e1a05185  lsl     r5, r5, #3         ; 8 * num_8x8_wide/high
   2cec: e734f518  udiv    r4, r8, r5
   2cf0: e0648594  mls     r4, r4, r5, r8     ; coord % divisor
   ...
   2d14: e1a07187  lsl     r7, r7, #3
   2d24: e735f718  udiv    r5, r8, r7
   2d28: e0658795  mls     r5, r5, r7, r8
  ```

- **Source attribution:** `loop_filter_is_block_edge(pass, x, y, sb_size)` in
  `crates/vip9r/src/tile_syntax/loop_filter.rs`:

  ```rust
  x.is_multiple_of(8 * usize::from(sb_size.num_8x8_wide()))
  y.is_multiple_of(8 * usize::from(sb_size.num_8x8_high()))
  ```

- **Why this is suboptimal on A55:** A32 integer divides take up to 12 cycles and
  are serializing on Cortex-A55.  The possible divisors here are 8, 16, 32, and
  64, so modulo can be a mask test.
- **Plausible remedy:** **source-level.** Replace this path with a power-of-two
  mask, e.g. derive `edge_span = 8 * num_8x8_*` and test
  `(coord & (edge_span - 1)) == 0`, or table the log2/span per `BlockSize`.
- **Engine-level angle:** V8 could strength-reduce if it knew the table values
  were powers of two; it currently does not.
- **Confidence:** **High.** The `UDIV`/`MLS` pair maps cleanly to the source
  modulo, and the divisor domain is fixed by VP9 block sizes.

## 3. `decode_block` materializes large block/mode records by value and encodes stored mode records field-by-field

- **Offsets:** `decode_block` wasm[14] `0x08700-0x0aa00` (78.5% of function on
  BBB, 60.4% on jellyfish).  The clearest subregions are `0x098bc-0x09af8`
  (large `DecodedBlockInfo`/call setup) and `0x09e80-0x0a500+`
  (`update_mode_context` / `update_current_frame_modes` record writes).
- **Estimated hot-region budget:** **4.9 ms/f jellyfish**, **5.4 ms/f BBB** for
  the containing region.
- **Representative asm:**

  ```asm
   98bc: f3000150  veor    q0, q0, q0       ; start constructing aggregate
   9934: e5415043  strb    r5, [r1, #-67]
   99d0: e14144bc  strh    r4, [r1, #-76]
   9a70: e0814004  add     r4, r1, r4
   9a78: f4040a0f  vst1.8  {d0-d1}, [r4]
   9af8: e12fff3c  blx     ip              ; decode_residual call boundary
   ...
   a3dc: e2894031  add     r4, r9, #49     ; next StoredModeInfo record
   a404: e3a03001  mov     r3, #1
   a408: e7c03007  strb    r3, [r0, r7]    ; valid
   a41c: e5c32030  strb    r2, [r3, #48]   ; segment_map_id
   a42c: e5c3202e  strb    r2, [r3, #46]   ; segment_id
   a4fc: e2872025  add     r2, r7, #37     ; repeated field stores continue
  ```

- **Source attribution:** `TileParser::decode_block`,
  `TileModeContexts::update_mode_context`,
  `TileParser::update_current_frame_modes`, and
  `ModeInfoViewMut::set` / `encode_stored_mode_info`.
- **Why this is suboptimal on A55:** the source representation makes the hot
  block path build a large `DecodedBlockInfo`, pass it across a call boundary,
  then re-read pieces to update neighbor contexts and encode one or more
  49-byte `StoredModeInfo` records.  The final encode is many small stores and
  address/bounds computations for a fixed-size record.  When preserving the old
  segmentation map, the path can also decode a previous full mode record just to
  keep `segment_map_id`.
- **Plausible remedy:** **source-level.** Avoid by-value aggregate traffic in the
  hot path: keep the syntax result in a compact mutable block state or pass it
  by reference, pre-encode a `[u8; STORED_MODE_INFO_BYTES]` template once per
  block, and copy/patch it for each covered MI.  Add byte-only accessors for
  `segment_map_id` preservation.  More invasive but potentially better: split
  current-frame mode storage by field so loop filter and MV reference users do
  not force a monolithic 49-byte encode/decode cycle.
- **Engine-level angle:** fixed-length slice bounds elimination and scalar
  replacement of the block aggregate would help, but the current source hides
  fixed record size behind slices and by-value structs.
- **Confidence:** **Medium-high.** The offsets clearly show aggregate/record
  traffic; exact savings are harder to isolate because the region also contains
  entropy-mode work.

## 4. Horizontal loop-filter SIMD kernels suffer engine-level scalar load64 lowering, constant materialization, and Q-register spills

- **Offsets:** `loop_filter_frame` wasm[49] `0x03f00-0x05600` and
  `0x05800-0x07100`.
- **Estimated hot-region budget:** combined **6.0 ms/f jellyfish**; BBB reports a
  similar horizontal-SIMD bucket around `0x05200-0x06c00` at about **0.9 ms/f**.
- **Representative asm:**

  ```asm
   4010: e0831006  add     r1, r3, r6
   4014: e1510005  cmp     r1, r5
   4020: e7927001  ldr     r7, [r2, r1]
   402c: e5900004  ldr     r0, [r0, #4]
   4034: ee0a7b10  vmov.32 d10[0], r7     ; v128_load64_zero via GPRs
   4038: ee2a0b10  vmov.32 d10[1], r0
   ...
   40d0: f40c8a0f  vst1.8  {d8-d9}, [ip]  ; spill live q regs
   40d8: f40caa0f  vst1.8  {d10-d11}, [ip]
   ...
   43ec: e300c200  movw    ip, #512       ; shuffle masks/constants rebuilt
   43f0: e340c604  movt    ip, #1540
   441c: f3b08b0e  vtbl.8  d8, {d0-d3}, d14
  ```

- **Source attribution:** inlined `loop_filter_tx4x4_horizontal_8`,
  `loop_filter_tx8x8_horizontal_8`, `loop_filter_tx16x16_horizontal_8`,
  `load_loop_filter_row_8`, and `i16x8_mask_to_u8x8` in
  `loop_filter.rs`.
- **Why this is suboptimal on A55:** A55 is in-order with limited ability to hide
  load/use and register-pressure stalls.  The generated code often lowers an
  8-byte SIMD row load to two scalar `LDR`s plus lane `VMOV`s instead of a single
  NEON 64-bit load, rebuilds vector constants/shuffle masks with `movw/movt`
  sequences, and spills Q registers to the stack in the wide kernels.  This is
  separate from the known `VTBL` shuffle fact; the extra scalar/stack traffic is
  the issue here.
- **Plausible remedy:** mostly **engine-level**: better `v128.load64_zero`
  lowering, NEON-immediate constant formation/CSE, and register allocation.
  A source workaround might reduce live Q pressure by splitting the Tx16 wide
  path into smaller helper phases or reducing simultaneous `wide3`/`wide4`/
  `narrow` live values, but this risks code-size and branch tradeoffs.
- **Engine-level tag:** primary.
- **Confidence:** **Medium.** The code shape is visibly expensive; whether a
  source refactor wins on A55 needs measurement because previous loop-filter
  SIMD changes had non-obvious tradeoffs.

## 5. `inverse_dct_simd_i16` is a generic recursive schedule with dynamic `brev`/trig-table work in hot code

- **Offsets:** `inverse_dct_simd_i16` wasm[31] `0x02e00-0x04200` and prologue
  bucket `0x00000-0x00500`.
- **Estimated hot-region budget:** **2.6 ms/f jellyfish**, **1.2 ms/f BBB** for
  `0x02e00-0x04200`; the prologue bucket adds about **0.9/0.4 ms/f**.
- **Representative asm:**

  ```asm
   2e0c: e79c9109  ldr     r9, [ip, r9, lsl #2]
   2e10: e12fff39  blx     r9              ; recursive/helper call shape
   ...
   2ff8: e30010ff  movw    r1, #255        ; dynamic brev(5, ...)
   3014: e0012225  and     r2, r1, r5, lsr #4
   3048: e0421da5  sub     r1, r2, r5, lsr #27
   307c: f4a20c6f  vld1.16 {d0[]-d1[]}, [r2] ; cos table load/splat
   3088: e3550021  cmp     r5, #33         ; cos64 quadrant handling
   3154: f2904c02  vmull.s16 q2, d0, d2
   3174: f228895a  vmul.i32 q4, q4, q5
  ```

- **Source attribution:** recursive, size-parametric
  `inverse_dct_simd_i16(t, n)` plus `b_simd_i16`, `h_simd_i16`, `brev`,
  `cos64`, and `sin64` in `residual.rs`.
- **Why this is suboptimal on A55:** the transform sizes are only 4/8/16/32, but
  the generated code carries a runtime schedule: recursive control flow,
  dynamic bit reversal, branchy `cos64` quadrant logic, and table loads for
  constants.  On an in-order core this interleaves scalar branch/table work with
  NEON multiply stages and adds call/stack overhead around what could be a
  straight-line schedule per size.
- **Plausible remedy:** **source-level.** Specialize/unroll the i16 DCT for each
  `n` (or dispatch once on `n` to size-specific helpers) with precomputed
  `(a,b,cos,sin,flip)` stages.  This would trade code size for removing dynamic
  `brev`/`cos64` work and recursive calls.  Measure carefully because the known
  V8 multiply-lowering limitations still remain.
- **Engine-level angle:** stronger inlining/constant propagation through the
  size dispatch could help, but source specialization is more realistic.
- **Confidence:** **Medium-high.** The dynamic schedule is clear; the code-size
  vs speed tradeoff needs benchmarking.

## 6. Loop-filter superblock setup decodes full mode records even though loop filtering needs only a few fields

- **Offsets:** `loop_filter_frame` wasm[49] `0x01c00-0x02200`, with continuation
  into `0x02500-0x02628` for writing `LoopFilterMiInfo`.
- **Estimated hot-region budget:** **1.7 ms/f jellyfish**, **1.1 ms/f BBB** for
  `0x01c00-0x02200`.
- **Representative asm:**

  ```asm
   1d94: e0883597  umull   r3, r8, r7, r5  ; index * 49
   1e0c: e5d6202d  ldrb    r2, [r6, #45]   ; tx_size decode
   1e48: e5d6502f  ldrb    r5, [r6, #47]   ; mi_size decode
   1ee8: f4224a0f  vld1.8  {d4-d5}, [r2]   ; copy sub_mvs, not needed by LF
   1f00: f4074a0f  vst1.8  {d4-d5}, [r7]
   1f40: e5919004  ldr     r9, [r1, #4]    ; mvs, not needed by LF
   ...
   25a0: e5854006  str     r4, [r5, #6]    ; final LoopFilterMiInfo strength
  ```

- **Source attribution:** `loop_filter_superblock_info -> loop_filter_mode_info
  -> ModeInfoView::get -> decode_stored_mode_info` in `loop_filter.rs` and
  `mode_info.rs`.
- **Why this is suboptimal on A55:** loop filtering only needs `valid`, `skip`,
  `tx_size`, `mi_size`, `segment_id`, `y_mode`, and `ref_frames[0]`.  It still
  pays to validate and copy motion vectors/sub-MVs, the second ref frame, and
  segment-map-only fields through the full `StoredModeInfo` path.
- **Plausible remedy:** **source-level.** Add a loop-filter-specific raw getter
  on `ModeInfoView` that reads only the needed bytes and constructs
  `LoopFilterMiInfo` directly (including `uv_tx_size`/strength lookup).  This is
  complementary to the MV-candidate specialized getter above.
- **Engine-level angle:** scalar replacement could remove unused fields if the
  aggregate did not cross abstraction boundaries; current codegen does not.
- **Confidence:** **High.** The asm copies fields that loop filtering cannot use.

## Non-findings / checked-but-not-reported

- `loop_filter_frame 0x08600-0x09700` maps to scalar `sample_filter_direct` /
  `wide_filter_direct` for direct edge filtering.  It is very hot
  (~10.4 ms/f jellyfish region budget), but the obvious SIMD answer is the
  already-known pass-0 loop-filter SIMD tradeoff, and the compare/branch walls
  include the already-known wasm/Rust bounds-check tax.  I did not count this as
  a new finding.
- I did not re-report V8's known `i8x16_shuffle -> VTBL`, bitmask lowering, or
  Q-form throughput limitations.  The SIMD loop-filter finding above is about
  additional scalar load64 lowering, constant materialization, and spills around
  those known facts.
- I found no additional hot integer divides in `mv_ref_candidate`,
  `inter_block_mode_info`, or `decode_block`; the obvious divide issue is the
  loop-filter block-edge modulo above.
- `mv_ref_candidate 0x00000-0x00200` is mostly candidate boundary/tile-range
  checking.  It is branchy but small and source-necessary; the bigger problem is
  what happens after the candidate is accepted.
- In `inverse_dct_simd_i16`, the lack of fused `VMLAL`/`VMLS` patterns is the
  known V8 extmul/multiply lowering issue, so the finding is limited to the
  source-level generic schedule and dynamic constants.

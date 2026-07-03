# Cortex-A55 arm32 generated-code audit: `decode_residual` wasm[23]

## Scope and method

Target assembly: `vip9r-perf-submit asm --arch arm32`, `023-decode_residual.s` from `/tmp/vip9r-asm/cbdbe9e3-arm32/`, 163936 bytes.  Offsets below are function-relative.

I also ran one short BBB device profile (`--target device profile --frames 0:5`) only as a sanity check; it reported `decode_residual` at 57.61% of samples / 104.51 ms per frame, consistent with the supplied profile.  No source changes were made.

Cost calibration used the supplied full-profile numbers:

- jellyfish: `decode_residual` = 51.5% of 143.5 ms/frame = 73.9 ms/frame.
- BBB: `decode_residual` = 56.8% of 104.5 ms/frame = 59.4 ms/frame.

The estimates below are opportunity estimates, not measured speedups.  I intentionally exclude the already-known general wasm heap bounds checks, stack checks, read_bool branch behavior, and monolith/icache phase cycling except where a specific new source/codegen shape stands out.

---

## 1. Per-nonzero-coefficient signed divide in fused dequantization

**Offsets:** primarily `0x121b4-0x122b0`, inside the hot `0x10b00-0x12600` token/dequant region.

**Representative asm:**

```asm
121b4: e3070fff  movw    r0, #32767
...
12244: e6bf0070  sxth    r0, r0
12248: e0000096  mul     r0, r6, r0        ; coefficient * quant
...
12278: e715f610  sdiv    r5, r0, r6        ; / dq_denom
12288: e7895003  str     r5, [r9, r3]      ; dequantized coeff
```

**Source attribution:** `crates/vip9r/src/tile_syntax/residual.rs`, `DequantizedCoefficients::set_signed_dequantized`:

```rust
self.coefficients[pos] = (i32::from(coefficient) * quant) / dq_denom;
```

`dq_denom(tx_size)` is only `1` for 4x4/8x8/16x16 and `2` for 32x32, but it reaches the hot loop as a variable integer denominator.

**Estimated cost:** The enclosing region is about **24.3 ms/frame on BBB** and **12.9 ms/frame on jellyfish**.  One A55 AArch32 `sdiv` is serializing and takes up to 12 cycles.  Because this executes on every nonzero residual coefficient, I would rank this as the clearest multi-ms/frame opportunity in BBB-heavy residual frames, even though the exact fraction of the region due to the divide is unknown.

**Why suboptimal on A55:** Cortex-A55 integer divides are iterative/serializing (`SDIV` throughput up to one per 12 cycles).  Dividing by a known `{1,2}` denominator should be either free or a small signed halving sequence.

**Remedy sketch:** Specialize fused dequant by `tx_size`/denominator before entering the coefficient loop.  For `dq_denom == 1`, store the product.  For `2`, use signed truncating halve, e.g. `(x + ((x >> 31) & 1)) >> 1`, or pre-scale in a tx32-specific path if that matches the spec.  Avoid carrying a variable `dq_denom` into `set_signed_dequantized`.

**Tag:** source-level.

**Confidence:** High for code shape and source attribution; medium-high for speedup magnitude.

---

## 2. Token loop has heavy frame spill/fill traffic beyond heap bounds checks

**Offsets:** `0x11768-0x122f8` within `0x10b00-0x12600`; setup and loop-invariant materialization also starts around `0x10b68` and `0x0b300`.

**Representative asm:**

```asm
11768: e50b410c  str     r4, [fp, #-268]
1176c: e51b203c  ldr     r2, [fp, #-60]
11774: e50b91c0  str     r9, [fp, #-448]
1177c: e50b00a0  str     r0, [fp, #-160]
11784: e55a7fe6  ldrb    r7, [sl, #-4070]
11788: e51b805c  ldr     r8, [fp, #-92]
1178c: e51b4084  ldr     r4, [fp, #-132]
11790: e51b0170  ldr     r0, [fp, #-368]
...
119bc: e55a4fe6  ldrb    r4, [sl, #-4070]
119c0: e50b60fc  str     r6, [fp, #-252]
119c4: e51b2198  ldr     r2, [fp, #-408]
```

In the `0x10b00-0x12600` region, static counts are roughly 506 memory instructions, including about 369 frame (`[fp,#-N]`) loads/stores, in 1728 instructions.

**Source attribution:** `TileParser::tokens` in `crates/vip9r/src/tile_syntax/mod.rs`, especially the coefficient loop around `scan`, `coef_bands`, `coef_probs`, counts arrays, `BoolDecoder`, token cache, and fused dequantization.

**Estimated cost:** Same enclosing region as finding 1: **24.3 ms/frame BBB**, **12.9 ms/frame jellyfish**.  The spill/fill overhead is likely smaller than the intrinsic entropy work and the divide, but a 10-15% reduction of this region would still be several percent of total BBB decode.

**Why suboptimal on A55:** A55 is in-order with two-cycle load-use latency and only one load pipeline.  The generated loop repeatedly reloads local state from the 900-byte frame and stores it back across branches.  This is separate from the known wasm heap bounds checks: these are spills/reloads of compiler-local state and loop invariants (table bases, counters, decoder fields, temporary probability/count pointers).

**Remedy sketch:** Future source experiments should try to reduce live state in the token loop rather than change the `read_bool` branch.  Plausible directions: specialize by `tx_size`/plane/ref-type before the loop, split token decoding from dequant writes, pass a smaller loop state, or keep `dq_denom`/quant/table bases in a more compact struct.  This may fight V8's wasm inliner/register allocator, so confirmation by asm and bench is required.

**Tag:** mixed source/engine.

**Confidence:** Medium.  The spill density is clear; the amount source restructuring can recover is uncertain.

---

## 3. Subpel SIMD filters spill coefficient vectors and reload them inside inner loops

**Offsets:** jellyfish-heavy regions `0x05200-0x05d00`, `0x06c00-0x07800`, `0x08d00-0x09600`; representative inner loops at `0x56c0-0x5a20` and `0x8f28-0x9278`.

**Representative asm:**

```asm
56c4: e1d320f6  ldrsh   r2, [r3, #6]
56c8: eea82b10  vdup.32 q4, r2
...
5748: f40c2a0f  vst1.8  {d2-d3}, [ip]
5750: f40c4a0f  vst1.8  {d4-d5}, [ip]
5758: f40c6a0f  vst1.8  {d6-d7}, [ip]
5760: f40c8a0f  vst1.8  {d8-d9}, [ip]
5768: f40caa0f  vst1.8  {d10-d11}, [ip]
5774: f44c2a0f  vst1.8  {d18-d19}, [ip]
...
59cc: e24bcf4a  sub     ip, fp, #296
59d0: f46caa0f  vld1.8  {d26-d27}, [ip]
59e0: e24bcf96  sub     ip, fp, #600
59e4: f46cca0f  vld1.8  {d28-d29}, [ip]
59f4: e24bcfa6  sub     ip, fp, #664
59f8: f46c8a0f  vld1.8  {d24-d25}, [ip]
5a08: e24bcfba  sub     ip, fp, #744
5a0c: f46c6a0f  vld1.8  {d22-d23}, [ip]
```

**Source attribution:** inlined `inter_predict_subpel_unscaled_block`, particularly `horizontal_filter_8`, `vertical_filter_8`, `WasmInterpCoefficients::new`, and `accumulate_u8x8` in `crates/vip9r/src/tile_syntax/inter_predict.rs`.

**Estimated cost:** The three subpel-heavy regions total about **37.3 ms/frame on jellyfish** and **~12.9 ms/frame on BBB**.  The spill/reload traffic is only part of those regions, but it sits inside the per-row/per-vector convolution loops.  A realistic opportunity might be a few ms/frame on jellyfish if register pressure can be reduced.

**Why suboptimal on A55:** The intended work is eight-tap MACs.  The generated loop also uses the stack as a temporary backing store for six Q-register coefficient vectors and reloads them mid-iteration.  On A55, ASIMD loads have forwarding latency and compete with the actual pixel loads; extra stack traffic is especially expensive on an in-order core.

**Remedy sketch:** Try lower-register-pressure source shapes: separate 4-wide and 8-wide kernels, compute low/high halves in separate helper calls, keep coefficients as scalar lanes and `vdup` closer to use, or otherwise prevent the whole horizontal/vertical/write path from being one giant V8 register-allocation problem.  A source fix is plausible but not guaranteed because wasm SIMD register allocation is ultimately V8-owned.

**Tag:** mixed source/engine.

**Confidence:** High for the code shape; medium for fixability and exact impact.

---

## 4. Redundant clamp sequences around saturating narrows in SIMD pack paths

**Offsets:** repeated in subpel and reconstruction/transform pack paths, e.g. `0x5a2c-0x5a5c`, `0x5c34-0x5cac`, `0x9290-0x92c0`, `0x94c8-0x9540`, `0x12e4c-0x12e98`, `0x19bb8-0x19c10`.

**Representative asm:**

```asm
9290: f3b62282  vqmovn.s32   d2, q1
9294: f3b63284  vqmovn.s32   d3, q2
9298: f3044154  veor         q2, q2, q2
929c: f2122644  vmax.s16     q1, q1, q2
92a0: e300c0ff  movw         ip, #255
92a4: e340c0ff  movt         ip, #255
92a8: ec4ccb16  vmov         d6, ip, ip
92ac: e300c0ff  movw         ip, #255
92b0: e340c0ff  movt         ip, #255
92b4: ec4ccb17  vmov         d7, ip, ip
92b8: f2122656  vmin.s16     q1, q1, q3
92bc: f3b22242  vqmovun.s16  d2, q1
92c0: f3b23244  vqmovun.s16  d3, q2
```

Another form clamps to `[0, 65535]` before `vqmovun.s32`:

```asm
12e4c: e308c000  movw        ip, #32768
12e54: ec4ccb12  vmov        d2, ip, ip
12e64: f2200642  vmax.s32    q0, q0, q1
12e68: e307cfff  movw        ip, #32767
12e78: f2200652  vmin.s32    q0, q0, q1
12e90: f3b60240  vqmovun.s32 d0, q0
```

**Source attribution:** `round_shift_pack_u8` in `inter_predict.rs`, and `add_residual_8` / `add_residual_4` in `residual.rs`, all using wasm SIMD narrowing intrinsics such as `i16x8_narrow_i32x4` and `u8x16_narrow_i16x8`.

**Estimated cost:** Present in the largest jellyfish subpel regions and in BBB reconstruction/transform tails.  I would expect **~1-3 ms/frame opportunity on jellyfish** and smaller but nonzero BBB impact if V8 generated direct narrows without the explicit min/max/constant materialization.

**Why suboptimal on A55:** The final NEON instructions (`VQMOVN`/`VQMOVUN`) are already saturating narrows.  The preceding clamp materializes constants using integer `movw`/`movt` + `vmov` pairs and adds several vector min/max instructions per output vector.  These instructions are all in the inner loops.

**Remedy sketch:** This looks primarily like V8 arm32 wasm SIMD lowering.  The source already asks for saturating wasm narrows, so normal Rust restructuring may not remove it.  A future experiment could check whether a different intrinsic sequence (for example one direct unsigned narrow level, or packing through signed narrows after proving nonnegative) changes V8 lowering, but this is probably engine-level.

**Tag:** engine-level, with possible source workaround exploration.

**Confidence:** High for redundancy/code shape; medium for speedup estimate.

---

## 5. Scalar inverse-transform tails contain repeated checked narrow/range tests

**Offsets:** `0x17b00-0x18a00` and parts of `0x12c00-0x14d00`; related scalar/reconstruction tails at `0x19900-0x19e00`.

**Representative asm:**

```asm
17b0c: e0940007  adds    r0, r4, r7
17b10: e0a89002  adc     r9, r8, r2
17b14: e1a04720  lsr     r4, r0, #14
17b18: e1844909  orr     r4, r4, r9, lsl #18
17b1c: e1a06749  asr     r6, r9, #14
17b20: e0940001  adds    r0, r4, r1        ; add INT_MIN low
17b24: e0a69003  adc     r9, r6, r3        ; range test high
17b28: e3790001  cmn     r9, #1
...
17b50: e1969009  orrs    r9, r6, r9
17b54: 1a0023a2  bne     0x209e4           ; InvalidBitstream on overflow
17b58: e3019524  movw    r9, #5412
17b5c: e7854009  str     r4, [r5, r9]
```

**Source attribution:** scalar transform helpers in `crates/vip9r/src/tile_syntax/residual.rs`, especially `narrow_i32(round2_i64(...))?` in `sh` and `inverse_adst4`.  The file already uses unchecked-but-safe `narrow_i32_butterfly` in the hotter butterfly helper, but not in all scalar ADST/shift paths.

**Estimated cost:** These regions are smaller than token/subpel: about **2.2 ms/frame BBB** for `0x17b00-0x18a00`, **2.7 ms/frame jellyfish** for `0x19900-0x19e00`, and additional transform/reconstruct time in `0x12c00-0x14d00`.  The checked-narrow chains are a subset; likely sub-ms to low-ms opportunity.

**Why suboptimal on A55:** A single conceptual “narrow i64 to i32 after rounded shift” becomes a long carry/compare/branch chain.  It is repeated in scalar transform lanes and interleaved with 64-bit multiply emulation (`umull` + `mla`).  The branches should almost never be taken for conformant streams.

**Remedy sketch:** If VP9 bounds make these intermediates guaranteed i32 for valid streams, extend the existing `narrow_i32_butterfly` trust model to the remaining scalar ADST/shift paths, or add a debug/assert-only check outside the inner transform math.  Another route is SIMD coverage for the remaining ADST/tail cases.  Correctness risk is malformed-stream behavior, so this needs spec review and tests.

**Tag:** source-level.

**Confidence:** Medium-high for attribution; medium for acceptable remedy.

---

## 6. Large by-value context/materialization copies in prediction setup

**Offsets:** `0x00730-0x00a90`, `0x00b160-0x00b2b0`, and setup portions of `0x0b000-0x0d200`.

**Representative asm:**

```asm
970:  e1a0c00d  mov     ip, sp
974:  e24dd004  sub     sp, sp, #4
...
988:  e3a020f0  mov     r2, #240
98c:  e1a00005  mov     r0, r5
990:  e1a01004  mov     r1, r4
994:  e59fc858  ldr     ip, [pc, #2136]
9bc:  e12fff3c  blx     ip              ; copy helper
...
a5c:  e30f2ea0  movw    r2, #65184
...
a68:  f4020a0f  vst1.8  {d0-d1}, [r2]
```

There are also long sequences of field stores/reloads while building inlined prediction contexts and temporary copies.

**Source attribution:** `decode_residual` calls `predict_inter` / `predict_intra` with `InterPredictionContext` / `IntraPredictionContext`, each of which carries `block: DecodedBlockInfo` by value.  The inlined pipeline also constructs request/state structs for prediction helpers.

**Estimated cost:** The early region costs **~3.4 ms/frame jellyfish** and **~2.0 ms/frame BBB**.  Only part is copying/materialization, so this is lower-ranked, likely below ~1 ms/frame unless the source shape unlocks register-pressure improvements elsewhere.

**Why suboptimal on A55:** Bulk copies and many individual frame stores are load/store-pipeline work before the actual prediction or entropy math.  They also increase live state in the monolith, contributing to later spills.

**Remedy sketch:** Consider passing `DecodedBlockInfo` by reference, or splitting prediction contexts into the few scalar fields actually needed by each helper (`ref_frames`, MVs, filter, modes) instead of carrying the whole block through by value.  The risk is that previous traversal/glue restructuring has measured poorly, so this should be tested only if paired with asm confirmation that the large copy disappears.

**Tag:** source-level, but V8 inlining may limit benefit.

**Confidence:** Medium-low.  The copy is real; exact source object and net performance impact need a focused experiment.

---

## Non-findings / checked items

- I did **not** find general integer division in the subpel filters or traversal code.  The important divide is the single `sdiv` in fused dequantization; no `udiv` appeared in `decode_residual`.
- The hot subpel regions do show the already-known V8 arm32 SIMD issues (`extmul` not becoming `VMLAL`, Q-form SIMD, no clever shuffle lowering).  I did not count those as new findings.
- The dense compare/branch pairs before heap accesses are the known arm32 wasm bounds checks and are not re-reported above.
- The `read_bool` decision branch is present throughout the token loop, but I did not identify a new branchless opportunity beyond the already-measured null result.
- Standalone `predict_inter` / `inter_predict_subpel_unscaled_block` functions remain cold in the profile; the live code audited here is the inlined copy inside wasm[23].
- The integer-copy/average rows around the subpel tails look mostly like expected memory copy / `vrhadd.u8` work; no obvious divide or scalar per-pixel fallback dominated those snippets.

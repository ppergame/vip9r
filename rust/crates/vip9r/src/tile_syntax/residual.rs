use crate::header::{SEG_LVL_ALT_Q, SegmentationParams, UncompressedFrameHeader};

use super::{
    CurrentFrameMut, CurrentPlaneMut, MAX_TX_COEFFS, TileSyntaxError, TxSize, TxType, clip1,
};

use core::arch::wasm32::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TransformBlock {
    pub(super) tx_size: TxSize,
    pub(super) tx_type: TxType,
    pub(super) plane: usize,
    pub(super) start: (usize, usize),
}

impl TransformBlock {
    pub(super) const fn new(
        plane: usize,
        start: (usize, usize),
        tx_size: TxSize,
        tx_type: TxType,
    ) -> Self {
        Self {
            tx_size,
            tx_type,
            plane,
            start,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct DequantizedCoefficients {
    pub(super) block: TransformBlock,
    pub(super) coefficients: [i32; MAX_TX_COEFFS],
    pub(super) eob: usize,
    pub(super) nonzero_row_mask: u32,
    dct_simd_scratch: [v128; MAX_TX_WIDTH],
    dct_scalar_scratch: [i32; MAX_TX_WIDTH],
    adst_copy: [v128; MAX_ADST_WIDTH],
    adst_s_lo: [v128; MAX_ADST_WIDTH],
    adst_s_hi: [v128; MAX_ADST_WIDTH],
    adst_scalar_copy: [i32; MAX_ADST_WIDTH],
    adst_scalar_s: [i64; MAX_ADST_WIDTH],
}

impl DequantizedCoefficients {
    pub(super) fn empty() -> Self {
        Self {
            block: TransformBlock::new(0, (0, 0), TxSize::Tx4x4, TxType::DctDct),
            coefficients: [0; MAX_TX_COEFFS],
            eob: 0,
            nonzero_row_mask: 0,
            dct_simd_scratch: [i32x4_splat(0); MAX_TX_WIDTH],
            dct_scalar_scratch: [0; MAX_TX_WIDTH],
            adst_copy: [i32x4_splat(0); MAX_ADST_WIDTH],
            adst_s_lo: [i64x2_splat(0); MAX_ADST_WIDTH],
            adst_s_hi: [i64x2_splat(0); MAX_ADST_WIDTH],
            adst_scalar_copy: [0; MAX_ADST_WIDTH],
            adst_scalar_s: [0; MAX_ADST_WIDTH],
        }
    }

    #[cfg(feature = "wasm-tests")]
    fn new(block: TransformBlock, eob: usize) -> Self {
        Self {
            block,
            coefficients: [0; MAX_TX_COEFFS],
            eob,
            nonzero_row_mask: 0,
            dct_simd_scratch: [i32x4_splat(0); MAX_TX_WIDTH],
            dct_scalar_scratch: [0; MAX_TX_WIDTH],
            adst_copy: [i32x4_splat(0); MAX_ADST_WIDTH],
            adst_s_lo: [i64x2_splat(0); MAX_ADST_WIDTH],
            adst_s_hi: [i64x2_splat(0); MAX_ADST_WIDTH],
            adst_scalar_copy: [0; MAX_ADST_WIDTH],
            adst_scalar_s: [0; MAX_ADST_WIDTH],
        }
    }

    pub(super) fn reset(&mut self, block: TransformBlock, eob: usize) {
        self.block = block;
        self.eob = eob;
        self.nonzero_row_mask = 0;
    }

    pub(super) fn set_signed_dequantized(
        &mut self,
        pos: usize,
        magnitude: u32,
        sign_bit: u32,
        dc_quant: i32,
        ac_quant: i32,
        dq_denom: i32,
    ) -> Result<(), TileSyntaxError> {
        if pos >= coefficient_count(self.block.tx_size) {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let magnitude = i16::try_from(magnitude).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let coefficient = if sign_bit == 0 {
            magnitude
        } else {
            magnitude
                .checked_neg()
                .ok_or(TileSyntaxError::InvalidBitstream)?
        };
        if coefficient == 0 {
            return Ok(());
        }

        let quant = if pos == 0 { dc_quant } else { ac_quant };
        self.coefficients[pos] = (i32::from(coefficient) * quant) / dq_denom;
        self.nonzero_row_mask |= 1u32 << (pos >> (2 + self.block.tx_size.index()));
        Ok(())
    }

    pub(super) fn set_eob(&mut self, eob: usize) -> Result<(), TileSyntaxError> {
        if eob > coefficient_count(self.block.tx_size) {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        self.eob = eob;
        Ok(())
    }

    pub(super) const fn nonzero_context(&self) -> bool {
        self.eob > 0
    }

    pub(super) fn clear_transform_extent(&mut self) {
        let count = coefficient_count(self.block.tx_size);
        self.coefficients[..count].fill(0);
        self.eob = 0;
        self.nonzero_row_mask = 0;
    }

    pub(super) fn inverse_transform(&mut self, lossless: bool) -> Result<(), TileSyntaxError> {
        if lossless {
            if self.block.tx_size != TxSize::Tx4x4 || self.block.tx_type != TxType::DctDct {
                return Err(TileSyntaxError::InvalidBitstream);
            }
        } else if self.block.tx_size == TxSize::Tx32x32 && self.block.tx_type != TxType::DctDct {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let n = 2 + self.block.tx_size.index();
        let width = transform_width(self.block.tx_size);

        if self.eob == 0 {
            return Ok(());
        }

        if !lossless && self.block.tx_type == TxType::DctDct && self.eob == 1 {
            return self.inverse_dct_dct_dc_only(n, width);
        }

        self.inverse_transform_2d(lossless, n, width, self.nonzero_row_mask)
    }

    #[cfg(feature = "wasm-tests")]
    fn inverse_transform_scalar(&mut self, lossless: bool) -> Result<(), TileSyntaxError> {
        if lossless {
            if self.block.tx_size != TxSize::Tx4x4 || self.block.tx_type != TxType::DctDct {
                return Err(TileSyntaxError::InvalidBitstream);
            }
        } else if self.block.tx_size == TxSize::Tx32x32 && self.block.tx_type != TxType::DctDct {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let n = 2 + self.block.tx_size.index();
        let width = transform_width(self.block.tx_size);

        if self.eob == 0 {
            return Ok(());
        }

        if !lossless && self.block.tx_type == TxType::DctDct && self.eob == 1 {
            return self.inverse_dct_dct_dc_only(n, width);
        }

        self.inverse_transform_2d_scalar(lossless, n, width, self.nonzero_row_mask)
    }

    fn inverse_transform_2d(
        &mut self,
        lossless: bool,
        n: usize,
        width: usize,
        nonzero_row_mask: u32,
    ) -> Result<(), TileSyntaxError> {
        if !lossless {
            return self.inverse_transform_2d_simd(n, width, nonzero_row_mask);
        }

        self.inverse_transform_2d_scalar(lossless, n, width, nonzero_row_mask)
    }

    fn inverse_transform_2d_scalar(
        &mut self,
        lossless: bool,
        n: usize,
        width: usize,
        nonzero_row_mask: u32,
    ) -> Result<(), TileSyntaxError> {
        let t = &mut self.dct_scalar_scratch;
        let adst_copy = &mut self.adst_scalar_copy;
        let adst_s = &mut self.adst_scalar_s;
        let active_rows = width.min(u32::BITS as usize - nonzero_row_mask.leading_zeros() as usize);

        for row in 0..active_rows {
            if nonzero_row_mask & (1u32 << row) == 0 {
                continue;
            }

            for (col, slot) in t.iter_mut().take(width).enumerate() {
                *slot = self.coefficients[row * width + col];
            }

            if lossless {
                inverse_wht(t, 2)?;
            } else if row_transform_is_dct(self.block.tx_type) {
                inverse_dct_permutation(t, n);
                inverse_dct(t, n)?;
            } else {
                inverse_adst_with_scratch(t, n, adst_s, adst_copy)?;
            }

            for (col, value) in t.iter().take(width).copied().enumerate() {
                self.coefficients[row * width + col] = value;
            }
        }

        let final_shift = core::cmp::min(6, n + 2);
        for col in 0..width {
            for (row, slot) in t.iter_mut().take(width).enumerate() {
                *slot = self.coefficients[row * width + col];
            }

            if lossless {
                inverse_wht(t, 0)?;
            } else if column_transform_is_dct(self.block.tx_type) {
                inverse_dct_permutation(t, n);
                inverse_dct(t, n)?;
            } else {
                inverse_adst_with_scratch(t, n, adst_s, adst_copy)?;
            }

            for (row, value) in t.iter().take(width).copied().enumerate() {
                self.coefficients[row * width + col] = if lossless {
                    value
                } else {
                    narrow_i32(round2_i64(i64::from(value), final_shift))?
                };
            }
        }

        Ok(())
    }

    fn inverse_transform_2d_simd(
        &mut self,
        n: usize,
        width: usize,
        nonzero_row_mask: u32,
    ) -> Result<(), TileSyntaxError> {
        if self.block.tx_type == TxType::DctDct {
            return self.inverse_transform_2d_dct_dct_simd_i16(n, width, nonzero_row_mask);
        }

        let row_transform = if row_transform_is_dct(self.block.tx_type) {
            InverseTransform1d::Dct
        } else {
            InverseTransform1d::Adst
        };
        let column_transform = if column_transform_is_dct(self.block.tx_type) {
            InverseTransform1d::Dct
        } else {
            InverseTransform1d::Adst
        };

        self.inverse_transform_rows_simd(n, width, nonzero_row_mask, row_transform)?;
        self.inverse_transform_columns_simd(n, width, column_transform)
    }

    fn inverse_transform_2d_dct_dct_simd_i16(
        &mut self,
        n: usize,
        width: usize,
        nonzero_row_mask: u32,
    ) -> Result<(), TileSyntaxError> {
        self.inverse_transform_rows_dct_dct_simd_i16(n, width, nonzero_row_mask)?;
        self.inverse_transform_columns_dct_dct_simd_i16(n, width)
    }

    fn inverse_transform_rows_dct_dct_simd_i16(
        &mut self,
        n: usize,
        width: usize,
        nonzero_row_mask: u32,
    ) -> Result<(), TileSyntaxError> {
        let active_rows = width.min(u32::BITS as usize - nonzero_row_mask.leading_zeros() as usize);
        let mut rows = [0usize; SIMD_DCT_I16_LANES];
        let mut row_count = 0usize;

        for row in 0..active_rows {
            if nonzero_row_mask & (1u32 << row) == 0 {
                continue;
            }

            rows[row_count] = row;
            row_count += 1;
            if row_count == SIMD_DCT_I16_LANES {
                self.inverse_transform_row_group_dct_dct_simd_i16(n, width, rows)?;
                row_count = 0;
            }
        }

        if row_count >= SIMD_TRANSFORM_LANES {
            self.inverse_transform_row_half_group_dct_dct_simd_i16(
                n,
                width,
                [rows[0], rows[1], rows[2], rows[3]],
            )?;
            rows.copy_within(SIMD_TRANSFORM_LANES..row_count, 0);
            row_count -= SIMD_TRANSFORM_LANES;
        }

        // Rows left over after the eight-lane and optional four-lane i16
        // groups use the same scalar tail policy as the four-lane SIMD driver.
        // The DCT reads exactly `width` entries and the brev-fused gather
        // overwrites every entry it reads.
        let t = &mut self.dct_scalar_scratch;
        let coefficients = &mut self.coefficients;
        for &row in rows.iter().take(row_count) {
            for col in 0..width {
                t[brev(n, col)] = coefficients[row * width + col];
            }
            inverse_dct(t, n)?;
            for (col, value) in t.iter().take(width).copied().enumerate() {
                coefficients[row * width + col] = value;
            }
        }

        Ok(())
    }

    fn inverse_transform_row_half_group_dct_dct_simd_i16(
        &mut self,
        n: usize,
        width: usize,
        rows: [usize; SIMD_TRANSFORM_LANES],
    ) -> Result<(), TileSyntaxError> {
        let t = &mut self.dct_simd_scratch;
        let coefficients = &mut self.coefficients;

        // Same i16 DCT kernel as full groups, with only the low four lanes
        // populated. This recovers the old four-row SIMD tail for common 4x4
        // DCT_DCT blocks while keeping the full eight-lane path dominant.
        for col in 0..width {
            let lo = i32x4_from_lanes(
                coefficients[rows[0] * width + col],
                coefficients[rows[1] * width + col],
                coefficients[rows[2] * width + col],
                coefficients[rows[3] * width + col],
            );
            t[brev(n, col)] = i16x8_narrow_i32x4(lo, i32x4_splat(0));
        }
        inverse_dct_simd_i16(t, n)?;

        for (col, &values) in t.iter().take(width).enumerate() {
            let lo = i32x4_extend_low_i16x8(values);
            coefficients[rows[0] * width + col] = i32x4_extract_lane::<0>(lo);
            coefficients[rows[1] * width + col] = i32x4_extract_lane::<1>(lo);
            coefficients[rows[2] * width + col] = i32x4_extract_lane::<2>(lo);
            coefficients[rows[3] * width + col] = i32x4_extract_lane::<3>(lo);
        }

        Ok(())
    }

    fn inverse_transform_row_group_dct_dct_simd_i16(
        &mut self,
        n: usize,
        width: usize,
        rows: [usize; SIMD_DCT_I16_LANES],
    ) -> Result<(), TileSyntaxError> {
        let t = &mut self.dct_simd_scratch;
        let coefficients = &mut self.coefficients;

        // Gather i32 coefficients into signed i16 lanes. For conforming VP9
        // bitstreams these are the first T-array writes and must fit i16 at
        // 8-bit depth; saturating narrow keeps malformed streams defined.
        for col in 0..width {
            let lo = i32x4_from_lanes(
                coefficients[rows[0] * width + col],
                coefficients[rows[1] * width + col],
                coefficients[rows[2] * width + col],
                coefficients[rows[3] * width + col],
            );
            let hi = i32x4_from_lanes(
                coefficients[rows[4] * width + col],
                coefficients[rows[5] * width + col],
                coefficients[rows[6] * width + col],
                coefficients[rows[7] * width + col],
            );
            t[brev(n, col)] = i16x8_narrow_i32x4(lo, hi);
        }
        inverse_dct_simd_i16(t, n)?;

        for (col, &values) in t.iter().take(width).enumerate() {
            let lo = i32x4_extend_low_i16x8(values);
            let hi = i32x4_extend_high_i16x8(values);
            coefficients[rows[0] * width + col] = i32x4_extract_lane::<0>(lo);
            coefficients[rows[1] * width + col] = i32x4_extract_lane::<1>(lo);
            coefficients[rows[2] * width + col] = i32x4_extract_lane::<2>(lo);
            coefficients[rows[3] * width + col] = i32x4_extract_lane::<3>(lo);
            coefficients[rows[4] * width + col] = i32x4_extract_lane::<0>(hi);
            coefficients[rows[5] * width + col] = i32x4_extract_lane::<1>(hi);
            coefficients[rows[6] * width + col] = i32x4_extract_lane::<2>(hi);
            coefficients[rows[7] * width + col] = i32x4_extract_lane::<3>(hi);
        }

        Ok(())
    }

    fn inverse_transform_columns_dct_dct_simd_i16(
        &mut self,
        n: usize,
        width: usize,
    ) -> Result<(), TileSyntaxError> {
        let final_shift = core::cmp::min(6, n + 2);
        let t = &mut self.dct_simd_scratch;
        let coefficients = &mut self.coefficients;

        if width == 4 {
            // 4x4 DCT_DCT has no full eight-column group. Use the low half of
            // one i16x8 group and keep the high lanes zero/stale-unused.
            for row in 0..width {
                t[brev(n, row)] = i16x8_narrow_i32x4(
                    load_i32x4(coefficients.as_ptr().wrapping_add(row * width)),
                    i32x4_splat(0),
                );
            }
            inverse_dct_simd_i16(t, n)?;

            for (row, &values) in t.iter().take(width).enumerate() {
                let values = round_shift_i16x8_low_to_i32x4(values, final_shift);
                store_i32x4(coefficients.as_mut_ptr().wrapping_add(row * width), values);
            }
            return Ok(());
        }

        for col in (0..width).step_by(SIMD_DCT_I16_LANES) {
            // Gather this eight-column group directly into bit-reversed DCT
            // input order. All entries read by the i16 SIMD DCT are
            // overwritten here.
            for row in 0..width {
                let row_base = coefficients.as_ptr().wrapping_add(row * width + col);
                t[brev(n, row)] = i16x8_narrow_i32x4(
                    load_i32x4(row_base),
                    load_i32x4(row_base.wrapping_add(SIMD_TRANSFORM_LANES)),
                );
            }
            inverse_dct_simd_i16(t, n)?;

            for (row, &values) in t.iter().take(width).enumerate() {
                let (lo, hi) = round_shift_i16x8_to_i32x4(values, final_shift);
                let row_base = coefficients.as_mut_ptr().wrapping_add(row * width + col);
                store_i32x4(row_base, lo);
                store_i32x4(row_base.wrapping_add(SIMD_TRANSFORM_LANES), hi);
            }
        }

        Ok(())
    }

    fn inverse_transform_rows_simd(
        &mut self,
        n: usize,
        width: usize,
        nonzero_row_mask: u32,
        transform: InverseTransform1d,
    ) -> Result<(), TileSyntaxError> {
        let active_rows = width.min(u32::BITS as usize - nonzero_row_mask.leading_zeros() as usize);
        let mut rows = [0usize; SIMD_TRANSFORM_LANES];
        let mut row_count = 0usize;

        for row in 0..active_rows {
            if nonzero_row_mask & (1u32 << row) == 0 {
                continue;
            }

            rows[row_count] = row;
            row_count += 1;
            if row_count == SIMD_TRANSFORM_LANES {
                self.inverse_transform_row_group_simd(n, width, rows, transform)?;
                row_count = 0;
            }
        }

        let t = &mut self.dct_scalar_scratch;
        let adst_copy = &mut self.adst_scalar_copy;
        let adst_s = &mut self.adst_scalar_s;
        let coefficients = &mut self.coefficients;
        for &row in rows.iter().take(row_count) {
            match transform {
                InverseTransform1d::Dct => {
                    // Write the gathered row in bit-reversed order, matching
                    // inverse_dct_permutation without a separate scratch copy.
                    for col in 0..width {
                        t[brev(n, col)] = coefficients[row * width + col];
                    }
                    inverse_dct(t, n)?;
                }
                InverseTransform1d::Adst => {
                    for col in 0..width {
                        t[col] = coefficients[row * width + col];
                    }
                    inverse_adst_with_scratch(t, n, adst_s, adst_copy)?;
                }
            }
            for (col, value) in t.iter().take(width).copied().enumerate() {
                coefficients[row * width + col] = value;
            }
        }

        Ok(())
    }

    fn inverse_transform_row_group_simd(
        &mut self,
        n: usize,
        width: usize,
        rows: [usize; SIMD_TRANSFORM_LANES],
        transform: InverseTransform1d,
    ) -> Result<(), TileSyntaxError> {
        let t = &mut self.dct_simd_scratch;
        let adst_copy = &mut self.adst_copy;
        let adst_s_lo = &mut self.adst_s_lo;
        let adst_s_hi = &mut self.adst_s_hi;
        let coefficients = &mut self.coefficients;

        match transform {
            InverseTransform1d::Dct => {
                // The DCT reads exactly `width == 1 << n` entries. Fill those
                // entries in their permuted positions; persistent scratch
                // beyond width may be stale and must remain unused.
                for col in 0..width {
                    t[brev(n, col)] = i32x4_from_lanes(
                        coefficients[rows[0] * width + col],
                        coefficients[rows[1] * width + col],
                        coefficients[rows[2] * width + col],
                        coefficients[rows[3] * width + col],
                    );
                }
                inverse_dct_simd(t, n)?;
            }
            InverseTransform1d::Adst => {
                // The ADST SIMD kernels perform their own input permutation,
                // so gather in natural order. As with DCT, all entries read by
                // the transform are overwritten for this group.
                for col in 0..width {
                    t[col] = i32x4_from_lanes(
                        coefficients[rows[0] * width + col],
                        coefficients[rows[1] * width + col],
                        coefficients[rows[2] * width + col],
                        coefficients[rows[3] * width + col],
                    );
                }
                inverse_adst_simd(t, n, adst_s_lo, adst_s_hi, adst_copy)?;
            }
        }

        for (col, &values) in t.iter().take(width).enumerate() {
            coefficients[rows[0] * width + col] = i32x4_extract_lane::<0>(values);
            coefficients[rows[1] * width + col] = i32x4_extract_lane::<1>(values);
            coefficients[rows[2] * width + col] = i32x4_extract_lane::<2>(values);
            coefficients[rows[3] * width + col] = i32x4_extract_lane::<3>(values);
        }

        Ok(())
    }

    fn inverse_transform_columns_simd(
        &mut self,
        n: usize,
        width: usize,
        transform: InverseTransform1d,
    ) -> Result<(), TileSyntaxError> {
        let final_shift = core::cmp::min(6, n + 2);
        let t = &mut self.dct_simd_scratch;
        let adst_copy = &mut self.adst_copy;
        let adst_s_lo = &mut self.adst_s_lo;
        let adst_s_hi = &mut self.adst_s_hi;
        let coefficients = &mut self.coefficients;

        for col in (0..width).step_by(SIMD_TRANSFORM_LANES) {
            match transform {
                InverseTransform1d::Dct => {
                    // Gather this column group directly into bit-reversed DCT
                    // input order. All entries the SIMD DCT reads are
                    // overwritten here.
                    for row in 0..width {
                        t[brev(n, row)] =
                            load_i32x4(coefficients.as_ptr().wrapping_add(row * width + col));
                    }
                    inverse_dct_simd(t, n)?;
                }
                InverseTransform1d::Adst => {
                    for (row, slot) in t.iter_mut().take(width).enumerate() {
                        *slot = load_i32x4(coefficients.as_ptr().wrapping_add(row * width + col));
                    }
                    inverse_adst_simd(t, n, adst_s_lo, adst_s_hi, adst_copy)?;
                }
            }

            for (row, &values) in t.iter().take(width).enumerate() {
                // Scalar narrows `round2_i64(i64::from(value), final_shift)`.
                // Since `value` is already i32 and final_shift is 4..=6 here,
                // that checked narrow cannot fail. Keep the rounding in i64
                // lanes anyway so malformed streams near i32::MAX do not get
                // an extra pre-shift i32 wrap.
                let values = round_shift_i32x4(values, final_shift);
                store_i32x4(
                    coefficients.as_mut_ptr().wrapping_add(row * width + col),
                    values,
                );
            }
        }

        Ok(())
    }

    fn inverse_dct_dct_dc_only(&mut self, n: usize, width: usize) -> Result<(), TileSyntaxError> {
        let pass1 = inverse_dct_dc_value(self.coefficients[0]);
        let pass2 = inverse_dct_dc_value(pass1);
        let final_shift = core::cmp::min(6, n + 2);
        let value = narrow_i32(round2_i64(i64::from(pass2), final_shift))?;

        self.coefficients[..width * width].fill(value);
        Ok(())
    }
}

impl PartialEq for DequantizedCoefficients {
    fn eq(&self, other: &Self) -> bool {
        self.block == other.block
            && self.coefficients == other.coefficients
            && self.eob == other.eob
            && self.nonzero_row_mask == other.nonzero_row_mask
    }
}

impl Eq for DequantizedCoefficients {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FrameDequant {
    base_q_idx: u8,
    delta_q_y_dc: i32,
    delta_q_uv_dc: i32,
    delta_q_uv_ac: i32,
    segmentation: SegmentationParams,
}

impl FrameDequant {
    #[cfg(feature = "wasm-tests")]
    pub(super) const fn new(
        base_q_idx: u8,
        delta_q_y_dc: i32,
        delta_q_uv_dc: i32,
        delta_q_uv_ac: i32,
    ) -> Self {
        Self {
            base_q_idx,
            delta_q_y_dc,
            delta_q_uv_dc,
            delta_q_uv_ac,
            segmentation: SegmentationParams::disabled(),
        }
    }

    pub(super) const fn new_with_segmentation(
        base_q_idx: u8,
        delta_q_y_dc: i32,
        delta_q_uv_dc: i32,
        delta_q_uv_ac: i32,
        segmentation: SegmentationParams,
    ) -> Self {
        Self {
            base_q_idx,
            delta_q_y_dc,
            delta_q_uv_dc,
            delta_q_uv_ac,
            segmentation,
        }
    }

    pub(super) const fn from_header(header: &UncompressedFrameHeader) -> Self {
        Self::new_with_segmentation(
            header.base_q_idx,
            header.delta_q_y_dc,
            header.delta_q_uv_dc,
            header.delta_q_uv_ac,
            header.segmentation,
        )
    }

    #[cfg(feature = "wasm-tests")]
    pub(super) const fn get_qindex(self) -> i32 {
        self.get_qindex_for_segment(0)
    }

    pub(super) const fn get_qindex_for_segment(self, segment_id: u8) -> i32 {
        if self.segmentation.enabled {
            let segment_index = segment_id as usize;
            if segment_index < self.segmentation.feature_enabled.len()
                && self.segmentation.feature_enabled[segment_index][SEG_LVL_ALT_Q]
            {
                let mut data = self.segmentation.feature_data[segment_index][SEG_LVL_ALT_Q] as i32;
                if !self.segmentation.abs_or_delta_update {
                    data += self.base_q_idx as i32;
                }
                return if data < 0 {
                    0
                } else if data > 255 {
                    255
                } else {
                    data
                };
            }
        }

        self.base_q_idx as i32
    }

    #[cfg(feature = "wasm-tests")]
    pub(super) fn get_dc_quant(self, plane: usize) -> i32 {
        let delta = if plane == 0 {
            self.delta_q_y_dc
        } else {
            self.delta_q_uv_dc
        };
        dc_q(self.get_qindex() + delta)
    }

    pub(super) fn get_dc_quant_for_segment(self, plane: usize, segment_id: u8) -> i32 {
        let delta = if plane == 0 {
            self.delta_q_y_dc
        } else {
            self.delta_q_uv_dc
        };
        dc_q(self.get_qindex_for_segment(segment_id) + delta)
    }

    #[cfg(feature = "wasm-tests")]
    pub(super) fn get_ac_quant(self, plane: usize) -> i32 {
        let delta = if plane == 0 { 0 } else { self.delta_q_uv_ac };
        ac_q(self.get_qindex() + delta)
    }

    pub(super) fn get_ac_quant_for_segment(self, plane: usize, segment_id: u8) -> i32 {
        let delta = if plane == 0 { 0 } else { self.delta_q_uv_ac };
        ac_q(self.get_qindex_for_segment(segment_id) + delta)
    }
}

pub(super) const fn coefficient_count(tx_size: TxSize) -> usize {
    16 << (tx_size.index() << 1)
}

const MAX_TX_WIDTH: usize = 32;
const MAX_ADST_WIDTH: usize = 16;
const SIMD_TRANSFORM_LANES: usize = 4;
const SIMD_DCT_I16_LANES: usize = 8;

#[derive(Clone, Copy)]
enum InverseTransform1d {
    Dct,
    Adst,
}

const COS64_LOOKUP: [i32; 33] = [
    16384, 16364, 16305, 16207, 16069, 15893, 15679, 15426, 15137, 14811, 14449, 14053, 13623,
    13160, 12665, 12140, 11585, 11003, 10394, 9760, 9102, 8423, 7723, 7005, 6270, 5520, 4756, 3981,
    3196, 2404, 1606, 804, 0,
];

const SINPI_1_9: i64 = 5283;
const SINPI_2_9: i64 = 9929;
const SINPI_3_9: i64 = 13377;
const SINPI_4_9: i64 = 15212;

const fn transform_width(tx_size: TxSize) -> usize {
    4 << tx_size.index()
}

const fn row_transform_is_dct(tx_type: TxType) -> bool {
    matches!(tx_type, TxType::DctDct | TxType::AdstDct)
}

const fn column_transform_is_dct(tx_type: TxType) -> bool {
    matches!(tx_type, TxType::DctDct | TxType::DctAdst)
}

const fn brev(num_bits: usize, x: usize) -> usize {
    let mut t = 0usize;
    let mut i = 0usize;
    while i < num_bits {
        let bit = (x >> i) & 1;
        t += bit << (num_bits - 1 - i);
        i += 1;
    }
    t
}

const fn cos64(angle: i32) -> i32 {
    let angle2 = (angle & 127) as usize;
    if angle2 <= 32 {
        COS64_LOOKUP[angle2]
    } else if angle2 <= 64 {
        -COS64_LOOKUP[64 - angle2]
    } else if angle2 <= 96 {
        -COS64_LOOKUP[angle2 - 64]
    } else {
        COS64_LOOKUP[128 - angle2]
    }
}

const fn sin64(angle: i32) -> i32 {
    cos64(angle - 32)
}

fn round2_i64(value: i64, bits: usize) -> i64 {
    (value + (1i64 << (bits - 1))) >> bits
}

fn narrow_i32(value: i64) -> Result<i32, TileSyntaxError> {
    i32::try_from(value).map_err(|_| TileSyntaxError::InvalidBitstream)
}

// VP9 requires intermediate transform values to be representable as i32 for a
// conformant bitstream. The hot butterfly path trusts that guarantee: malformed
// streams that overflow here wrap through this cast and may produce wrong
// pixels instead of a decode error, but safe Rust still avoids UB.
#[inline(always)]
fn narrow_i32_butterfly(value: i64) -> i32 {
    value as i32
}

fn inverse_dct_dc_value(value: i32) -> i32 {
    narrow_i32_butterfly(round2_i64(i64::from(value) * i64::from(cos64(16)), 14))
}

#[inline(always)]
fn b(t: &mut [i32; MAX_TX_WIDTH], a: usize, b_index: usize, angle: i32, flip: bool) {
    let ta = i64::from(t[a]);
    let tb = i64::from(t[b_index]);
    let cos = i64::from(cos64(angle));
    let sin = i64::from(sin64(angle));
    let x = narrow_i32_butterfly(round2_i64(ta * cos - tb * sin, 14));
    let y = narrow_i32_butterfly(round2_i64(ta * sin + tb * cos, 14));

    if flip {
        t[a] = y;
        t[b_index] = x;
    } else {
        t[a] = x;
        t[b_index] = y;
    }
}

#[inline(always)]
fn h(t: &mut [i32; MAX_TX_WIDTH], a: usize, b_index: usize, flip: bool) {
    let (x_index, y_index) = if flip { (b_index, a) } else { (a, b_index) };
    let x = i64::from(t[x_index]);
    let y = i64::from(t[y_index]);
    t[x_index] = narrow_i32_butterfly(x + y);
    t[y_index] = narrow_i32_butterfly(x - y);
}

#[inline(always)]
fn i32x4_from_lanes(a: i32, b: i32, c: i32, d: i32) -> v128 {
    let value = i32x4_replace_lane::<0>(i32x4_splat(0), a);
    let value = i32x4_replace_lane::<1>(value, b);
    let value = i32x4_replace_lane::<2>(value, c);
    i32x4_replace_lane::<3>(value, d)
}

#[inline(always)]
fn load_i32x4(src: *const i32) -> v128 {
    // Wasm vector loads are byte-addressed and permit unaligned addresses. The
    // callers pass four in-bounds contiguous i32 coefficients.
    unsafe { v128_load(src.cast::<v128>()) }
}

#[inline(always)]
fn store_i32x4(dst: *mut i32, value: v128) {
    // Wasm vector stores are byte-addressed and permit unaligned addresses. The
    // callers pass four in-bounds contiguous i32 coefficient slots.
    unsafe { v128_store(dst.cast::<v128>(), value) }
}

#[inline(always)]
fn round_shift_i32x4_i16_domain(value: v128, bits: usize) -> v128 {
    let rounding = i32x4_splat(1i32 << (bits - 1));
    i32x4_shr(i32x4_add(value, rounding), bits as u32)
}

#[inline(always)]
fn round_shift_i16x8_to_i32x4(value: v128, bits: usize) -> (v128, v128) {
    (
        round_shift_i32x4_i16_domain(i32x4_extend_low_i16x8(value), bits),
        round_shift_i32x4_i16_domain(i32x4_extend_high_i16x8(value), bits),
    )
}

#[inline(always)]
fn round_shift_i16x8_low_to_i32x4(value: v128, bits: usize) -> v128 {
    round_shift_i32x4_i16_domain(i32x4_extend_low_i16x8(value), bits)
}

#[inline(always)]
fn pack_i64x2_low_i32s(lo: v128, hi: v128) -> v128 {
    i32x4_shuffle::<0, 2, 4, 6>(lo, hi)
}

#[inline(always)]
fn round_shift_i64x2(value: v128, bits: usize) -> v128 {
    let rounding = i64x2_splat(1i64 << (bits - 1));
    i64x2_shr(i64x2_add(value, rounding), bits as u32)
}

#[inline(always)]
fn round_shift_i32x4(value: v128, bits: usize) -> v128 {
    let lo = round_shift_i64x2(i64x2_extend_low_i32x4(value), bits);
    let hi = round_shift_i64x2(i64x2_extend_high_i32x4(value), bits);
    pack_i64x2_low_i32s(lo, hi)
}

#[inline(always)]
fn round_pack_i64x2_low_i32s(lo: v128, hi: v128, bits: usize) -> v128 {
    pack_i64x2_low_i32s(round_shift_i64x2(lo, bits), round_shift_i64x2(hi, bits))
}

#[inline(always)]
fn b_simd(t: &mut [v128; MAX_TX_WIDTH], a: usize, b_index: usize, angle: i32, flip: bool) {
    let ta = t[a];
    let tb = t[b_index];
    let cos = i32x4_splat(cos64(angle));
    let sin = i32x4_splat(sin64(angle));

    let x_lo = round_shift_i64x2(
        i64x2_sub(
            i64x2_extmul_low_i32x4(ta, cos),
            i64x2_extmul_low_i32x4(tb, sin),
        ),
        14,
    );
    let x_hi = round_shift_i64x2(
        i64x2_sub(
            i64x2_extmul_high_i32x4(ta, cos),
            i64x2_extmul_high_i32x4(tb, sin),
        ),
        14,
    );
    let y_lo = round_shift_i64x2(
        i64x2_add(
            i64x2_extmul_low_i32x4(ta, sin),
            i64x2_extmul_low_i32x4(tb, cos),
        ),
        14,
    );
    let y_hi = round_shift_i64x2(
        i64x2_add(
            i64x2_extmul_high_i32x4(ta, sin),
            i64x2_extmul_high_i32x4(tb, cos),
        ),
        14,
    );

    let x = pack_i64x2_low_i32s(x_lo, x_hi);
    let y = pack_i64x2_low_i32s(y_lo, y_hi);

    if flip {
        t[a] = y;
        t[b_index] = x;
    } else {
        t[a] = x;
        t[b_index] = y;
    }
}

#[inline(always)]
fn h_simd(t: &mut [v128; MAX_TX_WIDTH], a: usize, b_index: usize, flip: bool) {
    let (x_index, y_index) = if flip { (b_index, a) } else { (a, b_index) };
    let x = t[x_index];
    let y = t[y_index];
    t[x_index] = i32x4_add(x, y);
    t[y_index] = i32x4_sub(x, y);
}

#[inline(always)]
fn b_simd_i16(t: &mut [v128; MAX_TX_WIDTH], a: usize, b_index: usize, angle: i32, flip: bool) {
    let ta = t[a];
    let tb = t[b_index];
    let cos = i16x8_splat(cos64(angle) as i16);
    let sin = i16x8_splat(sin64(angle) as i16);

    let x_lo = round_shift_i32x4_i16_domain(
        i32x4_sub(
            i32x4_extmul_low_i16x8(ta, cos),
            i32x4_extmul_low_i16x8(tb, sin),
        ),
        14,
    );
    let x_hi = round_shift_i32x4_i16_domain(
        i32x4_sub(
            i32x4_extmul_high_i16x8(ta, cos),
            i32x4_extmul_high_i16x8(tb, sin),
        ),
        14,
    );
    let y_lo = round_shift_i32x4_i16_domain(
        i32x4_add(
            i32x4_extmul_low_i16x8(ta, sin),
            i32x4_extmul_low_i16x8(tb, cos),
        ),
        14,
    );
    let y_hi = round_shift_i32x4_i16_domain(
        i32x4_add(
            i32x4_extmul_high_i16x8(ta, sin),
            i32x4_extmul_high_i16x8(tb, cos),
        ),
        14,
    );

    let x = i16x8_narrow_i32x4(x_lo, x_hi);
    let y = i16x8_narrow_i32x4(y_lo, y_hi);

    if flip {
        t[a] = y;
        t[b_index] = x;
    } else {
        t[a] = x;
        t[b_index] = y;
    }
}

#[inline(always)]
fn h_simd_i16(t: &mut [v128; MAX_TX_WIDTH], a: usize, b_index: usize, flip: bool) {
    let (x_index, y_index) = if flip { (b_index, a) } else { (a, b_index) };
    let x = t[x_index];
    let y = t[y_index];
    t[x_index] = i16x8_add(x, y);
    t[y_index] = i16x8_sub(x, y);
}

fn sb_simd(
    t: &[v128; MAX_TX_WIDTH],
    s_lo: &mut [v128; MAX_ADST_WIDTH],
    s_hi: &mut [v128; MAX_ADST_WIDTH],
    a: usize,
    b_index: usize,
    angle: i32,
    flip: bool,
) {
    let ta = t[a];
    let tb = t[b_index];
    let cos = i32x4_splat(cos64(angle));
    let sin = i32x4_splat(sin64(angle));

    let x_lo = i64x2_sub(
        i64x2_extmul_low_i32x4(ta, cos),
        i64x2_extmul_low_i32x4(tb, sin),
    );
    let x_hi = i64x2_sub(
        i64x2_extmul_high_i32x4(ta, cos),
        i64x2_extmul_high_i32x4(tb, sin),
    );
    let y_lo = i64x2_add(
        i64x2_extmul_low_i32x4(ta, sin),
        i64x2_extmul_low_i32x4(tb, cos),
    );
    let y_hi = i64x2_add(
        i64x2_extmul_high_i32x4(ta, sin),
        i64x2_extmul_high_i32x4(tb, cos),
    );

    if flip {
        s_lo[a] = y_lo;
        s_hi[a] = y_hi;
        s_lo[b_index] = x_lo;
        s_hi[b_index] = x_hi;
    } else {
        s_lo[a] = x_lo;
        s_hi[a] = x_hi;
        s_lo[b_index] = y_lo;
        s_hi[b_index] = y_hi;
    }
}

fn sh_simd(
    t: &mut [v128; MAX_TX_WIDTH],
    s_lo: &[v128; MAX_ADST_WIDTH],
    s_hi: &[v128; MAX_ADST_WIDTH],
    a: usize,
    b_index: usize,
) {
    let a_lo = s_lo[a];
    let a_hi = s_hi[a];
    let b_lo = s_lo[b_index];
    let b_hi = s_hi[b_index];

    t[a] = round_pack_i64x2_low_i32s(i64x2_add(a_lo, b_lo), i64x2_add(a_hi, b_hi), 14);
    t[b_index] = round_pack_i64x2_low_i32s(i64x2_sub(a_lo, b_lo), i64x2_sub(a_hi, b_hi), 14);
}

fn sb(
    t: &[i32; MAX_TX_WIDTH],
    s: &mut [i64; MAX_ADST_WIDTH],
    a: usize,
    b_index: usize,
    angle: i32,
    flip: bool,
) {
    let ta = i64::from(t[a]);
    let tb = i64::from(t[b_index]);
    let cos = i64::from(cos64(angle));
    let sin = i64::from(sin64(angle));
    let x = ta * cos - tb * sin;
    let y = ta * sin + tb * cos;

    if flip {
        s[a] = y;
        s[b_index] = x;
    } else {
        s[a] = x;
        s[b_index] = y;
    }
}

fn sh(
    t: &mut [i32; MAX_TX_WIDTH],
    s: &[i64; MAX_ADST_WIDTH],
    a: usize,
    b_index: usize,
) -> Result<(), TileSyntaxError> {
    t[a] = narrow_i32(round2_i64(s[a] + s[b_index], 14))?;
    t[b_index] = narrow_i32(round2_i64(s[a] - s[b_index], 14))?;
    Ok(())
}

fn inverse_dct_permutation(t: &mut [i32; MAX_TX_WIDTH], n: usize) {
    let len = 1usize << n;
    for i in 0..len {
        let j = brev(n, i);
        if i < j {
            t.swap(i, j);
        }
    }
}

fn inverse_dct(t: &mut [i32; MAX_TX_WIDTH], n: usize) -> Result<(), TileSyntaxError> {
    if !(2..=5).contains(&n) {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    let n0 = 1usize << n;
    let n1 = 1usize << (n - 1);
    let n2 = 1usize << (n - 2);

    if n == 2 {
        b(t, 0, 1, 16, true);
    } else {
        inverse_dct(t, n - 1)?;
    }

    for i in 0..n2 {
        b(t, n1 + i, n0 - 1 - i, 32 - brev(5, n1 + i) as i32, false);
    }

    if n >= 3 {
        let n3 = 1usize << (n - 3);
        for i in 0..n3 {
            for j in 0..=1 {
                h(t, n1 + 4 * i + 2 * j, n1 + 1 + 4 * i + 2 * j, j != 0);
            }
        }
    }

    if n == 5 {
        let n3 = 1usize << (n - 3);
        for i in 0..=1 {
            for j in 0..=1 {
                b(
                    t,
                    n0 - n + 3 - n2 * j - 4 * i,
                    n1 + n - 4 + n2 * j + 4 * i,
                    28 - 16 * i as i32 + 56 * j as i32,
                    true,
                );
            }
        }
        for i in 0..=1 {
            for j in 0..=3 {
                h(t, n1 + n3 * j + i, n1 + n2 - 5 + n3 * j - i, (j & 1) != 0);
            }
        }
    }

    if n >= 4 {
        for i in 0..=usize::from(n == 5) {
            for j in 0..=1 {
                b(
                    t,
                    n0 - n + 2 - i - n2 * j,
                    n1 + n - 3 + i + n2 * j,
                    24 + 48 * j as i32,
                    true,
                );
            }
        }
        for i in 0..=(2 * n - 7) {
            for j in 0..=1 {
                h(t, n1 + n2 * j + i, n1 + n2 - 1 + n2 * j - i, (j & 1) != 0);
            }
        }
    }

    if n >= 3 {
        let n3 = 1usize << (n - 3);
        for i in 0..n3 {
            b(t, n0 - n3 - 1 - i, n1 + n3 + i, 16, true);
        }
    }

    for i in 0..n1 {
        h(t, i, n0 - 1 - i, false);
    }

    Ok(())
}

fn inverse_dct_simd(t: &mut [v128; MAX_TX_WIDTH], n: usize) -> Result<(), TileSyntaxError> {
    if !(2..=5).contains(&n) {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    let n0 = 1usize << n;
    let n1 = 1usize << (n - 1);
    let n2 = 1usize << (n - 2);

    if n == 2 {
        b_simd(t, 0, 1, 16, true);
    } else {
        inverse_dct_simd(t, n - 1)?;
    }

    for i in 0..n2 {
        b_simd(t, n1 + i, n0 - 1 - i, 32 - brev(5, n1 + i) as i32, false);
    }

    if n >= 3 {
        let n3 = 1usize << (n - 3);
        for i in 0..n3 {
            for j in 0..=1 {
                h_simd(t, n1 + 4 * i + 2 * j, n1 + 1 + 4 * i + 2 * j, j != 0);
            }
        }
    }

    if n == 5 {
        let n3 = 1usize << (n - 3);
        for i in 0..=1 {
            for j in 0..=1 {
                b_simd(
                    t,
                    n0 - n + 3 - n2 * j - 4 * i,
                    n1 + n - 4 + n2 * j + 4 * i,
                    28 - 16 * i as i32 + 56 * j as i32,
                    true,
                );
            }
        }
        for i in 0..=1 {
            for j in 0..=3 {
                h_simd(t, n1 + n3 * j + i, n1 + n2 - 5 + n3 * j - i, (j & 1) != 0);
            }
        }
    }

    if n >= 4 {
        for i in 0..=usize::from(n == 5) {
            for j in 0..=1 {
                b_simd(
                    t,
                    n0 - n + 2 - i - n2 * j,
                    n1 + n - 3 + i + n2 * j,
                    24 + 48 * j as i32,
                    true,
                );
            }
        }
        for i in 0..=(2 * n - 7) {
            for j in 0..=1 {
                h_simd(t, n1 + n2 * j + i, n1 + n2 - 1 + n2 * j - i, (j & 1) != 0);
            }
        }
    }

    if n >= 3 {
        let n3 = 1usize << (n - 3);
        for i in 0..n3 {
            b_simd(t, n0 - n3 - 1 - i, n1 + n3 + i, 16, true);
        }
    }

    for i in 0..n1 {
        h_simd(t, i, n0 - 1 - i, false);
    }

    Ok(())
}

fn inverse_dct_simd_i16(t: &mut [v128; MAX_TX_WIDTH], n: usize) -> Result<(), TileSyntaxError> {
    if !(2..=5).contains(&n) {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    let n0 = 1usize << n;
    let n1 = 1usize << (n - 1);
    let n2 = 1usize << (n - 2);

    if n == 2 {
        b_simd_i16(t, 0, 1, 16, true);
    } else {
        inverse_dct_simd_i16(t, n - 1)?;
    }

    for i in 0..n2 {
        b_simd_i16(t, n1 + i, n0 - 1 - i, 32 - brev(5, n1 + i) as i32, false);
    }

    if n >= 3 {
        let n3 = 1usize << (n - 3);
        for i in 0..n3 {
            for j in 0..=1 {
                h_simd_i16(t, n1 + 4 * i + 2 * j, n1 + 1 + 4 * i + 2 * j, j != 0);
            }
        }
    }

    if n == 5 {
        let n3 = 1usize << (n - 3);
        for i in 0..=1 {
            for j in 0..=1 {
                b_simd_i16(
                    t,
                    n0 - n + 3 - n2 * j - 4 * i,
                    n1 + n - 4 + n2 * j + 4 * i,
                    28 - 16 * i as i32 + 56 * j as i32,
                    true,
                );
            }
        }
        for i in 0..=1 {
            for j in 0..=3 {
                h_simd_i16(t, n1 + n3 * j + i, n1 + n2 - 5 + n3 * j - i, (j & 1) != 0);
            }
        }
    }

    if n >= 4 {
        for i in 0..=usize::from(n == 5) {
            for j in 0..=1 {
                b_simd_i16(
                    t,
                    n0 - n + 2 - i - n2 * j,
                    n1 + n - 3 + i + n2 * j,
                    24 + 48 * j as i32,
                    true,
                );
            }
        }
        for i in 0..=(2 * n - 7) {
            for j in 0..=1 {
                h_simd_i16(t, n1 + n2 * j + i, n1 + n2 - 1 + n2 * j - i, (j & 1) != 0);
            }
        }
    }

    if n >= 3 {
        let n3 = 1usize << (n - 3);
        for i in 0..n3 {
            b_simd_i16(t, n0 - n3 - 1 - i, n1 + n3 + i, 16, true);
        }
    }

    for i in 0..n1 {
        h_simd_i16(t, i, n0 - 1 - i, false);
    }

    Ok(())
}

#[inline(always)]
fn inverse_adst_input_permutation_simd(
    t: &mut [v128; MAX_TX_WIDTH],
    n: usize,
    copy_t: &mut [v128; MAX_ADST_WIDTH],
) {
    let n0 = 1usize << n;
    let n1 = 1usize << (n - 1);
    copy_t[..n0].copy_from_slice(&t[..n0]);
    for i in 0..n1 {
        t[2 * i] = copy_t[n0 - 1 - 2 * i];
        t[2 * i + 1] = copy_t[2 * i];
    }
}

#[inline(always)]
fn inverse_adst_output_permutation_simd(
    t: &mut [v128; MAX_TX_WIDTH],
    n: usize,
    copy_t: &mut [v128; MAX_ADST_WIDTH],
) {
    let len = 1usize << n;
    copy_t[..len].copy_from_slice(&t[..len]);

    if n == 4 {
        for a in 0..=1 {
            for b_bit in 0..=1 {
                for c in 0..=1 {
                    for d in 0..=1 {
                        t[8 * a + 4 * b_bit + 2 * c + d] =
                            copy_t[8 * (d ^ c) + 4 * (c ^ b_bit) + 2 * (b_bit ^ a) + a];
                    }
                }
            }
        }
    } else {
        for a in 0..=1 {
            for b_bit in 0..=1 {
                for c in 0..=1 {
                    t[4 * a + 2 * b_bit + c] = copy_t[4 * (c ^ b_bit) + 2 * (b_bit ^ a) + a];
                }
            }
        }
    }
}

fn inverse_adst4_simd(t: &mut [v128; MAX_TX_WIDTH]) -> Result<(), TileSyntaxError> {
    let t0_lo = i64x2_extend_low_i32x4(t[0]);
    let t0_hi = i64x2_extend_high_i32x4(t[0]);
    let t1_lo = i64x2_extend_low_i32x4(t[1]);
    let t1_hi = i64x2_extend_high_i32x4(t[1]);
    let t2_lo = i64x2_extend_low_i32x4(t[2]);
    let t2_hi = i64x2_extend_high_i32x4(t[2]);
    let t3_lo = i64x2_extend_low_i32x4(t[3]);
    let t3_hi = i64x2_extend_high_i32x4(t[3]);

    let sinpi_1_9 = i64x2_splat(SINPI_1_9);
    let sinpi_2_9 = i64x2_splat(SINPI_2_9);
    let sinpi_3_9 = i64x2_splat(SINPI_3_9);
    let sinpi_4_9 = i64x2_splat(SINPI_4_9);

    let s0_lo = i64x2_mul(sinpi_1_9, t0_lo);
    let s0_hi = i64x2_mul(sinpi_1_9, t0_hi);
    let s1_lo = i64x2_mul(sinpi_2_9, t0_lo);
    let s1_hi = i64x2_mul(sinpi_2_9, t0_hi);
    let s2_lo = i64x2_mul(sinpi_3_9, t1_lo);
    let s2_hi = i64x2_mul(sinpi_3_9, t1_hi);
    let s3_lo = i64x2_mul(sinpi_4_9, t2_lo);
    let s3_hi = i64x2_mul(sinpi_4_9, t2_hi);
    let s4_lo = i64x2_mul(sinpi_1_9, t2_lo);
    let s4_hi = i64x2_mul(sinpi_1_9, t2_hi);
    let s5_lo = i64x2_mul(sinpi_2_9, t3_lo);
    let s5_hi = i64x2_mul(sinpi_2_9, t3_hi);
    let s6_lo = i64x2_mul(sinpi_4_9, t3_lo);
    let s6_hi = i64x2_mul(sinpi_4_9, t3_hi);
    let v_lo = i64x2_add(i64x2_sub(t0_lo, t2_lo), t3_lo);
    let v_hi = i64x2_add(i64x2_sub(t0_hi, t2_hi), t3_hi);
    let s7_lo = i64x2_mul(sinpi_3_9, v_lo);
    let s7_hi = i64x2_mul(sinpi_3_9, v_hi);

    let x0_lo = i64x2_add(i64x2_add(s0_lo, s3_lo), s5_lo);
    let x0_hi = i64x2_add(i64x2_add(s0_hi, s3_hi), s5_hi);
    let x1_lo = i64x2_sub(i64x2_sub(s1_lo, s4_lo), s6_lo);
    let x1_hi = i64x2_sub(i64x2_sub(s1_hi, s4_hi), s6_hi);
    let x2_lo = s7_lo;
    let x2_hi = s7_hi;
    let x3_lo = s2_lo;
    let x3_hi = s2_hi;
    let s0_lo = i64x2_add(x0_lo, x3_lo);
    let s0_hi = i64x2_add(x0_hi, x3_hi);
    let s1_lo = i64x2_add(x1_lo, x3_lo);
    let s1_hi = i64x2_add(x1_hi, x3_hi);
    let s2_lo = x2_lo;
    let s2_hi = x2_hi;
    let s3_lo = i64x2_sub(i64x2_add(x0_lo, x1_lo), x3_lo);
    let s3_hi = i64x2_sub(i64x2_add(x0_hi, x1_hi), x3_hi);

    t[0] = round_pack_i64x2_low_i32s(s0_lo, s0_hi, 14);
    t[1] = round_pack_i64x2_low_i32s(s1_lo, s1_hi, 14);
    t[2] = round_pack_i64x2_low_i32s(s2_lo, s2_hi, 14);
    t[3] = round_pack_i64x2_low_i32s(s3_lo, s3_hi, 14);
    Ok(())
}

fn inverse_adst8_simd(
    t: &mut [v128; MAX_TX_WIDTH],
    s_lo: &mut [v128; MAX_ADST_WIDTH],
    s_hi: &mut [v128; MAX_ADST_WIDTH],
    copy_t: &mut [v128; MAX_ADST_WIDTH],
) -> Result<(), TileSyntaxError> {
    inverse_adst_input_permutation_simd(t, 3, copy_t);
    for i in 0..=3 {
        sb_simd(t, s_lo, s_hi, 2 * i, 1 + 2 * i, 30 - 8 * i as i32, true);
    }
    for i in 0..=3 {
        sh_simd(t, s_lo, s_hi, i, 4 + i);
    }
    for i in 0..=1 {
        sb_simd(t, s_lo, s_hi, 4 + 3 * i, 5 + i, 24 - 16 * i as i32, true);
    }
    for i in 0..=1 {
        sh_simd(t, s_lo, s_hi, 4 + i, 6 + i);
    }
    for i in 0..=1 {
        h_simd(t, i, 2 + i, false);
    }
    for i in 0..=1 {
        b_simd(t, 2 + 4 * i, 3 + 4 * i, 16, true);
    }
    inverse_adst_output_permutation_simd(t, 3, copy_t);
    for i in 0..=3 {
        let index = 1 + 2 * i;
        t[index] = i32x4_sub(i32x4_splat(0), t[index]);
    }

    Ok(())
}

fn inverse_adst16_simd(
    t: &mut [v128; MAX_TX_WIDTH],
    s_lo: &mut [v128; MAX_ADST_WIDTH],
    s_hi: &mut [v128; MAX_ADST_WIDTH],
    copy_t: &mut [v128; MAX_ADST_WIDTH],
) -> Result<(), TileSyntaxError> {
    inverse_adst_input_permutation_simd(t, 4, copy_t);
    for i in 0..=7 {
        sb_simd(t, s_lo, s_hi, 2 * i, 1 + 2 * i, 31 - 4 * i as i32, true);
    }
    for i in 0..=7 {
        sh_simd(t, s_lo, s_hi, i, 8 + i);
    }
    for i in 0..=3 {
        sb_simd(
            t,
            s_lo,
            s_hi,
            8 + 2 * i,
            9 + 2 * i,
            28 - 16 * i as i32,
            true,
        );
    }
    for i in 0..=3 {
        sh_simd(t, s_lo, s_hi, 8 + i, 12 + i);
    }
    for i in 0..=3 {
        h_simd(t, i, 4 + i, false);
    }
    for i in 0..=1 {
        for j in 0..=1 {
            sb_simd(
                t,
                s_lo,
                s_hi,
                4 + 8 * i + 3 * j,
                5 + 8 * i + j,
                24 - 16 * j as i32,
                true,
            );
        }
    }
    for i in 0..=1 {
        for j in 0..=1 {
            sh_simd(t, s_lo, s_hi, 4 + 8 * j + i, 6 + 8 * j + i);
        }
    }
    for i in 0..=1 {
        for j in 0..=1 {
            h_simd(t, 8 * j + i, 2 + 8 * j + i, false);
        }
    }
    for i in 0..=1 {
        for j in 0..=1 {
            b_simd(
                t,
                2 + 4 * j + 8 * i,
                3 + 4 * j + 8 * i,
                48 + 64 * (i ^ j) as i32,
                false,
            );
        }
    }
    inverse_adst_output_permutation_simd(t, 4, copy_t);
    for i in 0..=1 {
        for j in 0..=1 {
            let index = 1 + 12 * j + 2 * i;
            t[index] = i32x4_sub(i32x4_splat(0), t[index]);
        }
    }

    Ok(())
}

fn inverse_adst_simd(
    t: &mut [v128; MAX_TX_WIDTH],
    n: usize,
    s_lo: &mut [v128; MAX_ADST_WIDTH],
    s_hi: &mut [v128; MAX_ADST_WIDTH],
    copy_t: &mut [v128; MAX_ADST_WIDTH],
) -> Result<(), TileSyntaxError> {
    match n {
        2 => inverse_adst4_simd(t),
        3 => inverse_adst8_simd(t, s_lo, s_hi, copy_t),
        4 => inverse_adst16_simd(t, s_lo, s_hi, copy_t),
        _ => Err(TileSyntaxError::InvalidBitstream),
    }
}

fn inverse_adst_input_permutation(
    t: &mut [i32; MAX_TX_WIDTH],
    n: usize,
    copy_t: &mut [i32; MAX_ADST_WIDTH],
) {
    let n0 = 1usize << n;
    let n1 = 1usize << (n - 1);
    copy_t[..n0].copy_from_slice(&t[..n0]);
    for i in 0..n1 {
        t[2 * i] = copy_t[n0 - 1 - 2 * i];
        t[2 * i + 1] = copy_t[2 * i];
    }
}

fn inverse_adst_output_permutation(
    t: &mut [i32; MAX_TX_WIDTH],
    n: usize,
    copy_t: &mut [i32; MAX_ADST_WIDTH],
) {
    let len = 1usize << n;
    copy_t[..len].copy_from_slice(&t[..len]);

    if n == 4 {
        for a in 0..=1 {
            for b_bit in 0..=1 {
                for c in 0..=1 {
                    for d in 0..=1 {
                        t[8 * a + 4 * b_bit + 2 * c + d] =
                            copy_t[8 * (d ^ c) + 4 * (c ^ b_bit) + 2 * (b_bit ^ a) + a];
                    }
                }
            }
        }
    } else {
        for a in 0..=1 {
            for b_bit in 0..=1 {
                for c in 0..=1 {
                    t[4 * a + 2 * b_bit + c] = copy_t[4 * (c ^ b_bit) + 2 * (b_bit ^ a) + a];
                }
            }
        }
    }
}

fn inverse_adst4(t: &mut [i32; MAX_TX_WIDTH]) -> Result<(), TileSyntaxError> {
    let t0 = i64::from(t[0]);
    let t1 = i64::from(t[1]);
    let t2 = i64::from(t[2]);
    let t3 = i64::from(t[3]);

    let s0 = SINPI_1_9 * t0;
    let s1 = SINPI_2_9 * t0;
    let s2 = SINPI_3_9 * t1;
    let s3 = SINPI_4_9 * t2;
    let s4 = SINPI_1_9 * t2;
    let s5 = SINPI_2_9 * t3;
    let s6 = SINPI_4_9 * t3;
    let v = t0 - t2 + t3;
    let s7 = SINPI_3_9 * v;
    let x0 = s0 + s3 + s5;
    let x1 = s1 - s4 - s6;
    let x2 = s7;
    let x3 = s2;
    let s0 = x0 + x3;
    let s1 = x1 + x3;
    let s2 = x2;
    let s3 = x0 + x1 - x3;

    t[0] = narrow_i32(round2_i64(s0, 14))?;
    t[1] = narrow_i32(round2_i64(s1, 14))?;
    t[2] = narrow_i32(round2_i64(s2, 14))?;
    t[3] = narrow_i32(round2_i64(s3, 14))?;
    Ok(())
}

fn inverse_adst8_with_scratch(
    t: &mut [i32; MAX_TX_WIDTH],
    s: &mut [i64; MAX_ADST_WIDTH],
    copy_t: &mut [i32; MAX_ADST_WIDTH],
) -> Result<(), TileSyntaxError> {
    inverse_adst_input_permutation(t, 3, copy_t);
    for i in 0..=3 {
        sb(t, s, 2 * i, 1 + 2 * i, 30 - 8 * i as i32, true);
    }
    for i in 0..=3 {
        sh(t, s, i, 4 + i)?;
    }
    for i in 0..=1 {
        sb(t, s, 4 + 3 * i, 5 + i, 24 - 16 * i as i32, true);
    }
    for i in 0..=1 {
        sh(t, s, 4 + i, 6 + i)?;
    }
    for i in 0..=1 {
        h(t, i, 2 + i, false);
    }
    for i in 0..=1 {
        b(t, 2 + 4 * i, 3 + 4 * i, 16, true);
    }
    inverse_adst_output_permutation(t, 3, copy_t);
    for i in 0..=3 {
        t[1 + 2 * i] = narrow_i32(-i64::from(t[1 + 2 * i]))?;
    }

    Ok(())
}

fn inverse_adst16_with_scratch(
    t: &mut [i32; MAX_TX_WIDTH],
    s: &mut [i64; MAX_ADST_WIDTH],
    copy_t: &mut [i32; MAX_ADST_WIDTH],
) -> Result<(), TileSyntaxError> {
    inverse_adst_input_permutation(t, 4, copy_t);
    for i in 0..=7 {
        sb(t, s, 2 * i, 1 + 2 * i, 31 - 4 * i as i32, true);
    }
    for i in 0..=7 {
        sh(t, s, i, 8 + i)?;
    }
    for i in 0..=3 {
        sb(t, s, 8 + 2 * i, 9 + 2 * i, 28 - 16 * i as i32, true);
    }
    for i in 0..=3 {
        sh(t, s, 8 + i, 12 + i)?;
    }
    for i in 0..=3 {
        h(t, i, 4 + i, false);
    }
    for i in 0..=1 {
        for j in 0..=1 {
            sb(
                t,
                s,
                4 + 8 * i + 3 * j,
                5 + 8 * i + j,
                24 - 16 * j as i32,
                true,
            );
        }
    }
    for i in 0..=1 {
        for j in 0..=1 {
            sh(t, s, 4 + 8 * j + i, 6 + 8 * j + i)?;
        }
    }
    for i in 0..=1 {
        for j in 0..=1 {
            h(t, 8 * j + i, 2 + 8 * j + i, false);
        }
    }
    for i in 0..=1 {
        for j in 0..=1 {
            b(
                t,
                2 + 4 * j + 8 * i,
                3 + 4 * j + 8 * i,
                48 + 64 * (i ^ j) as i32,
                false,
            );
        }
    }
    inverse_adst_output_permutation(t, 4, copy_t);
    for i in 0..=1 {
        for j in 0..=1 {
            let index = 1 + 12 * j + 2 * i;
            t[index] = narrow_i32(-i64::from(t[index]))?;
        }
    }

    Ok(())
}

fn inverse_adst_with_scratch(
    t: &mut [i32; MAX_TX_WIDTH],
    n: usize,
    s: &mut [i64; MAX_ADST_WIDTH],
    copy_t: &mut [i32; MAX_ADST_WIDTH],
) -> Result<(), TileSyntaxError> {
    match n {
        2 => inverse_adst4(t),
        3 => inverse_adst8_with_scratch(t, s, copy_t),
        4 => inverse_adst16_with_scratch(t, s, copy_t),
        _ => Err(TileSyntaxError::InvalidBitstream),
    }
}

fn inverse_wht(t: &mut [i32; MAX_TX_WIDTH], shift: usize) -> Result<(), TileSyntaxError> {
    let mut a = i64::from(t[0]) >> shift;
    let mut c = i64::from(t[1]) >> shift;
    let mut d = i64::from(t[2]) >> shift;
    let mut b = i64::from(t[3]) >> shift;
    a += c;
    d -= b;
    let e = (a - d) >> 1;
    b = e - b;
    c = e - c;
    a -= b;
    d += c;

    t[0] = narrow_i32(a)?;
    t[1] = narrow_i32(b)?;
    t[2] = narrow_i32(c)?;
    t[3] = narrow_i32(d)?;
    Ok(())
}

pub(super) const fn dq_denom(tx_size: TxSize) -> i32 {
    match tx_size {
        TxSize::Tx32x32 => 2,
        TxSize::Tx4x4 | TxSize::Tx8x8 | TxSize::Tx16x16 => 1,
    }
}

fn dc_q(q_index: i32) -> i32 {
    DC_QLOOKUP_8BIT[clipped_q_index(q_index)]
}

fn ac_q(q_index: i32) -> i32 {
    AC_QLOOKUP_8BIT[clipped_q_index(q_index)]
}

fn clipped_q_index(q_index: i32) -> usize {
    q_index.clamp(0, 255) as usize
}

const DC_QLOOKUP_8BIT: [i32; 256] = [
    4, 8, 8, 9, 10, 11, 12, 12, 13, 14, 15, 16, 17, 18, 19, 19, 20, 21, 22, 23, 24, 25, 26, 26, 27,
    28, 29, 30, 31, 32, 32, 33, 34, 35, 36, 37, 38, 38, 39, 40, 41, 42, 43, 43, 44, 45, 46, 47, 48,
    48, 49, 50, 51, 52, 53, 53, 54, 55, 56, 57, 57, 58, 59, 60, 61, 62, 62, 63, 64, 65, 66, 66, 67,
    68, 69, 70, 70, 71, 72, 73, 74, 74, 75, 76, 77, 78, 78, 79, 80, 81, 81, 82, 83, 84, 85, 85, 87,
    88, 90, 92, 93, 95, 96, 98, 99, 101, 102, 104, 105, 107, 108, 110, 111, 113, 114, 116, 117,
    118, 120, 121, 123, 125, 127, 129, 131, 134, 136, 138, 140, 142, 144, 146, 148, 150, 152, 154,
    156, 158, 161, 164, 166, 169, 172, 174, 177, 180, 182, 185, 187, 190, 192, 195, 199, 202, 205,
    208, 211, 214, 217, 220, 223, 226, 230, 233, 237, 240, 243, 247, 250, 253, 257, 261, 265, 269,
    272, 276, 280, 284, 288, 292, 296, 300, 304, 309, 313, 317, 322, 326, 330, 335, 340, 344, 349,
    354, 359, 364, 369, 374, 379, 384, 389, 395, 400, 406, 411, 417, 423, 429, 435, 441, 447, 454,
    461, 467, 475, 482, 489, 497, 505, 513, 522, 530, 539, 549, 559, 569, 579, 590, 602, 614, 626,
    640, 654, 668, 684, 700, 717, 736, 755, 775, 796, 819, 843, 869, 896, 925, 955, 988, 1022,
    1058, 1098, 1139, 1184, 1232, 1282, 1336,
];

const AC_QLOOKUP_8BIT: [i32; 256] = [
    4, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54,
    55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78,
    79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101,
    102, 104, 106, 108, 110, 112, 114, 116, 118, 120, 122, 124, 126, 128, 130, 132, 134, 136, 138,
    140, 142, 144, 146, 148, 150, 152, 155, 158, 161, 164, 167, 170, 173, 176, 179, 182, 185, 188,
    191, 194, 197, 200, 203, 207, 211, 215, 219, 223, 227, 231, 235, 239, 243, 247, 251, 255, 260,
    265, 270, 275, 280, 285, 290, 295, 300, 305, 311, 317, 323, 329, 335, 341, 347, 353, 359, 366,
    373, 380, 387, 394, 401, 408, 416, 424, 432, 440, 448, 456, 465, 474, 483, 492, 501, 510, 520,
    530, 540, 550, 560, 571, 582, 593, 604, 615, 627, 639, 651, 663, 676, 689, 702, 715, 729, 743,
    757, 771, 786, 801, 816, 832, 848, 864, 881, 898, 915, 933, 951, 969, 988, 1007, 1026, 1046,
    1066, 1087, 1108, 1129, 1151, 1173, 1196, 1219, 1243, 1267, 1292, 1317, 1343, 1369, 1396, 1423,
    1451, 1479, 1508, 1537, 1567, 1597, 1628, 1660, 1692, 1725, 1759, 1793, 1828,
];

pub(super) fn reconstruct(
    current_frame: &mut CurrentFrameMut<'_>,
    dequantized: &DequantizedCoefficients,
) -> Result<(), TileSyntaxError> {
    let plane = current_frame.plane_mut(dequantized.block.plane)?;
    add_residual_block(
        plane,
        dequantized.block.start,
        dequantized.block.tx_size,
        &dequantized.coefficients,
    )
}

pub(super) fn add_residual_block(
    plane: &mut CurrentPlaneMut<'_>,
    start: (usize, usize),
    tx_size: TxSize,
    residuals: &[i32; MAX_TX_COEFFS],
) -> Result<(), TileSyntaxError> {
    let size = transform_width(tx_size);
    if residual_block_inside(plane, start, size) {
        return add_residual_block_interior_simd(plane, start, size, residuals);
    }

    add_residual_block_scalar(plane, start, size, residuals)
}

#[inline(always)]
pub(super) fn residual_block_inside(
    plane: &CurrentPlaneMut<'_>,
    start: (usize, usize),
    size: usize,
) -> bool {
    start
        .0
        .checked_add(size)
        .is_some_and(|right| right <= plane.width)
        && start
            .1
            .checked_add(size)
            .is_some_and(|bottom| bottom <= plane.height)
}

pub(super) fn add_residual_block_scalar(
    plane: &mut CurrentPlaneMut<'_>,
    start: (usize, usize),
    size: usize,
    residuals: &[i32; MAX_TX_COEFFS],
) -> Result<(), TileSyntaxError> {
    for row in 0..size {
        let y = start
            .1
            .checked_add(row)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        for col in 0..size {
            let x = start
                .0
                .checked_add(col)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            if x >= plane.width || y >= plane.height {
                continue;
            }
            let predicted = plane.sample_clamped(x, y)?;
            plane.set_visible(
                x,
                y,
                clip1(i32::from(predicted) + residuals[row * size + col]),
            )?;
        }
    }
    Ok(())
}

pub(super) fn add_residual_block_interior_simd(
    plane: &mut CurrentPlaneMut<'_>,
    start: (usize, usize),
    size: usize,
    residuals: &[i32; MAX_TX_COEFFS],
) -> Result<(), TileSyntaxError> {
    debug_assert!(residual_block_inside(plane, start, size));

    for row in 0..size {
        let y = start
            .1
            .checked_add(row)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let dst_start = y
            .checked_mul(plane.stride)
            .and_then(|base| base.checked_add(start.0))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let dst_end = dst_start
            .checked_add(size)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let dst = plane
            .data
            .get_mut(dst_start..dst_end)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let residual_start = row
            .checked_mul(size)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let residual_end = residual_start
            .checked_add(size)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let residual_row = residuals
            .get(residual_start..residual_end)
            .ok_or(TileSyntaxError::InvalidBitstream)?;

        add_residual_row_simd(dst, residual_row);
    }

    Ok(())
}

#[inline(always)]
pub(super) fn add_residual_row_simd(dst: &mut [u8], residuals: &[i32]) {
    debug_assert_eq!(dst.len(), residuals.len());

    let mut col = 0;
    while col + 8 <= dst.len() {
        let prediction = unsafe { v128_load64_zero(dst.as_ptr().wrapping_add(col).cast::<u64>()) };
        let residual_lo = unsafe { v128_load(residuals.as_ptr().wrapping_add(col).cast::<v128>()) };
        let residual_hi =
            unsafe { v128_load(residuals.as_ptr().wrapping_add(col + 4).cast::<v128>()) };
        let reconstruction = add_residual_8(prediction, residual_lo, residual_hi);
        unsafe {
            v128_store64_lane::<0>(
                reconstruction,
                dst.as_mut_ptr().wrapping_add(col).cast::<u64>(),
            );
        }
        col += 8;
    }
    if col + 4 <= dst.len() {
        let prediction = unsafe { v128_load32_zero(dst.as_ptr().wrapping_add(col).cast::<u32>()) };
        let residuals = unsafe { v128_load(residuals.as_ptr().wrapping_add(col).cast::<v128>()) };
        let reconstruction = add_residual_4(prediction, residuals);
        unsafe {
            v128_store32_lane::<0>(
                reconstruction,
                dst.as_mut_ptr().wrapping_add(col).cast::<u32>(),
            );
        }
        col += 4;
    }

    debug_assert_eq!(col, dst.len());
}

#[inline(always)]
pub(super) fn add_residual_8(prediction: v128, residual_lo: v128, residual_hi: v128) -> v128 {
    let predicted = i16x8_extend_low_u8x16(prediction);
    let lo = i32x4_add(i32x4_extend_low_i16x8(predicted), residual_lo);
    let hi = i32x4_add(i32x4_extend_high_i16x8(predicted), residual_hi);
    let packed_i16 = i16x8_narrow_i32x4(lo, hi);
    u8x16_narrow_i16x8(packed_i16, i16x8_splat(0))
}

#[inline(always)]
pub(super) fn add_residual_4(prediction: v128, residuals: v128) -> v128 {
    let predicted = i16x8_extend_low_u8x16(prediction);
    let reconstruction = i32x4_add(i32x4_extend_low_i16x8(predicted), residuals);
    let packed_i16 = i16x8_narrow_i32x4(reconstruction, i32x4_splat(0));
    u8x16_narrow_i16x8(packed_i16, i16x8_splat(0))
}

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::super::scan_table;
    use super::super::test_support::*;
    use super::*;

    #[derive(Clone, Copy)]
    enum TransformPattern {
        SingleCoeff,
        SingleRow,
        Full,
    }

    impl TransformPattern {
        const fn name(self) -> &'static str {
            match self {
                Self::SingleCoeff => "single-coeff",
                Self::SingleRow => "single-row",
                Self::Full => "full",
            }
        }
    }

    fn block(tx_size: TxSize, tx_type: TxType, values: &[(usize, i32)]) -> DequantizedCoefficients {
        let eob = match values {
            [] => 0,
            [(0, _)] => 1,
            _ => coefficient_count(tx_size),
        };
        let mut block =
            DequantizedCoefficients::new(TransformBlock::new(0, (0, 0), tx_size, tx_type), eob);
        let width = transform_width(tx_size);
        for &(index, value) in values {
            block.coefficients[index] = value;
            if value != 0 {
                block.nonzero_row_mask |= 1u32 << (index / width);
            }
        }
        block
    }

    fn pseudo_dequantized_value(seed: u32, index: usize, limit: i32) -> i32 {
        let mut x = seed ^ (index as u32).wrapping_mul(0x9e37_79b9);
        x ^= x >> 16;
        x = x.wrapping_mul(0x7feb_352d);
        x ^= x >> 15;
        x = x.wrapping_mul(0x846c_a68b);
        x ^= x >> 16;

        let span = (2 * limit + 1) as u32;
        let value = (x % span) as i32 - limit;
        if value == 0 {
            if (x & 1) == 0 { 1 } else { -1 }
        } else {
            value
        }
    }

    fn patterned_block(
        tx_size: TxSize,
        tx_type: TxType,
        pattern: TransformPattern,
        seed: u32,
    ) -> DequantizedCoefficients {
        let width = transform_width(tx_size);
        let count = coefficient_count(tx_size);
        let mut block =
            DequantizedCoefficients::new(TransformBlock::new(0, (0, 0), tx_size, tx_type), count);
        // Exercise realistic dequantized magnitudes without going anywhere near
        // raw i32 extremes. DCT_DCT now uses the spec-licensed i16 T-array
        // domain, so keep that sweep inside a conforming range; mixed/ADST
        // paths still use the wider i32 S-array path and retain the historical
        // stress range.
        let limit = if tx_type == TxType::DctDct {
            1024
        } else {
            AC_QLOOKUP_8BIT[255] * 32
        };

        match pattern {
            TransformPattern::SingleCoeff => {
                let index = width + 1;
                block.coefficients[index] = pseudo_dequantized_value(seed, index, limit);
                block.nonzero_row_mask = 1u32 << (index / width);
            }
            TransformPattern::SingleRow => {
                let row = width / 2;
                block.nonzero_row_mask = 1u32 << row;
                for col in 0..width {
                    let index = row * width + col;
                    block.coefficients[index] = pseudo_dequantized_value(seed, index, limit);
                }
            }
            TransformPattern::Full => {
                block.nonzero_row_mask = (1u32 << width) - 1;
                for index in 0..count {
                    block.coefficients[index] = pseudo_dequantized_value(seed, index, limit);
                }
            }
        }

        block
    }

    fn assert_current_matches_scalar(
        block: DequantizedCoefficients,
        lossless: bool,
        pattern: &str,
    ) {
        let tx_size = block.block.tx_size;
        let tx_type = block.block.tx_type;
        let count = coefficient_count(tx_size);
        let mut current = block.clone();
        let mut scalar = block;

        let current_result = current.inverse_transform(lossless);
        let scalar_result = scalar.inverse_transform_scalar(lossless);
        assert_eq!(
            current_result, scalar_result,
            "result mismatch for {tx_size:?} {tx_type:?} {pattern}"
        );

        if current_result.is_ok()
            && let Some(index) = current.coefficients[..count]
                .iter()
                .zip(scalar.coefficients[..count].iter())
                .position(|(current, scalar)| current != scalar)
        {
            panic!(
                "coefficient mismatch for {tx_size:?} {tx_type:?} {pattern} at {index}: simd={} scalar={}",
                current.coefficients[index], scalar.coefficients[index]
            );
        }
    }

    #[test]
    fn inverse_dct_permutation_matches_copy_reference() {
        for n in 2..=5 {
            let len = 1usize << n;
            let mut input = [0i32; MAX_TX_WIDTH];
            for (i, value) in input.iter_mut().enumerate() {
                *value = 0x1000 + i as i32 * 17;
            }

            let mut expected = input;
            for i in 0..len {
                expected[i] = input[brev(n, i)];
            }

            let mut actual = input;
            inverse_dct_permutation(&mut actual, n);
            assert_eq!(actual, expected, "n={n}");
        }
    }

    #[test]
    fn zero_blocks_remain_zero_for_legal_transform_combinations() {
        for tx_size in [TxSize::Tx4x4, TxSize::Tx8x8, TxSize::Tx16x16] {
            for tx_type in [
                TxType::DctDct,
                TxType::AdstDct,
                TxType::DctAdst,
                TxType::AdstAdst,
            ] {
                let mut block = block(tx_size, tx_type, &[]);
                assert_eq!(block.inverse_transform(false), Ok(()));
                assert_eq!(
                    &block.coefficients[..coefficient_count(tx_size)],
                    &[0; 1024][..coefficient_count(tx_size)]
                );
            }
        }

        let mut tx32 = block(TxSize::Tx32x32, TxType::DctDct, &[]);
        assert_eq!(tx32.inverse_transform(false), Ok(()));
        assert_eq!(tx32.coefficients, [0; MAX_TX_COEFFS]);

        let mut wht = block(TxSize::Tx4x4, TxType::DctDct, &[]);
        assert_eq!(wht.inverse_transform(true), Ok(()));
        assert_eq!(&wht.coefficients[..16], [0; 16]);
    }

    #[test]
    fn inverse_transform_current_path_matches_scalar_sweep() {
        for tx_size in [
            TxSize::Tx4x4,
            TxSize::Tx8x8,
            TxSize::Tx16x16,
            TxSize::Tx32x32,
        ] {
            let tx_types: &[TxType] = if tx_size == TxSize::Tx32x32 {
                &[TxType::DctDct]
            } else {
                &[
                    TxType::DctDct,
                    TxType::AdstDct,
                    TxType::DctAdst,
                    TxType::AdstAdst,
                ]
            };

            for &tx_type in tx_types {
                for pattern in [
                    TransformPattern::SingleCoeff,
                    TransformPattern::SingleRow,
                    TransformPattern::Full,
                ] {
                    let seed = 0x51ed_600d
                        ^ ((tx_size.index() as u32) << 12)
                        ^ ((tx_type as u32) << 8)
                        ^ pattern.name().as_bytes()[0] as u32;
                    let block = patterned_block(tx_size, tx_type, pattern, seed);
                    assert_current_matches_scalar(block, false, pattern.name());
                }
            }
        }
    }

    #[test]
    fn lossless_wht_current_path_matches_scalar() {
        let block = patterned_block(
            TxSize::Tx4x4,
            TxType::DctDct,
            TransformPattern::Full,
            0x1055_1e55,
        );
        assert_current_matches_scalar(block, true, "lossless-wht-full");
    }

    #[test]
    fn dct_dct_dc_vectors_cover_all_transform_sizes() {
        for tx_size in [
            TxSize::Tx4x4,
            TxSize::Tx8x8,
            TxSize::Tx16x16,
            TxSize::Tx32x32,
        ] {
            let mut b = block(tx_size, TxType::DctDct, &[(0, 4096)]);
            b.inverse_transform(false).unwrap();
            let expected = match tx_size {
                TxSize::Tx4x4 => 128,
                TxSize::Tx8x8 => 64,
                TxSize::Tx16x16 | TxSize::Tx32x32 => 32,
            };
            assert!(
                b.coefficients[..coefficient_count(tx_size)]
                    .iter()
                    .all(|&value| value == expected),
                "unexpected DCT_DCT DC vector for {tx_size:?}: {:?}",
                &b.coefficients[..coefficient_count(tx_size)]
            );
        }
    }

    #[test]
    fn dct_dct_dc_only_path_matches_general_transform() {
        let dc_values = [
            i16::MIN as i32,
            -16384,
            -8192,
            -4096,
            -255,
            -1,
            0,
            1,
            255,
            4096,
            8192,
            16384,
            i16::MAX as i32,
        ];

        for tx_size in [
            TxSize::Tx4x4,
            TxSize::Tx8x8,
            TxSize::Tx16x16,
            TxSize::Tx32x32,
        ] {
            let n = 2 + tx_size.index();
            let width = transform_width(tx_size);
            let count = coefficient_count(tx_size);
            for dc in dc_values {
                let mut general = block(tx_size, TxType::DctDct, &[(0, dc)]);
                general
                    .inverse_transform_2d(false, n, width, u32::MAX)
                    .unwrap();

                let mut fast = block(tx_size, TxType::DctDct, &[(0, dc)]);
                fast.inverse_transform(false).unwrap();

                assert_eq!(
                    &fast.coefficients[..count],
                    &general.coefficients[..count],
                    "DC-only mismatch for {tx_size:?} dc={dc}"
                );
            }
        }
    }

    #[test]
    fn four_by_four_adst_vectors_cover_each_adst_bearing_type() {
        let cases = [
            (
                TxType::AdstDct,
                [1, 2, 3, 5, 2, 4, 6, 8, 1, 4, 6, 7, 2, 3, 5, 7],
            ),
            (
                TxType::DctAdst,
                [0, 1, 6, 10, -1, 1, 6, 8, -1, 1, 4, 6, 0, 1, 3, 5],
            ),
            (
                TxType::AdstAdst,
                [0, 1, 3, 6, -1, 1, 5, 9, -1, 1, 6, 9, 0, 1, 5, 8],
            ),
        ];

        for (tx_type, expected) in cases {
            let mut b = block(
                TxSize::Tx4x4,
                tx_type,
                &[(0, 128), (1, -64), (4, 32), (5, -16), (10, 8)],
            );
            b.inverse_transform(false).unwrap();
            assert_eq!(&b.coefficients[..16], expected);
        }
    }

    #[test]
    fn eight_and_sixteen_adst_paths_produce_nonzero_residuals() {
        for tx_size in [TxSize::Tx8x8, TxSize::Tx16x16] {
            for tx_type in [TxType::AdstDct, TxType::DctAdst, TxType::AdstAdst] {
                let mut b = block(tx_size, tx_type, &[(0, 128), (1, -64), (7, 32)]);
                b.inverse_transform(false).unwrap();
                assert!(
                    b.coefficients[..coefficient_count(tx_size)]
                        .iter()
                        .any(|&value| value != 0),
                    "{tx_size:?} {tx_type:?} ADST smoke vector stayed all-zero"
                );
            }
        }
    }

    #[test]
    fn lossless_wht_four_by_four_fixed_vector() {
        let mut b = block(
            TxSize::Tx4x4,
            TxType::DctDct,
            &[(0, 4), (1, -8), (2, 12), (3, -16), (4, 20), (5, -24)],
        );
        b.inverse_transform(true).unwrap();
        assert_eq!(
            &b.coefficients[..16],
            [0, 0, 2, 5, -1, -1, 1, 5, -1, 0, -4, 0, -1, 0, -4, 0]
        );
    }

    #[test]
    fn impossible_transform_combinations_return_errors() {
        let mut lossless_non4x4 = block(TxSize::Tx8x8, TxType::DctDct, &[]);
        assert_eq!(
            lossless_non4x4.inverse_transform(true),
            Err(TileSyntaxError::InvalidBitstream)
        );

        let mut lossless_adst = block(TxSize::Tx4x4, TxType::AdstDct, &[]);
        assert_eq!(
            lossless_adst.inverse_transform(true),
            Err(TileSyntaxError::InvalidBitstream)
        );

        let mut tx32_adst = block(TxSize::Tx32x32, TxType::DctAdst, &[]);
        assert_eq!(
            tx32_adst.inverse_transform(false),
            Err(TileSyntaxError::InvalidBitstream)
        );
    }
    #[test]
    fn reconstruction_adds_residuals_and_clips_visible_samples() {
        let mut data = [100u8; 16];
        let mut residuals = [0i32; MAX_TX_COEFFS];
        residuals[0] = 200;
        residuals[1] = -150;
        residuals[2] = 20;
        residuals[15] = -1;

        {
            let mut plane =
                CurrentPlaneMut::new(&mut data, crate::PlaneShape::new(4, 4, 4)).unwrap();
            add_residual_block(&mut plane, (0, 0), TxSize::Tx4x4, &residuals).unwrap();
        }

        assert_eq!(data[0], 255);
        assert_eq!(data[1], 0);
        assert_eq!(data[2], 120);
        assert_eq!(data[15], 99);
    }

    #[test]
    fn reconstruction_simd_fast_path_matches_scalar_reference() -> Result<(), TileSyntaxError> {
        const MAX_STRIDE: usize = 48;
        const MAX_HEIGHT: usize = 40;
        const TX_SIZES: [TxSize; 4] = [
            TxSize::Tx4x4,
            TxSize::Tx8x8,
            TxSize::Tx16x16,
            TxSize::Tx32x32,
        ];

        for tx_size in TX_SIZES {
            let size = transform_width(tx_size);
            let cases = [
                (size + 5, size + 6, (3, 4)),
                (size, size, (0, 0)),
                (size + 1, size + 3, (size - 2, 1)),
                (size + 3, size + 1, (1, size - 2)),
                (size + 1, size + 1, (size - 2, size - 2)),
            ];

            for (case_index, (width, height, start)) in cases.into_iter().enumerate() {
                let stride = MAX_STRIDE;
                let mut initial = [0u8; MAX_STRIDE * MAX_HEIGHT];
                let mut seed = 0x0102_0304u32 ^ ((size as u32) << 16) ^ ((case_index as u32) << 8);
                fill_pseudorandom(&mut initial, &mut seed);

                let mut residuals = [0i32; MAX_TX_COEFFS];
                for (index, residual) in residuals[..size * size].iter_mut().enumerate() {
                    *residual = match index % 8 {
                        0 => 300,
                        1 => -300,
                        2 => 100_000,
                        3 => -100_000,
                        4 => 17,
                        5 => -19,
                        6 => 255,
                        _ => -255,
                    };
                }

                let mut scalar_data = initial;
                let mut simd_data = initial;
                {
                    let mut scalar_plane = CurrentPlaneMut::new(
                        &mut scalar_data,
                        crate::PlaneShape::new(width as u32, height as u32, stride),
                    )?;
                    add_residual_block_scalar(&mut scalar_plane, start, size, &residuals)?;
                }
                {
                    let mut simd_plane = CurrentPlaneMut::new(
                        &mut simd_data,
                        crate::PlaneShape::new(width as u32, height as u32, stride),
                    )?;
                    add_residual_block(&mut simd_plane, start, tx_size, &residuals)?;
                }

                if scalar_data != simd_data {
                    let mismatch = scalar_data
                        .iter()
                        .zip(simd_data.iter())
                        .position(|(scalar, simd)| scalar != simd)
                        .expect("mismatch exists");
                    panic!(
                        "residual reconstruction mismatch: tx_size={tx_size:?} \
                         case={case_index} width={width} height={height} start={start:?} \
                         index={mismatch} scalar={} simd={}",
                        scalar_data[mismatch], simd_data[mismatch]
                    );
                }
            }
        }

        Ok(())
    }

    #[test]
    fn dequant_helpers_clip_q_indexes() {
        assert_eq!(FrameDequant::new(0, -99, 0, 0).get_dc_quant(0), 4);
        assert_eq!(FrameDequant::new(255, 0, 0, 99).get_ac_quant(1), 1828);
    }

    #[test]
    fn dequant_helpers_apply_y_and_uv_deltas() {
        let dequant = FrameDequant::new(4, 2, 4, 5);

        assert_eq!(dequant.get_dc_quant(0), 12);
        assert_eq!(dequant.get_ac_quant(0), 11);
        assert_eq!(dequant.get_dc_quant(1), 13);
        assert_eq!(dequant.get_ac_quant(2), 16);
    }

    #[test]
    fn fused_dequantized_writes_use_dc_ac_quantizers_and_tx32_dq_denom() {
        let dequant = FrameDequant::new(4, 2, 0, 0);
        let mut output16 = DequantizedCoefficients::empty();
        let tx16_block = TransformBlock::new(0, (8, 16), TxSize::Tx16x16, TxType::DctDct);
        let tx16_ac = usize::from(scan_table(tx16_block.tx_size, tx16_block.tx_type)[1]);
        output16.reset(tx16_block, 0);
        output16
            .set_signed_dequantized(
                0,
                2,
                0,
                dequant.get_dc_quant_for_segment(0, 0),
                dequant.get_ac_quant_for_segment(0, 0),
                dq_denom(tx16_block.tx_size),
            )
            .unwrap();
        output16
            .set_signed_dequantized(
                tx16_ac,
                4,
                0,
                dequant.get_dc_quant_for_segment(0, 0),
                dequant.get_ac_quant_for_segment(0, 0),
                dq_denom(tx16_block.tx_size),
            )
            .unwrap();
        output16.set_eob(2).unwrap();
        assert_eq!(output16.block, tx16_block);
        assert_eq!(output16.eob, 2);
        assert_eq!(output16.coefficients[0], 24);
        assert_eq!(output16.coefficients[tx16_ac], 44);
        assert_eq!(
            output16.nonzero_row_mask,
            1u32 | (1u32 << (tx16_ac >> (2 + tx16_block.tx_size.index())))
        );

        let mut output32 = DequantizedCoefficients::empty();
        let tx32_block = TransformBlock::new(0, (8, 16), TxSize::Tx32x32, TxType::DctDct);
        let tx32_ac = usize::from(scan_table(tx32_block.tx_size, tx32_block.tx_type)[1]);
        output32.reset(tx32_block, 0);
        output32
            .set_signed_dequantized(
                0,
                2,
                0,
                dequant.get_dc_quant_for_segment(0, 0),
                dequant.get_ac_quant_for_segment(0, 0),
                dq_denom(tx32_block.tx_size),
            )
            .unwrap();
        output32
            .set_signed_dequantized(
                tx32_ac,
                4,
                0,
                dequant.get_dc_quant_for_segment(0, 0),
                dequant.get_ac_quant_for_segment(0, 0),
                dq_denom(tx32_block.tx_size),
            )
            .unwrap();
        output32.set_eob(2).unwrap();
        assert_eq!(output32.block, tx32_block);
        assert_eq!(output32.eob, 2);
        assert_eq!(output32.coefficients[0], 12);
        assert_eq!(output32.coefficients[tx32_ac], 22);
    }

    #[test]
    fn alt_q_segmentation_adjusts_quantizer_index() {
        let mut segmentation = crate::header::SegmentationParams::disabled();
        segmentation.enabled = true;
        segmentation.feature_enabled[1][crate::header::SEG_LVL_ALT_Q] = true;
        segmentation.feature_data[1][crate::header::SEG_LVL_ALT_Q] = -25;
        let dequant = FrameDequant::new_with_segmentation(131, 0, 0, 0, segmentation);

        assert_eq!(dequant.get_qindex_for_segment(0), 131);
        assert_eq!(dequant.get_qindex_for_segment(1), 106);
    }
}

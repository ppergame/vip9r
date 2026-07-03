use crate::header::{SEG_LVL_ALT_Q, SegmentationParams, UncompressedFrameHeader};

use super::{MAX_TX_COEFFS, PLANES, TileSyntaxError, TxSize, TxType};

#[cfg(target_arch = "wasm32")]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TransformCoefficients {
    pub(super) block: TransformBlock,
    pub(super) coefficients: [i16; MAX_TX_COEFFS],
    pub(super) eob: usize,
}

impl TransformCoefficients {
    pub(super) const fn empty() -> Self {
        Self {
            block: TransformBlock::new(0, (0, 0), TxSize::Tx4x4, TxType::DctDct),
            coefficients: [0; MAX_TX_COEFFS],
            eob: 0,
        }
    }

    #[cfg(feature = "wasm-tests")]
    pub(super) fn new(
        plane: usize,
        start: (usize, usize),
        tx_size: TxSize,
        tx_type: TxType,
    ) -> Result<Self, TileSyntaxError> {
        if plane >= PLANES {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        Ok(Self {
            block: TransformBlock::new(plane, start, tx_size, tx_type),
            coefficients: [0; MAX_TX_COEFFS],
            eob: 0,
        })
    }

    pub(super) fn reset(
        &mut self,
        plane: usize,
        start: (usize, usize),
        tx_size: TxSize,
        tx_type: TxType,
    ) -> Result<(), TileSyntaxError> {
        if plane >= PLANES {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        self.block = TransformBlock::new(plane, start, tx_size, tx_type);
        self.eob = 0;
        Ok(())
    }

    pub(super) fn set_signed(
        &mut self,
        pos: usize,
        magnitude: u32,
        sign_bit: u32,
    ) -> Result<(), TileSyntaxError> {
        let magnitude = i16::try_from(magnitude).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let coefficient = if sign_bit == 0 {
            magnitude
        } else {
            magnitude
                .checked_neg()
                .ok_or(TileSyntaxError::InvalidBitstream)?
        };
        self.set_quantized(pos, coefficient)
    }

    pub(super) fn set_quantized(
        &mut self,
        pos: usize,
        coefficient: i16,
    ) -> Result<(), TileSyntaxError> {
        if pos >= coefficient_count(self.block.tx_size) {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let slot = self
            .coefficients
            .get_mut(pos)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        *slot = coefficient;
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

    pub(super) fn clear_scan_prefix(&mut self, scan: &[u16], eob: usize) {
        for &pos in scan.iter().take(eob) {
            self.coefficients[usize::from(pos)] = 0;
        }
        self.eob = 0;
    }
}

#[derive(Clone, Debug)]
pub(super) struct DequantizedCoefficients {
    pub(super) block: TransformBlock,
    pub(super) coefficients: [i32; MAX_TX_COEFFS],
    pub(super) eob: usize,
    nonzero_row_mask: u32,
    #[cfg(target_arch = "wasm32")]
    dct_simd_scratch: [v128; MAX_TX_WIDTH],
    #[cfg(target_arch = "wasm32")]
    dct_scalar_scratch: [i32; MAX_TX_WIDTH],
}

impl DequantizedCoefficients {
    pub(super) fn empty() -> Self {
        Self {
            block: TransformBlock::new(0, (0, 0), TxSize::Tx4x4, TxType::DctDct),
            coefficients: [0; MAX_TX_COEFFS],
            eob: 0,
            nonzero_row_mask: 0,
            #[cfg(target_arch = "wasm32")]
            dct_simd_scratch: [i32x4_splat(0); MAX_TX_WIDTH],
            #[cfg(target_arch = "wasm32")]
            dct_scalar_scratch: [0; MAX_TX_WIDTH],
        }
    }

    #[cfg(feature = "wasm-tests")]
    fn new(block: TransformBlock, eob: usize) -> Self {
        Self {
            block,
            coefficients: [0; MAX_TX_COEFFS],
            eob,
            nonzero_row_mask: 0,
            #[cfg(target_arch = "wasm32")]
            dct_simd_scratch: [i32x4_splat(0); MAX_TX_WIDTH],
            #[cfg(target_arch = "wasm32")]
            dct_scalar_scratch: [0; MAX_TX_WIDTH],
        }
    }

    pub(super) fn reset(&mut self, block: TransformBlock, eob: usize) {
        self.block = block;
        self.eob = eob;
        self.nonzero_row_mask = 0;
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
        #[cfg(target_arch = "wasm32")]
        {
            if !lossless {
                return self.inverse_transform_2d_simd(n, width, nonzero_row_mask);
            }
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
        let mut t = [0i32; MAX_TX_WIDTH];
        let active_rows = width.min(u32::BITS as usize - nonzero_row_mask.leading_zeros() as usize);

        for row in 0..active_rows {
            if nonzero_row_mask & (1u32 << row) == 0 {
                continue;
            }

            for (col, slot) in t.iter_mut().take(width).enumerate() {
                *slot = self.coefficients[row * width + col];
            }

            if lossless {
                inverse_wht(&mut t, 2)?;
            } else if row_transform_is_dct(self.block.tx_type) {
                inverse_dct_permutation(&mut t, n);
                inverse_dct(&mut t, n)?;
            } else {
                inverse_adst(&mut t, n)?;
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
                inverse_wht(&mut t, 0)?;
            } else if column_transform_is_dct(self.block.tx_type) {
                inverse_dct_permutation(&mut t, n);
                inverse_dct(&mut t, n)?;
            } else {
                inverse_adst(&mut t, n)?;
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

    #[cfg(target_arch = "wasm32")]
    fn inverse_transform_2d_simd(
        &mut self,
        n: usize,
        width: usize,
        nonzero_row_mask: u32,
    ) -> Result<(), TileSyntaxError> {
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

    #[cfg(target_arch = "wasm32")]
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
                    inverse_adst(t, n)?;
                }
            }
            for (col, value) in t.iter().take(width).copied().enumerate() {
                coefficients[row * width + col] = value;
            }
        }

        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    fn inverse_transform_row_group_simd(
        &mut self,
        n: usize,
        width: usize,
        rows: [usize; SIMD_TRANSFORM_LANES],
        transform: InverseTransform1d,
    ) -> Result<(), TileSyntaxError> {
        let t = &mut self.dct_simd_scratch;
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
                inverse_adst_simd(t, n)?;
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

    #[cfg(target_arch = "wasm32")]
    fn inverse_transform_columns_simd(
        &mut self,
        n: usize,
        width: usize,
        transform: InverseTransform1d,
    ) -> Result<(), TileSyntaxError> {
        let final_shift = core::cmp::min(6, n + 2);
        let t = &mut self.dct_simd_scratch;
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
                    inverse_adst_simd(t, n)?;
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

    pub(super) fn dequantize_into(
        self,
        input: &TransformCoefficients,
        segment_id: u8,
        scan: &[u16],
        output: &mut DequantizedCoefficients,
    ) -> Result<(), TileSyntaxError> {
        output.reset(input.block, input.eob);
        if input.eob == 0 {
            return Ok(());
        }

        let count = coefficient_count(input.block.tx_size);
        if input.eob > count {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let row_shift = 2 + input.block.tx_size.index();
        let dq_denom = dq_denom(input.block.tx_size);
        let dc_quant = self.get_dc_quant_for_segment(input.block.plane, segment_id);
        let ac_quant = self.get_ac_quant_for_segment(input.block.plane, segment_id);

        for c in 0..input.eob {
            let pos = usize::from(*scan.get(c).ok_or(TileSyntaxError::InvalidBitstream)?);
            if pos >= count {
                return Err(TileSyntaxError::InvalidBitstream);
            }

            let coefficient = *input
                .coefficients
                .get(pos)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            if coefficient == 0 {
                continue;
            }

            let quant = if pos == 0 { dc_quant } else { ac_quant };
            output.coefficients[pos] = (i32::from(coefficient) * quant) / dq_denom;
            output.nonzero_row_mask |= 1u32 << (pos >> row_shift);
        }

        Ok(())
    }
}

pub(super) const fn coefficient_count(tx_size: TxSize) -> usize {
    16 << (tx_size.index() << 1)
}

const MAX_TX_WIDTH: usize = 32;
#[cfg(target_arch = "wasm32")]
const SIMD_TRANSFORM_LANES: usize = 4;

#[cfg(target_arch = "wasm32")]
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

#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn i32x4_from_lanes(a: i32, b: i32, c: i32, d: i32) -> v128 {
    let value = i32x4_replace_lane::<0>(i32x4_splat(0), a);
    let value = i32x4_replace_lane::<1>(value, b);
    let value = i32x4_replace_lane::<2>(value, c);
    i32x4_replace_lane::<3>(value, d)
}

#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn load_i32x4(src: *const i32) -> v128 {
    // Wasm vector loads are byte-addressed and permit unaligned addresses. The
    // callers pass four in-bounds contiguous i32 coefficients.
    unsafe { v128_load(src.cast::<v128>()) }
}

#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn store_i32x4(dst: *mut i32, value: v128) {
    // Wasm vector stores are byte-addressed and permit unaligned addresses. The
    // callers pass four in-bounds contiguous i32 coefficient slots.
    unsafe { v128_store(dst.cast::<v128>(), value) }
}

#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn pack_i64x2_low_i32s(lo: v128, hi: v128) -> v128 {
    i32x4_shuffle::<0, 2, 4, 6>(lo, hi)
}

#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn round_shift_i64x2(value: v128, bits: usize) -> v128 {
    let rounding = i64x2_splat(1i64 << (bits - 1));
    i64x2_shr(i64x2_add(value, rounding), bits as u32)
}

#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn round_shift_i32x4(value: v128, bits: usize) -> v128 {
    let lo = round_shift_i64x2(i64x2_extend_low_i32x4(value), bits);
    let hi = round_shift_i64x2(i64x2_extend_high_i32x4(value), bits);
    pack_i64x2_low_i32s(lo, hi)
}

#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn round_pack_i64x2_low_i32s(lo: v128, hi: v128, bits: usize) -> v128 {
    pack_i64x2_low_i32s(round_shift_i64x2(lo, bits), round_shift_i64x2(hi, bits))
}

#[cfg(target_arch = "wasm32")]
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

#[cfg(target_arch = "wasm32")]
#[inline(always)]
fn h_simd(t: &mut [v128; MAX_TX_WIDTH], a: usize, b_index: usize, flip: bool) {
    let (x_index, y_index) = if flip { (b_index, a) } else { (a, b_index) };
    let x = t[x_index];
    let y = t[y_index];
    t[x_index] = i32x4_add(x, y);
    t[y_index] = i32x4_sub(x, y);
}

#[cfg(target_arch = "wasm32")]
fn sb_simd(
    t: &[v128; MAX_TX_WIDTH],
    s_lo: &mut [v128; MAX_TX_WIDTH],
    s_hi: &mut [v128; MAX_TX_WIDTH],
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

#[cfg(target_arch = "wasm32")]
fn sh_simd(
    t: &mut [v128; MAX_TX_WIDTH],
    s_lo: &[v128; MAX_TX_WIDTH],
    s_hi: &[v128; MAX_TX_WIDTH],
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
    s: &mut [i64; MAX_TX_WIDTH],
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
    s: &[i64; MAX_TX_WIDTH],
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

#[cfg(target_arch = "wasm32")]
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

#[cfg(target_arch = "wasm32")]
fn inverse_adst_input_permutation_simd(t: &mut [v128; MAX_TX_WIDTH], n: usize) {
    let n0 = 1usize << n;
    let n1 = 1usize << (n - 1);
    let mut copy_t = [i32x4_splat(0); MAX_TX_WIDTH];
    copy_t[..n0].copy_from_slice(&t[..n0]);
    for i in 0..n1 {
        t[2 * i] = copy_t[n0 - 1 - 2 * i];
        t[2 * i + 1] = copy_t[2 * i];
    }
}

#[cfg(target_arch = "wasm32")]
fn inverse_adst_output_permutation_simd(t: &mut [v128; MAX_TX_WIDTH], n: usize) {
    let len = 1usize << n;
    let mut copy_t = [i32x4_splat(0); MAX_TX_WIDTH];
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

#[cfg(target_arch = "wasm32")]
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

#[cfg(target_arch = "wasm32")]
fn inverse_adst8_simd(t: &mut [v128; MAX_TX_WIDTH]) -> Result<(), TileSyntaxError> {
    let mut s_lo = [i64x2_splat(0); MAX_TX_WIDTH];
    let mut s_hi = [i64x2_splat(0); MAX_TX_WIDTH];

    inverse_adst_input_permutation_simd(t, 3);
    for i in 0..=3 {
        sb_simd(
            t,
            &mut s_lo,
            &mut s_hi,
            2 * i,
            1 + 2 * i,
            30 - 8 * i as i32,
            true,
        );
    }
    for i in 0..=3 {
        sh_simd(t, &s_lo, &s_hi, i, 4 + i);
    }
    for i in 0..=1 {
        sb_simd(
            t,
            &mut s_lo,
            &mut s_hi,
            4 + 3 * i,
            5 + i,
            24 - 16 * i as i32,
            true,
        );
    }
    for i in 0..=1 {
        sh_simd(t, &s_lo, &s_hi, 4 + i, 6 + i);
    }
    for i in 0..=1 {
        h_simd(t, i, 2 + i, false);
    }
    for i in 0..=1 {
        b_simd(t, 2 + 4 * i, 3 + 4 * i, 16, true);
    }
    inverse_adst_output_permutation_simd(t, 3);
    for i in 0..=3 {
        let index = 1 + 2 * i;
        t[index] = i32x4_sub(i32x4_splat(0), t[index]);
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn inverse_adst16_simd(t: &mut [v128; MAX_TX_WIDTH]) -> Result<(), TileSyntaxError> {
    let mut s_lo = [i64x2_splat(0); MAX_TX_WIDTH];
    let mut s_hi = [i64x2_splat(0); MAX_TX_WIDTH];

    inverse_adst_input_permutation_simd(t, 4);
    for i in 0..=7 {
        sb_simd(
            t,
            &mut s_lo,
            &mut s_hi,
            2 * i,
            1 + 2 * i,
            31 - 4 * i as i32,
            true,
        );
    }
    for i in 0..=7 {
        sh_simd(t, &s_lo, &s_hi, i, 8 + i);
    }
    for i in 0..=3 {
        sb_simd(
            t,
            &mut s_lo,
            &mut s_hi,
            8 + 2 * i,
            9 + 2 * i,
            28 - 16 * i as i32,
            true,
        );
    }
    for i in 0..=3 {
        sh_simd(t, &s_lo, &s_hi, 8 + i, 12 + i);
    }
    for i in 0..=3 {
        h_simd(t, i, 4 + i, false);
    }
    for i in 0..=1 {
        for j in 0..=1 {
            sb_simd(
                t,
                &mut s_lo,
                &mut s_hi,
                4 + 8 * i + 3 * j,
                5 + 8 * i + j,
                24 - 16 * j as i32,
                true,
            );
        }
    }
    for i in 0..=1 {
        for j in 0..=1 {
            sh_simd(t, &s_lo, &s_hi, 4 + 8 * j + i, 6 + 8 * j + i);
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
    inverse_adst_output_permutation_simd(t, 4);
    for i in 0..=1 {
        for j in 0..=1 {
            let index = 1 + 12 * j + 2 * i;
            t[index] = i32x4_sub(i32x4_splat(0), t[index]);
        }
    }

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn inverse_adst_simd(t: &mut [v128; MAX_TX_WIDTH], n: usize) -> Result<(), TileSyntaxError> {
    match n {
        2 => inverse_adst4_simd(t),
        3 => inverse_adst8_simd(t),
        4 => inverse_adst16_simd(t),
        _ => Err(TileSyntaxError::InvalidBitstream),
    }
}

fn inverse_adst_input_permutation(t: &mut [i32; MAX_TX_WIDTH], n: usize) {
    let n0 = 1usize << n;
    let n1 = 1usize << (n - 1);
    let mut copy_t = [0i32; MAX_TX_WIDTH];
    copy_t[..n0].copy_from_slice(&t[..n0]);
    for i in 0..n1 {
        t[2 * i] = copy_t[n0 - 1 - 2 * i];
        t[2 * i + 1] = copy_t[2 * i];
    }
}

fn inverse_adst_output_permutation(t: &mut [i32; MAX_TX_WIDTH], n: usize) {
    let len = 1usize << n;
    let mut copy_t = [0i32; MAX_TX_WIDTH];
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

fn inverse_adst8(t: &mut [i32; MAX_TX_WIDTH]) -> Result<(), TileSyntaxError> {
    let mut s = [0i64; MAX_TX_WIDTH];

    inverse_adst_input_permutation(t, 3);
    for i in 0..=3 {
        sb(t, &mut s, 2 * i, 1 + 2 * i, 30 - 8 * i as i32, true);
    }
    for i in 0..=3 {
        sh(t, &s, i, 4 + i)?;
    }
    for i in 0..=1 {
        sb(t, &mut s, 4 + 3 * i, 5 + i, 24 - 16 * i as i32, true);
    }
    for i in 0..=1 {
        sh(t, &s, 4 + i, 6 + i)?;
    }
    for i in 0..=1 {
        h(t, i, 2 + i, false);
    }
    for i in 0..=1 {
        b(t, 2 + 4 * i, 3 + 4 * i, 16, true);
    }
    inverse_adst_output_permutation(t, 3);
    for i in 0..=3 {
        t[1 + 2 * i] = narrow_i32(-i64::from(t[1 + 2 * i]))?;
    }

    Ok(())
}

fn inverse_adst16(t: &mut [i32; MAX_TX_WIDTH]) -> Result<(), TileSyntaxError> {
    let mut s = [0i64; MAX_TX_WIDTH];

    inverse_adst_input_permutation(t, 4);
    for i in 0..=7 {
        sb(t, &mut s, 2 * i, 1 + 2 * i, 31 - 4 * i as i32, true);
    }
    for i in 0..=7 {
        sh(t, &s, i, 8 + i)?;
    }
    for i in 0..=3 {
        sb(t, &mut s, 8 + 2 * i, 9 + 2 * i, 28 - 16 * i as i32, true);
    }
    for i in 0..=3 {
        sh(t, &s, 8 + i, 12 + i)?;
    }
    for i in 0..=3 {
        h(t, i, 4 + i, false);
    }
    for i in 0..=1 {
        for j in 0..=1 {
            sb(
                t,
                &mut s,
                4 + 8 * i + 3 * j,
                5 + 8 * i + j,
                24 - 16 * j as i32,
                true,
            );
        }
    }
    for i in 0..=1 {
        for j in 0..=1 {
            sh(t, &s, 4 + 8 * j + i, 6 + 8 * j + i)?;
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
    inverse_adst_output_permutation(t, 4);
    for i in 0..=1 {
        for j in 0..=1 {
            let index = 1 + 12 * j + 2 * i;
            t[index] = narrow_i32(-i64::from(t[index]))?;
        }
    }

    Ok(())
}

fn inverse_adst(t: &mut [i32; MAX_TX_WIDTH], n: usize) -> Result<(), TileSyntaxError> {
    match n {
        2 => inverse_adst4(t),
        3 => inverse_adst8(t),
        4 => inverse_adst16(t),
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

const fn dq_denom(tx_size: TxSize) -> i32 {
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

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
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
        // raw i32 extremes. This spans roughly a medium coefficient multiplied
        // by the largest 8-bit AC dequantizer.
        let limit = AC_QLOOKUP_8BIT[255] * 32;

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
}

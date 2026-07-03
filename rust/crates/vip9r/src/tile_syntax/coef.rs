use super::*;

pub(super) fn scan_table(tx_size: TxSize, tx_type: TxType) -> &'static [u16] {
    match tx_size {
        TxSize::Tx4x4 => match tx_type {
            TxType::AdstDct => &ROW_SCAN_4X4,
            TxType::DctAdst => &COL_SCAN_4X4,
            TxType::DctDct | TxType::AdstAdst => &DEFAULT_SCAN_4X4,
        },
        TxSize::Tx8x8 => match tx_type {
            TxType::AdstDct => &ROW_SCAN_8X8,
            TxType::DctAdst => &COL_SCAN_8X8,
            TxType::DctDct | TxType::AdstAdst => &DEFAULT_SCAN_8X8,
        },
        TxSize::Tx16x16 => match tx_type {
            TxType::AdstDct => &ROW_SCAN_16X16,
            TxType::DctAdst => &COL_SCAN_16X16,
            TxType::DctDct | TxType::AdstAdst => &DEFAULT_SCAN_16X16,
        },
        TxSize::Tx32x32 => &DEFAULT_SCAN_32X32,
    }
}

pub(super) fn coef_band_table(tx_size: TxSize) -> &'static [u8] {
    match tx_size {
        TxSize::Tx4x4 => &COEFBAND_4X4,
        TxSize::Tx8x8 => &COEFBAND_8X8,
        TxSize::Tx16x16 => &COEFBAND_16X16,
        TxSize::Tx32x32 => &COEFBAND_32X32,
    }
}

#[inline(always)]
pub(super) fn read_more_coefs(
    decoder: &mut BoolDecoder<'_>,
    probability_row: &[u8; 3],
    counts: &mut [u32; 2],
) -> Result<bool, TileSyntaxError> {
    let more_coefs = decoder.read_bool(probability_row[0])?;
    increment_count(&mut counts[bool_index(more_coefs)]);
    Ok(more_coefs)
}

#[inline(always)]
pub(super) fn read_token(
    decoder: &mut BoolDecoder<'_>,
    probability_row: &[u8; 3],
    counts: &mut [u32; 3],
) -> Result<CoefToken, TileSyntaxError> {
    let mut node = 0usize;
    loop {
        let probability = token_probability(probability_row, node)?;
        let bit = usize::from(decoder.read_bool(probability)?);
        match TOKEN_TREE[node][bit] {
            TokenTreeBranch::Node(next) => node = usize::from(next),
            TokenTreeBranch::Token(token) => {
                increment_count(&mut counts[coef_token_count_index(token)]);
                return Ok(token);
            }
        }
    }
}

#[inline(always)]
pub(super) fn coef_token_count_index(token: CoefToken) -> usize {
    core::cmp::min(2, token.index())
}

#[inline(always)]
pub(super) fn read_coef(
    decoder: &mut BoolDecoder<'_>,
    token: CoefToken,
) -> Result<u32, TileSyntaxError> {
    let [cat, num_extra, base] = EXTRA_BITS[token.index()];
    let mut coef = u32::from(base);

    for e in 0..num_extra {
        let probability = CAT_PROBS[usize::from(cat)][usize::from(e)];
        let bit = u32::from(decoder.read_bool(probability)?);
        coef += bit << (u32::from(num_extra) - 1 - u32::from(e));
    }

    Ok(coef)
}

pub(super) fn coefficient_token_context(
    pos: usize,
    tx_size: TxSize,
    tx_type: TxType,
    token_cache: &[u8; MAX_TX_COEFFS],
) -> Result<usize, TileSyntaxError> {
    let n_shift = 2 + tx_size.index();
    let n = 1usize << n_shift;
    let i = pos >> n_shift;
    let j = pos & (n - 1);
    let (nb0, nb1) = if i > 0 && j > 0 {
        let a = ((i - 1) << n_shift) + j;
        let a2 = (i << n_shift) + j - 1;
        match tx_type {
            TxType::DctAdst => (a, a),
            TxType::AdstDct => (a2, a2),
            TxType::DctDct | TxType::AdstAdst => (a, a2),
        }
    } else if i > 0 {
        let a = ((i - 1) << n_shift) + j;
        (a, a)
    } else {
        let jm1 = j.checked_sub(1).ok_or(TileSyntaxError::InvalidBitstream)?;
        let a = (i << n_shift) + jm1;
        (a, a)
    };

    let cache0 = usize::from(
        *token_cache
            .get(nb0)
            .ok_or(TileSyntaxError::InvalidBitstream)?,
    );
    let cache1 = usize::from(
        *token_cache
            .get(nb1)
            .ok_or(TileSyntaxError::InvalidBitstream)?,
    );
    Ok((1 + cache0 + cache1) >> 1)
}

#[inline(always)]
pub(super) fn token_probability(
    probability_row: &[u8; 3],
    node: usize,
) -> Result<u8, TileSyntaxError> {
    if node == 0 {
        Ok(probability_row[1])
    } else if node == 1 {
        Ok(probability_row[2])
    } else {
        pareto(node - 2, probability_row[2])
    }
}

#[inline(always)]
pub(super) fn pareto(table_index: usize, prob: u8) -> Result<u8, TileSyntaxError> {
    if prob == 0 {
        return Err(TileSyntaxError::InvalidBitstream);
    }
    let x = usize::from((prob - 1) / 2);
    if prob & 1 != 0 {
        Ok(PARETO_TABLE[x][table_index])
    } else {
        let a = u16::from(PARETO_TABLE[x][table_index]);
        let b = u16::from(PARETO_TABLE[usize::from(prob / 2)][table_index]);
        Ok(((a + b) >> 1) as u8)
    }
}

pub(super) const fn coef_band_8x8plus<const N: usize>() -> [u8; N] {
    let mut bands = [0u8; N];
    let mut c = 0usize;
    while c < N {
        bands[c] = if c < 10 {
            COEFBAND_8X8PLUS_FIRST[c]
        } else if c < 21 {
            4
        } else {
            5
        };
        c += 1;
    }
    bands
}

pub(super) const COEFBAND_4X4: [u8; 16] = [0, 1, 1, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 5, 5, 5];
pub(super) const COEFBAND_8X8PLUS_FIRST: [u8; 10] = [0, 1, 1, 2, 2, 2, 3, 3, 3, 3];
pub(super) const COEFBAND_8X8: [u8; 64] = coef_band_8x8plus::<64>();
pub(super) const COEFBAND_16X16: [u8; 256] = coef_band_8x8plus::<256>();
pub(super) const COEFBAND_32X32: [u8; 1024] = coef_band_8x8plus::<1024>();

pub(super) const ENERGY_CLASS: [u8; 11] = [0, 1, 2, 3, 3, 4, 4, 5, 5, 5, 5];

#[derive(Clone, Copy)]
pub(super) enum TokenTreeBranch {
    Node(u8),
    Token(CoefToken),
}

pub(super) const TOKEN_TREE: [[TokenTreeBranch; 2]; 10] = [
    [
        TokenTreeBranch::Token(CoefToken::Zero),
        TokenTreeBranch::Node(1),
    ],
    [
        TokenTreeBranch::Token(CoefToken::One),
        TokenTreeBranch::Node(2),
    ],
    [TokenTreeBranch::Node(3), TokenTreeBranch::Node(5)],
    [
        TokenTreeBranch::Token(CoefToken::Two),
        TokenTreeBranch::Node(4),
    ],
    [
        TokenTreeBranch::Token(CoefToken::Three),
        TokenTreeBranch::Token(CoefToken::Four),
    ],
    [TokenTreeBranch::Node(6), TokenTreeBranch::Node(7)],
    [
        TokenTreeBranch::Token(CoefToken::DctValCategory1),
        TokenTreeBranch::Token(CoefToken::DctValCategory2),
    ],
    [TokenTreeBranch::Node(8), TokenTreeBranch::Node(9)],
    [
        TokenTreeBranch::Token(CoefToken::DctValCategory3),
        TokenTreeBranch::Token(CoefToken::DctValCategory4),
    ],
    [
        TokenTreeBranch::Token(CoefToken::DctValCategory5),
        TokenTreeBranch::Token(CoefToken::DctValCategory6),
    ],
];

pub(super) const EXTRA_BITS: [[u8; 3]; 11] = [
    [0, 0, 0],
    [0, 0, 1],
    [0, 0, 2],
    [0, 0, 3],
    [0, 0, 4],
    [1, 1, 5],
    [2, 2, 7],
    [3, 3, 11],
    [4, 4, 19],
    [5, 5, 35],
    [6, 14, 67],
];

pub(super) const CAT_PROBS: [[u8; 14]; 7] = [
    [0; 14],
    [159, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [165, 145, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [173, 148, 140, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [176, 155, 140, 135, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [180, 157, 141, 134, 130, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [
        254, 254, 254, 252, 249, 243, 230, 196, 177, 153, 140, 133, 130, 129,
    ],
];

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn all_zero_token_block_takes_immediate_more_coefs_zero_path() {
        let probabilities = FrameContext::DEFAULT;
        let mut contexts = TileModeContexts::new(1).unwrap();
        let mut counts = SyntaxCounts::default();
        let mut parser = TileParser {
            decoder: BoolDecoder::new(&[0x00, 0x00]).unwrap(),
            probabilities: &probabilities,
            counts: &mut counts,
            contexts: &mut contexts,
            tx_mode: TxMode::Only4x4,
            frame_is_intra: true,
            frame_width: 8,
            frame_height: 8,
            reference_mode: ReferenceMode::Single,
            compound_reference: None,
            interpolation_filter: None,
            allow_high_precision_mv: false,
            use_prev_frame_mvs: false,
            ref_frame_sign_bias: [false; 4],
            lossless: true,
            dequant: FrameDequant::new(0, 0, 0, 0),
            segmentation: crate::header::SegmentationParams::disabled(),
            segment_map_reset: false,
            mi_rows: 1,
            mi_cols: 1,
            tile_col_start: 0,
            tile_col_end: 1,
            left_row_base: 0,
            prev_frame_modes: None,
            current_frame_modes: None,
            reference_frames: None,
            intra: IntraPredictionBuffers::new(),
            residual: ResidualBuffers::new(),
            interp_buffer: [0; MAX_INTERP_BUFFER],
        };

        parser
            .tokens(
                0,
                (0, 0),
                TxSize::Tx4x4,
                0,
                BlockSize::Block8x8,
                test_block(false),
            )
            .unwrap();
        let coefficients = &parser.residual.dequantized;

        assert!(!coefficients.nonzero_context());
        assert_eq!(coefficients.eob, 0);
        assert_eq!(coefficients.coefficients[..16], [0; 16]);
        assert_eq!(coefficients.nonzero_row_mask, 0);
        assert!(!parser.residual.dequantized_dirty);
        assert_eq!(parser.decoder.bit_offset(), 1);
        assert_eq!(parser.decoder.finish(), Ok(()));
    }

    #[test]
    fn token_sign_bits_store_dequantized_coefficients_in_raster_position_order() {
        let probabilities = sign_bit_test_probabilities();

        for (data, expected) in [([0x01, 0x80], 4), ([0x40, 0x80], -4)] {
            let mut contexts = TileModeContexts::new(1).unwrap();
            let mut counts = SyntaxCounts::default();
            let mut parser = TileParser {
                decoder: BoolDecoder::new(&data).unwrap(),
                probabilities: &probabilities,
                counts: &mut counts,
                contexts: &mut contexts,
                tx_mode: TxMode::Only4x4,
                frame_is_intra: true,
                frame_width: 8,
                frame_height: 8,
                reference_mode: ReferenceMode::Single,
                compound_reference: None,
                interpolation_filter: None,
                allow_high_precision_mv: false,
                use_prev_frame_mvs: false,
                ref_frame_sign_bias: [false; 4],
                lossless: true,
                dequant: FrameDequant::new(0, 0, 0, 0),
                segmentation: crate::header::SegmentationParams::disabled(),
                segment_map_reset: false,
                mi_rows: 1,
                mi_cols: 1,
                tile_col_start: 0,
                tile_col_end: 1,
                left_row_base: 0,
                prev_frame_modes: None,
                current_frame_modes: None,
                reference_frames: None,
                intra: IntraPredictionBuffers::new(),
                residual: ResidualBuffers::new(),
                interp_buffer: [0; MAX_INTERP_BUFFER],
            };

            parser
                .tokens(
                    0,
                    (4, 8),
                    TxSize::Tx4x4,
                    0,
                    BlockSize::Block8x8,
                    test_block(false),
                )
                .unwrap();
            let coefficients = &parser.residual.dequantized;

            assert!(coefficients.nonzero_context());
            assert_eq!(coefficients.eob, 2);
            assert_eq!(coefficients.block.plane, 0);
            assert_eq!(coefficients.block.start, (4, 8));
            assert_eq!(coefficients.block.tx_size, TxSize::Tx4x4);
            assert_eq!(coefficients.block.tx_type, TxType::DctDct);
            assert_eq!(coefficients.coefficients[0], 0);
            assert_eq!(coefficients.coefficients[1], 0);
            assert_eq!(coefficients.coefficients[4], expected);
            assert_eq!(coefficients.nonzero_row_mask, 0b10);
            assert!(parser.residual.dequantized_dirty);
            assert_eq!(parser.decoder.finish(), Ok(()));
        }
    }

    #[test]
    fn token_workspace_clears_previous_dequantized_block_before_reuse() {
        let sign_probabilities = sign_bit_test_probabilities();
        let zero_probabilities = FrameContext::DEFAULT;
        let mut contexts = TileModeContexts::new(1).unwrap();
        let mut counts = SyntaxCounts::default();
        let mut parser = TileParser {
            decoder: BoolDecoder::new(&[0x01, 0x80]).unwrap(),
            probabilities: &sign_probabilities,
            counts: &mut counts,
            contexts: &mut contexts,
            tx_mode: TxMode::Only4x4,
            frame_is_intra: true,
            frame_width: 8,
            frame_height: 8,
            reference_mode: ReferenceMode::Single,
            compound_reference: None,
            interpolation_filter: None,
            allow_high_precision_mv: false,
            use_prev_frame_mvs: false,
            ref_frame_sign_bias: [false; 4],
            lossless: true,
            dequant: FrameDequant::new(0, 0, 0, 0),
            segmentation: crate::header::SegmentationParams::disabled(),
            segment_map_reset: false,
            mi_rows: 1,
            mi_cols: 1,
            tile_col_start: 0,
            tile_col_end: 1,
            left_row_base: 0,
            prev_frame_modes: None,
            current_frame_modes: None,
            reference_frames: None,
            intra: IntraPredictionBuffers::new(),
            residual: ResidualBuffers::new(),
            interp_buffer: [0; MAX_INTERP_BUFFER],
        };

        parser
            .tokens(
                0,
                (4, 8),
                TxSize::Tx4x4,
                0,
                BlockSize::Block8x8,
                test_block(false),
            )
            .unwrap();
        assert_eq!(parser.residual.dequantized.coefficients[4], 4);
        assert_eq!(parser.residual.dequantized.nonzero_row_mask, 0b10);
        assert!(parser.residual.dequantized_dirty);
        assert_eq!(parser.residual.token_cache[..16], [0; 16]);

        parser.decoder = BoolDecoder::new(&[0x00, 0x00]).unwrap();
        parser.probabilities = &zero_probabilities;
        parser
            .tokens(
                0,
                (4, 8),
                TxSize::Tx4x4,
                0,
                BlockSize::Block8x8,
                test_block(false),
            )
            .unwrap();

        assert_eq!(parser.residual.dequantized.eob, 0);
        assert_eq!(parser.residual.dequantized.coefficients[4], 0);
        assert_eq!(parser.residual.dequantized.nonzero_row_mask, 0);
        assert!(!parser.residual.dequantized_dirty);
        assert_eq!(parser.decoder.finish(), Ok(()));
    }

    #[test]
    fn category_token_extra_bits_are_parsed() {
        let probabilities = FrameContext::DEFAULT;
        let mut contexts = TileModeContexts::new(1).unwrap();
        let mut counts = SyntaxCounts::default();
        let mut parser = TileParser {
            decoder: BoolDecoder::new(&[0x00, 0x00]).unwrap(),
            probabilities: &probabilities,
            counts: &mut counts,
            contexts: &mut contexts,
            tx_mode: TxMode::Only4x4,
            frame_is_intra: true,
            frame_width: 8,
            frame_height: 8,
            reference_mode: ReferenceMode::Single,
            compound_reference: None,
            interpolation_filter: None,
            allow_high_precision_mv: false,
            use_prev_frame_mvs: false,
            ref_frame_sign_bias: [false; 4],
            lossless: true,
            dequant: FrameDequant::new(0, 0, 0, 0),
            segmentation: crate::header::SegmentationParams::disabled(),
            segment_map_reset: false,
            mi_rows: 1,
            mi_cols: 1,
            tile_col_start: 0,
            tile_col_end: 1,
            left_row_base: 0,
            prev_frame_modes: None,
            current_frame_modes: None,
            reference_frames: None,
            intra: IntraPredictionBuffers::new(),
            residual: ResidualBuffers::new(),
            interp_buffer: [0; MAX_INTERP_BUFFER],
        };

        assert_eq!(
            read_coef(&mut parser.decoder, CoefToken::DctValCategory1),
            Ok(5)
        );
        assert_eq!(parser.decoder.bit_offset(), 1);
        assert_eq!(parser.decoder.finish(), Ok(()));

        let mut contexts = TileModeContexts::new(1).unwrap();
        let mut counts = SyntaxCounts::default();
        let mut parser = TileParser {
            decoder: BoolDecoder::new(&[0x50, 0x00]).unwrap(),
            probabilities: &probabilities,
            counts: &mut counts,
            contexts: &mut contexts,
            tx_mode: TxMode::Only4x4,
            frame_is_intra: true,
            frame_width: 8,
            frame_height: 8,
            reference_mode: ReferenceMode::Single,
            compound_reference: None,
            interpolation_filter: None,
            allow_high_precision_mv: false,
            use_prev_frame_mvs: false,
            ref_frame_sign_bias: [false; 4],
            lossless: true,
            dequant: FrameDequant::new(0, 0, 0, 0),
            segmentation: crate::header::SegmentationParams::disabled(),
            segment_map_reset: false,
            mi_rows: 1,
            mi_cols: 1,
            tile_col_start: 0,
            tile_col_end: 1,
            left_row_base: 0,
            prev_frame_modes: None,
            current_frame_modes: None,
            reference_frames: None,
            intra: IntraPredictionBuffers::new(),
            residual: ResidualBuffers::new(),
            interp_buffer: [0; MAX_INTERP_BUFFER],
        };

        assert_eq!(
            read_coef(&mut parser.decoder, CoefToken::DctValCategory1),
            Ok(6)
        );
        assert_eq!(parser.decoder.finish(), Ok(()));
    }
}

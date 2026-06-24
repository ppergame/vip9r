use crate::boolcoder::BoolDecoder;
use crate::error::ParserError;
use crate::header::UncompressedFrameHeader;
use crate::probability::{
    COEF_BANDS, FrameContext, PREV_COEF_CONTEXTS, SKIP_CONTEXTS, TX_SIZE_CONTEXTS, TX_SIZES,
    UNCONSTRAINED_NODES,
};

const DIFF_UPDATE_PROBABILITY: u8 = 252;
const MAX_PROB: u8 = 255;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TxMode {
    Only4x4,
    Allow8x8,
    Allow16x16,
    Allow32x32,
    Select,
}

impl TxMode {
    fn from_raw(raw: u8) -> Result<Self, ParserError> {
        match raw {
            0 => Ok(Self::Only4x4),
            1 => Ok(Self::Allow8x8),
            2 => Ok(Self::Allow16x16),
            3 => Ok(Self::Allow32x32),
            4 => Ok(Self::Select),
            _ => Err(ParserError::InvalidBitstream),
        }
    }

    pub(crate) fn biggest_tx_size(self) -> usize {
        match self {
            Self::Only4x4 => 0,
            Self::Allow8x8 => 1,
            Self::Allow16x16 => 2,
            Self::Allow32x32 | Self::Select => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompressedHeader {
    pub(crate) tx_mode: TxMode,
}

pub(crate) fn parse_intra_compressed_header(
    data: &[u8],
    header: &UncompressedFrameHeader,
    probabilities: &mut FrameContext,
) -> Result<CompressedHeader, ParserError> {
    if !header.frame_is_intra {
        return Err(ParserError::InvalidBitstream);
    }

    let mut decoder = BoolDecoder::new(data)?;
    let tx_mode = read_tx_mode(&mut decoder, header.lossless)?;
    if tx_mode == TxMode::Select {
        tx_mode_probs(&mut decoder, probabilities)?;
    }
    read_coef_probs(&mut decoder, probabilities, tx_mode)?;
    read_skip_prob(&mut decoder, probabilities)?;
    decoder.finish()?;

    Ok(CompressedHeader { tx_mode })
}

fn read_tx_mode(decoder: &mut BoolDecoder<'_>, lossless: bool) -> Result<TxMode, ParserError> {
    if lossless {
        return Ok(TxMode::Only4x4);
    }

    let mut raw = decoder.read_literal(2)? as u8;
    if raw == 3 {
        raw = raw
            .checked_add(decoder.read_literal(1)? as u8)
            .ok_or(ParserError::InvalidBitstream)?;
    }
    TxMode::from_raw(raw)
}

fn tx_mode_probs(
    decoder: &mut BoolDecoder<'_>,
    probabilities: &mut FrameContext,
) -> Result<(), ParserError> {
    for i in 0..TX_SIZE_CONTEXTS {
        for j in 0..(TX_SIZES - 3) {
            probabilities.tx_probs[1][i][j] =
                diff_update_prob(decoder, probabilities.tx_probs[1][i][j])?;
        }
    }
    for i in 0..TX_SIZE_CONTEXTS {
        for j in 0..(TX_SIZES - 2) {
            probabilities.tx_probs[2][i][j] =
                diff_update_prob(decoder, probabilities.tx_probs[2][i][j])?;
        }
    }
    for i in 0..TX_SIZE_CONTEXTS {
        for j in 0..(TX_SIZES - 1) {
            probabilities.tx_probs[3][i][j] =
                diff_update_prob(decoder, probabilities.tx_probs[3][i][j])?;
        }
    }
    Ok(())
}

fn read_coef_probs(
    decoder: &mut BoolDecoder<'_>,
    probabilities: &mut FrameContext,
    tx_mode: TxMode,
) -> Result<(), ParserError> {
    for tx_size in 0..=tx_mode.biggest_tx_size() {
        let update_probs = decoder.read_literal(1)? != 0;
        if !update_probs {
            continue;
        }

        for block_type in 0..2 {
            for ref_type in 0..2 {
                for band in 0..COEF_BANDS {
                    let max_l = if band == 0 { 3 } else { PREV_COEF_CONTEXTS };
                    for context in 0..max_l {
                        for node in 0..UNCONSTRAINED_NODES {
                            probabilities.coef_probs[tx_size][block_type][ref_type][band]
                                [context][node] = diff_update_prob(
                                decoder,
                                probabilities.coef_probs[tx_size][block_type][ref_type][band]
                                    [context][node],
                            )?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn read_skip_prob(
    decoder: &mut BoolDecoder<'_>,
    probabilities: &mut FrameContext,
) -> Result<(), ParserError> {
    for i in 0..SKIP_CONTEXTS {
        probabilities.skip_prob[i] = diff_update_prob(decoder, probabilities.skip_prob[i])?;
    }
    Ok(())
}

fn diff_update_prob(decoder: &mut BoolDecoder<'_>, prob: u8) -> Result<u8, ParserError> {
    if decoder.read_bool(DIFF_UPDATE_PROBABILITY)? {
        let delta_prob = decode_term_subexp(decoder)?;
        inv_remap_prob(delta_prob, prob)
    } else {
        Ok(prob)
    }
}

fn decode_term_subexp(decoder: &mut BoolDecoder<'_>) -> Result<u8, ParserError> {
    if decoder.read_literal(1)? == 0 {
        return u8::try_from(decoder.read_literal(4)?).map_err(|_| ParserError::InvalidBitstream);
    }
    if decoder.read_literal(1)? == 0 {
        return u8::try_from(decoder.read_literal(4)? + 16)
            .map_err(|_| ParserError::InvalidBitstream);
    }
    if decoder.read_literal(1)? == 0 {
        return u8::try_from(decoder.read_literal(5)? + 32)
            .map_err(|_| ParserError::InvalidBitstream);
    }

    let v = decoder.read_literal(7)?;
    let value = if v < 65 {
        v + 64
    } else {
        (v << 1) - 1 + decoder.read_literal(1)?
    };
    if value >= u32::from(MAX_PROB) {
        return Err(ParserError::InvalidBitstream);
    }
    u8::try_from(value).map_err(|_| ParserError::InvalidBitstream)
}

fn inv_remap_prob(delta_prob: u8, prob: u8) -> Result<u8, ParserError> {
    if delta_prob == MAX_PROB || prob == 0 {
        return Err(ParserError::InvalidBitstream);
    }

    let v = u16::from(INV_MAP_TABLE[usize::from(delta_prob)]);
    let m = u16::from(prob) - 1;
    let remapped = if (m << 1) <= 255 {
        1 + inv_recenter_nonneg(v, m)
    } else {
        255 - inv_recenter_nonneg(v, 255 - 1 - m)
    };
    u8::try_from(remapped).map_err(|_| ParserError::InvalidBitstream)
}

fn inv_recenter_nonneg(v: u16, m: u16) -> u16 {
    if v > 2 * m {
        return v;
    }
    if v & 1 != 0 {
        return m - ((v + 1) >> 1);
    }
    m + (v >> 1)
}

const INV_MAP_TABLE: [u8; MAX_PROB as usize] = [
    7, 20, 33, 46, 59, 72, 85, 98, 111, 124, 137, 150, 163, 176, 189, 202, 215, 228, 241, 254, 1,
    2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 21, 22, 23, 24, 25, 26, 27, 28,
    29, 30, 31, 32, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 47, 48, 49, 50, 51, 52, 53, 54,
    55, 56, 57, 58, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 73, 74, 75, 76, 77, 78, 79, 80,
    81, 82, 83, 84, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 99, 100, 101, 102, 103, 104,
    105, 106, 107, 108, 109, 110, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 125,
    126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 138, 139, 140, 141, 142, 143, 144, 145,
    146, 147, 148, 149, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 164, 165, 166,
    167, 168, 169, 170, 171, 172, 173, 174, 175, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186,
    187, 188, 190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 203, 204, 205, 206, 207,
    208, 209, 210, 211, 212, 213, 214, 216, 217, 218, 219, 220, 221, 222, 223, 224, 225, 226, 227,
    229, 230, 231, 232, 233, 234, 235, 236, 237, 238, 239, 240, 242, 243, 244, 245, 246, 247, 248,
    249, 250, 251, 252, 253, 253,
];

#[cfg(test)]
mod tests {
    use super::{
        TxMode, diff_update_prob, inv_remap_prob, parse_intra_compressed_header, read_tx_mode,
    };
    use crate::boolcoder::BoolDecoder;
    use crate::error::ParserError;
    use crate::header::{FrameType, UncompressedFrameHeader};
    use crate::probability::FrameContext;

    #[test]
    fn lossless_forces_only_4x4() {
        let mut probabilities = FrameContext::DEFAULT;
        let parsed =
            parse_intra_compressed_header(&[0x00, 0x00], &test_header(true), &mut probabilities)
                .unwrap();

        assert_eq!(parsed.tx_mode, TxMode::Only4x4);
    }

    #[test]
    fn non_lossless_tx_mode_literal_and_select_behavior() {
        let mut literal = BoolDecoder::new(&[0x40, 0x00]).unwrap();
        assert_eq!(read_tx_mode(&mut literal, false), Ok(TxMode::Allow16x16));
        assert_eq!(literal.finish(), Ok(()));

        let mut select = BoolDecoder::new(&[0x70, 0x00]).unwrap();
        assert_eq!(read_tx_mode(&mut select, false), Ok(TxMode::Select));
        assert_eq!(select.finish(), Ok(()));
    }

    #[test]
    fn diff_update_prob_handles_update_and_no_update_paths() {
        let mut no_update = BoolDecoder::new(&[0x00, 0x00]).unwrap();
        assert_eq!(diff_update_prob(&mut no_update, 128), Ok(128));
        assert_eq!(no_update.finish(), Ok(()));

        let mut update = BoolDecoder::new(&[0x7e, 0x00, 0x00]).unwrap();
        assert_eq!(inv_remap_prob(0, 128), Ok(124));
        assert_eq!(diff_update_prob(&mut update, 128), Ok(124));
        assert_eq!(update.finish(), Ok(()));
    }

    #[test]
    fn compressed_header_finish_rejects_non_zero_padding() {
        let mut probabilities = FrameContext::DEFAULT;

        assert_eq!(
            parse_intra_compressed_header(&[0x00, 0x01], &test_header(true), &mut probabilities),
            Err(ParserError::InvalidBitstream)
        );
    }

    fn test_header(lossless: bool) -> UncompressedFrameHeader {
        UncompressedFrameHeader {
            profile: 0,
            bit_depth: 8,
            frame_type: FrameType::Key,
            show_frame: true,
            show_existing_frame: false,
            frame_to_show_map_idx: None,
            error_resilient_mode: false,
            intra_only: false,
            frame_is_intra: true,
            reset_frame_context: 0,
            refresh_frame_context: true,
            frame_parallel_decoding_mode: false,
            raw_frame_context_idx: 0,
            frame_context_idx: 0,
            refresh_frame_flags: 0xff,
            ref_frame_idx: [0; 3],
            ref_frame_sign_bias: [false; 4],
            allow_high_precision_mv: false,
            interpolation_filter: None,
            frame_width: 16,
            frame_height: 16,
            render_width: 16,
            render_height: 16,
            base_q_idx: if lossless { 0 } else { 1 },
            delta_q_y_dc: 0,
            delta_q_uv_dc: 0,
            delta_q_uv_ac: 0,
            lossless,
            segmentation_enabled: false,
            segmentation_update_map: false,
            tile_cols_log2: 0,
            tile_rows_log2: 0,
            header_size_in_bytes: 2,
            compressed_header_offset: 0,
            tile_data_offset: 2,
        }
    }
}

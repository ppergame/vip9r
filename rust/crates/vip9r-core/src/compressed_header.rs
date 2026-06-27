use crate::boolcoder::BoolDecoder;
use crate::error::ParserError;
use crate::header::{InterpolationFilter, UncompressedFrameHeader};
use crate::probability::{
    BLOCK_SIZE_GROUPS, CLASS0_SIZE, COEF_BANDS, COMP_MODE_CONTEXTS, FrameContext,
    INTER_MODE_CONTEXTS, INTER_MODES, INTERP_FILTER_CONTEXTS, INTRA_MODES, IS_INTER_CONTEXTS,
    MV_CLASSES, MV_FR_SIZE, MV_JOINTS, MV_OFFSET_BITS, PARTITION_CONTEXTS, PARTITION_TYPES,
    PREV_COEF_CONTEXTS, REF_CONTEXTS, SKIP_CONTEXTS, SWITCHABLE_FILTERS, TX_SIZE_CONTEXTS,
    TX_SIZES, UNCONSTRAINED_NODES,
};

const DIFF_UPDATE_PROBABILITY: u8 = 252;
const MV_UPDATE_PROBABILITY: u8 = 252;
const MAX_PROB: u8 = 255;
const LAST_FRAME_INDEX: usize = 1;
const GOLDEN_FRAME_INDEX: usize = 2;
const ALTREF_FRAME_INDEX: usize = 3;

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
pub(crate) enum ReferenceMode {
    Single,
    Compound,
    Select,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InterReferenceFrame {
    Last,
    Golden,
    Altref,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompoundReferenceSetup {
    pub(crate) comp_fixed_ref: InterReferenceFrame,
    pub(crate) comp_var_ref: [InterReferenceFrame; 2],
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompressedHeader {
    pub(crate) tx_mode: TxMode,
    pub(crate) reference_mode: ReferenceMode,
    pub(crate) compound_reference: Option<CompoundReferenceSetup>,
}

impl CompressedHeader {
    pub(crate) const fn intra(tx_mode: TxMode) -> Self {
        Self {
            tx_mode,
            reference_mode: ReferenceMode::Single,
            compound_reference: None,
        }
    }
}

pub(crate) fn parse_intra_compressed_header(
    data: &[u8],
    header: &UncompressedFrameHeader,
    probabilities: &mut FrameContext,
) -> Result<CompressedHeader, ParserError> {
    if !header.frame_is_intra {
        return Err(ParserError::InvalidBitstream);
    }

    parse_compressed_header(data, header, probabilities)
}

pub(crate) fn parse_inter_compressed_header(
    data: &[u8],
    header: &UncompressedFrameHeader,
    probabilities: &mut FrameContext,
) -> Result<CompressedHeader, ParserError> {
    if header.frame_is_intra {
        return Err(ParserError::InvalidBitstream);
    }

    parse_compressed_header(data, header, probabilities)
}

fn parse_compressed_header(
    data: &[u8],
    header: &UncompressedFrameHeader,
    probabilities: &mut FrameContext,
) -> Result<CompressedHeader, ParserError> {
    let mut decoder = BoolDecoder::new(data)?;
    let tx_mode = read_tx_mode(&mut decoder, header.lossless)?;
    if tx_mode == TxMode::Select {
        tx_mode_probs(&mut decoder, probabilities)?;
    }
    read_coef_probs(&mut decoder, probabilities, tx_mode)?;
    read_skip_prob(&mut decoder, probabilities)?;

    let mut compressed_header = CompressedHeader::intra(tx_mode);
    if !header.frame_is_intra {
        read_inter_mode_probs(&mut decoder, probabilities)?;
        match header.interpolation_filter {
            Some(InterpolationFilter::Switchable) => {
                read_interp_filter_probs(&mut decoder, probabilities)?;
            }
            Some(_) => {}
            None => return Err(ParserError::InvalidBitstream),
        }
        read_is_inter_probs(&mut decoder, probabilities)?;
        let reference_mode = frame_reference_mode(&mut decoder, header)?;
        frame_reference_mode_probs(&mut decoder, probabilities, reference_mode.reference_mode)?;
        read_y_mode_probs(&mut decoder, probabilities)?;
        read_partition_probs(&mut decoder, probabilities)?;
        read_mv_probs(&mut decoder, probabilities, header.allow_high_precision_mv)?;

        compressed_header.reference_mode = reference_mode.reference_mode;
        compressed_header.compound_reference = reference_mode.compound_reference;
    }

    decoder.finish()?;

    Ok(compressed_header)
}

fn read_tx_mode(decoder: &mut BoolDecoder<'_>, lossless: bool) -> Result<TxMode, ParserError> {
    if lossless {
        return Ok(TxMode::Only4x4);
    }

    let mut raw = decoder.read_literal(2)? as u8;
    if raw == 3 {
        raw += decoder.read_literal(1)? as u8;
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

fn read_inter_mode_probs(
    decoder: &mut BoolDecoder<'_>,
    probabilities: &mut FrameContext,
) -> Result<(), ParserError> {
    for i in 0..INTER_MODE_CONTEXTS {
        for j in 0..(INTER_MODES - 1) {
            probabilities.inter_mode_probs[i][j] =
                diff_update_prob(decoder, probabilities.inter_mode_probs[i][j])?;
        }
    }
    Ok(())
}

fn read_interp_filter_probs(
    decoder: &mut BoolDecoder<'_>,
    probabilities: &mut FrameContext,
) -> Result<(), ParserError> {
    for i in 0..INTERP_FILTER_CONTEXTS {
        for j in 0..(SWITCHABLE_FILTERS - 1) {
            probabilities.interp_filter_probs[i][j] =
                diff_update_prob(decoder, probabilities.interp_filter_probs[i][j])?;
        }
    }
    Ok(())
}

fn read_is_inter_probs(
    decoder: &mut BoolDecoder<'_>,
    probabilities: &mut FrameContext,
) -> Result<(), ParserError> {
    for i in 0..IS_INTER_CONTEXTS {
        probabilities.is_inter_prob[i] = diff_update_prob(decoder, probabilities.is_inter_prob[i])?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameReferenceMode {
    reference_mode: ReferenceMode,
    compound_reference: Option<CompoundReferenceSetup>,
}

fn frame_reference_mode(
    decoder: &mut BoolDecoder<'_>,
    header: &UncompressedFrameHeader,
) -> Result<FrameReferenceMode, ParserError> {
    let last_sign_bias = sign_bias(header, LAST_FRAME_INDEX)?;
    let mut compound_reference_allowed = false;
    for index in GOLDEN_FRAME_INDEX..=ALTREF_FRAME_INDEX {
        if sign_bias(header, index)? != last_sign_bias {
            compound_reference_allowed = true;
            break;
        }
    }

    if !compound_reference_allowed {
        return Ok(FrameReferenceMode {
            reference_mode: ReferenceMode::Single,
            compound_reference: None,
        });
    }

    let non_single_reference = decoder.read_literal(1)? != 0;
    if !non_single_reference {
        return Ok(FrameReferenceMode {
            reference_mode: ReferenceMode::Single,
            compound_reference: None,
        });
    }

    let reference_select = decoder.read_literal(1)? != 0;
    let reference_mode = if reference_select {
        ReferenceMode::Select
    } else {
        ReferenceMode::Compound
    };

    Ok(FrameReferenceMode {
        reference_mode,
        compound_reference: Some(setup_compound_reference_mode(header)?),
    })
}

fn frame_reference_mode_probs(
    decoder: &mut BoolDecoder<'_>,
    probabilities: &mut FrameContext,
    reference_mode: ReferenceMode,
) -> Result<(), ParserError> {
    if reference_mode == ReferenceMode::Select {
        for i in 0..COMP_MODE_CONTEXTS {
            probabilities.comp_mode_prob[i] =
                diff_update_prob(decoder, probabilities.comp_mode_prob[i])?;
        }
    }

    if reference_mode != ReferenceMode::Compound {
        for i in 0..REF_CONTEXTS {
            probabilities.single_ref_prob[i][0] =
                diff_update_prob(decoder, probabilities.single_ref_prob[i][0])?;
            probabilities.single_ref_prob[i][1] =
                diff_update_prob(decoder, probabilities.single_ref_prob[i][1])?;
        }
    }

    if reference_mode != ReferenceMode::Single {
        for i in 0..REF_CONTEXTS {
            probabilities.comp_ref_prob[i] =
                diff_update_prob(decoder, probabilities.comp_ref_prob[i])?;
        }
    }

    Ok(())
}

fn read_y_mode_probs(
    decoder: &mut BoolDecoder<'_>,
    probabilities: &mut FrameContext,
) -> Result<(), ParserError> {
    for i in 0..BLOCK_SIZE_GROUPS {
        for j in 0..(INTRA_MODES - 1) {
            probabilities.y_mode_probs[i][j] =
                diff_update_prob(decoder, probabilities.y_mode_probs[i][j])?;
        }
    }
    Ok(())
}

fn read_partition_probs(
    decoder: &mut BoolDecoder<'_>,
    probabilities: &mut FrameContext,
) -> Result<(), ParserError> {
    for i in 0..PARTITION_CONTEXTS {
        for j in 0..(PARTITION_TYPES - 1) {
            probabilities.partition_probs[i][j] =
                diff_update_prob(decoder, probabilities.partition_probs[i][j])?;
        }
    }
    Ok(())
}

fn read_mv_probs(
    decoder: &mut BoolDecoder<'_>,
    probabilities: &mut FrameContext,
    allow_high_precision_mv: bool,
) -> Result<(), ParserError> {
    for i in 0..(MV_JOINTS - 1) {
        probabilities.mv_probs.joint[i] = update_mv_prob(decoder, probabilities.mv_probs.joint[i])?;
    }

    for i in 0..2 {
        probabilities.mv_probs.sign[i] = update_mv_prob(decoder, probabilities.mv_probs.sign[i])?;
        for j in 0..(MV_CLASSES - 1) {
            probabilities.mv_probs.class[i][j] =
                update_mv_prob(decoder, probabilities.mv_probs.class[i][j])?;
        }
        probabilities.mv_probs.class0_bit[i] =
            update_mv_prob(decoder, probabilities.mv_probs.class0_bit[i])?;
        for j in 0..MV_OFFSET_BITS {
            probabilities.mv_probs.bits[i][j] =
                update_mv_prob(decoder, probabilities.mv_probs.bits[i][j])?;
        }
    }

    for i in 0..2 {
        for j in 0..CLASS0_SIZE {
            for k in 0..(MV_FR_SIZE - 1) {
                probabilities.mv_probs.class0_fr[i][j][k] =
                    update_mv_prob(decoder, probabilities.mv_probs.class0_fr[i][j][k])?;
            }
        }
        for k in 0..(MV_FR_SIZE - 1) {
            probabilities.mv_probs.fr[i][k] =
                update_mv_prob(decoder, probabilities.mv_probs.fr[i][k])?;
        }
    }

    if allow_high_precision_mv {
        for i in 0..2 {
            probabilities.mv_probs.class0_hp[i] =
                update_mv_prob(decoder, probabilities.mv_probs.class0_hp[i])?;
            probabilities.mv_probs.hp[i] = update_mv_prob(decoder, probabilities.mv_probs.hp[i])?;
        }
    }

    Ok(())
}

fn update_mv_prob(decoder: &mut BoolDecoder<'_>, prob: u8) -> Result<u8, ParserError> {
    if decoder.read_bool(MV_UPDATE_PROBABILITY)? {
        let mv_prob = decoder.read_literal(7)?;
        Ok(((mv_prob as u8) << 1) | 1)
    } else {
        Ok(prob)
    }
}

fn setup_compound_reference_mode(
    header: &UncompressedFrameHeader,
) -> Result<CompoundReferenceSetup, ParserError> {
    let last_sign_bias = sign_bias(header, LAST_FRAME_INDEX)?;
    let golden_sign_bias = sign_bias(header, GOLDEN_FRAME_INDEX)?;
    let altref_sign_bias = sign_bias(header, ALTREF_FRAME_INDEX)?;

    if last_sign_bias == golden_sign_bias {
        Ok(CompoundReferenceSetup {
            comp_fixed_ref: InterReferenceFrame::Altref,
            comp_var_ref: [InterReferenceFrame::Last, InterReferenceFrame::Golden],
        })
    } else if last_sign_bias == altref_sign_bias {
        Ok(CompoundReferenceSetup {
            comp_fixed_ref: InterReferenceFrame::Golden,
            comp_var_ref: [InterReferenceFrame::Last, InterReferenceFrame::Altref],
        })
    } else {
        Ok(CompoundReferenceSetup {
            comp_fixed_ref: InterReferenceFrame::Last,
            comp_var_ref: [InterReferenceFrame::Golden, InterReferenceFrame::Altref],
        })
    }
}

fn sign_bias(header: &UncompressedFrameHeader, index: usize) -> Result<bool, ParserError> {
    header
        .ref_frame_sign_bias
        .get(index)
        .copied()
        .ok_or(ParserError::InvalidBitstream)
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
        return Ok(decoder.read_literal(4)? as u8);
    }
    if decoder.read_literal(1)? == 0 {
        return Ok((decoder.read_literal(4)? + 16) as u8);
    }
    if decoder.read_literal(1)? == 0 {
        return Ok((decoder.read_literal(5)? + 32) as u8);
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
    Ok(value as u8)
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
    Ok(remapped as u8)
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
        InterReferenceFrame, ReferenceMode, TxMode, diff_update_prob, frame_reference_mode,
        inv_remap_prob, parse_inter_compressed_header, parse_intra_compressed_header, read_tx_mode,
        setup_compound_reference_mode, update_mv_prob,
    };
    use crate::boolcoder::BoolDecoder;
    use crate::error::ParserError;
    use crate::header::{FrameType, InterpolationFilter, UncompressedFrameHeader};
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

    #[test]
    fn inter_compressed_header_no_updates_without_compound_reference() {
        let mut probabilities = FrameContext::DEFAULT;
        let header = test_inter_header(
            true,
            InterpolationFilter::Switchable,
            [false, false, false, false],
        );

        let parsed = parse_inter_compressed_header(&[0; 64], &header, &mut probabilities).unwrap();

        assert_eq!(parsed.tx_mode, TxMode::Only4x4);
        assert_eq!(parsed.reference_mode, ReferenceMode::Single);
        assert_eq!(parsed.compound_reference, None);
        assert_eq!(probabilities, FrameContext::DEFAULT);
    }

    #[test]
    fn update_mv_prob_handles_update_and_no_update_paths() {
        let mut no_update = BoolDecoder::new(&[0x00, 0x00]).unwrap();
        assert_eq!(update_mv_prob(&mut no_update, 128), Ok(128));
        assert_eq!(no_update.finish(), Ok(()));

        let mut update = BoolDecoder::new(&[0x7e, 0x00, 0x00]).unwrap();
        assert_eq!(update_mv_prob(&mut update, 128), Ok(1));
        assert_eq!(update.finish(), Ok(()));
    }

    #[test]
    fn frame_reference_mode_selects_single_compound_and_select_modes() {
        let no_compound_header = test_inter_header(
            true,
            InterpolationFilter::EightTap,
            [false, false, false, false],
        );
        let mut no_compound = BoolDecoder::new(&[0x00, 0x00]).unwrap();
        let parsed = frame_reference_mode(&mut no_compound, &no_compound_header).unwrap();
        assert_eq!(parsed.reference_mode, ReferenceMode::Single);
        assert_eq!(parsed.compound_reference, None);
        assert_eq!(no_compound.finish(), Ok(()));

        let compound_header = test_inter_header(
            true,
            InterpolationFilter::EightTap,
            [false, false, false, true],
        );
        let mut compound = BoolDecoder::new(&[0x40, 0x00]).unwrap();
        let parsed = frame_reference_mode(&mut compound, &compound_header).unwrap();
        assert_eq!(parsed.reference_mode, ReferenceMode::Compound);
        assert_eq!(
            parsed.compound_reference.unwrap().comp_fixed_ref,
            InterReferenceFrame::Altref
        );
        assert_eq!(compound.finish(), Ok(()));

        let mut select = BoolDecoder::new(&[0x60, 0x00]).unwrap();
        let parsed = frame_reference_mode(&mut select, &compound_header).unwrap();
        assert_eq!(parsed.reference_mode, ReferenceMode::Select);
    }

    #[test]
    fn compound_reference_setup_follows_sign_bias_branches() {
        let fixed_altref = setup_compound_reference_mode(&test_inter_header(
            true,
            InterpolationFilter::EightTap,
            [false, false, false, true],
        ))
        .unwrap();
        assert_eq!(fixed_altref.comp_fixed_ref, InterReferenceFrame::Altref);
        assert_eq!(
            fixed_altref.comp_var_ref,
            [InterReferenceFrame::Last, InterReferenceFrame::Golden]
        );

        let fixed_golden = setup_compound_reference_mode(&test_inter_header(
            true,
            InterpolationFilter::EightTap,
            [false, true, false, true],
        ))
        .unwrap();
        assert_eq!(fixed_golden.comp_fixed_ref, InterReferenceFrame::Golden);
        assert_eq!(
            fixed_golden.comp_var_ref,
            [InterReferenceFrame::Last, InterReferenceFrame::Altref]
        );

        let fixed_last = setup_compound_reference_mode(&test_inter_header(
            true,
            InterpolationFilter::EightTap,
            [false, false, true, true],
        ))
        .unwrap();
        assert_eq!(fixed_last.comp_fixed_ref, InterReferenceFrame::Last);
        assert_eq!(
            fixed_last.comp_var_ref,
            [InterReferenceFrame::Golden, InterReferenceFrame::Altref]
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

    fn test_inter_header(
        lossless: bool,
        interpolation_filter: InterpolationFilter,
        ref_frame_sign_bias: [bool; 4],
    ) -> UncompressedFrameHeader {
        UncompressedFrameHeader {
            profile: 0,
            bit_depth: 8,
            frame_type: FrameType::NonKey,
            show_frame: true,
            show_existing_frame: false,
            frame_to_show_map_idx: None,
            error_resilient_mode: false,
            intra_only: false,
            frame_is_intra: false,
            reset_frame_context: 0,
            refresh_frame_context: true,
            frame_parallel_decoding_mode: false,
            raw_frame_context_idx: 0,
            frame_context_idx: 0,
            refresh_frame_flags: 0x01,
            ref_frame_idx: [0, 1, 2],
            ref_frame_sign_bias,
            allow_high_precision_mv: false,
            interpolation_filter: Some(interpolation_filter),
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
            header_size_in_bytes: 64,
            compressed_header_offset: 0,
            tile_data_offset: 64,
        }
    }
}

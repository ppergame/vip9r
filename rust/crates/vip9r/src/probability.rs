use crate::error::ParserError;

pub(crate) const FRAME_CONTEXTS: usize = 4;
pub(crate) const TX_SIZES: usize = 4;
pub(crate) const TX_SIZE_CONTEXTS: usize = 2;
pub(crate) const SKIP_CONTEXTS: usize = 3;
pub(crate) const BLOCK_TYPES: usize = 2;
pub(crate) const REF_TYPES: usize = 2;
pub(crate) const COEF_BANDS: usize = 6;
pub(crate) const PREV_COEF_CONTEXTS: usize = 6;
pub(crate) const UNCONSTRAINED_NODES: usize = 3;
pub(crate) const INTER_MODE_CONTEXTS: usize = 7;
pub(crate) const INTER_MODES: usize = 4;
pub(crate) const INTERP_FILTER_CONTEXTS: usize = 4;
pub(crate) const SWITCHABLE_FILTERS: usize = 3;
pub(crate) const IS_INTER_CONTEXTS: usize = 4;
pub(crate) const COMP_MODE_CONTEXTS: usize = 5;
pub(crate) const REF_CONTEXTS: usize = 5;
pub(crate) const BLOCK_SIZE_GROUPS: usize = 4;
pub(crate) const INTRA_MODES: usize = 10;
pub(crate) const PARTITION_CONTEXTS: usize = 16;
pub(crate) const PARTITION_TYPES: usize = 4;
pub(crate) const MV_JOINTS: usize = 4;
pub(crate) const MV_CLASSES: usize = 11;
pub(crate) const CLASS0_SIZE: usize = 2;
pub(crate) const MV_OFFSET_BITS: usize = 10;
pub(crate) const MV_FR_SIZE: usize = 4;

pub(crate) type TxProbs = [[[u8; TX_SIZES - 1]; TX_SIZE_CONTEXTS]; TX_SIZES];
pub(crate) type CoefProbs = [[[[[[u8; UNCONSTRAINED_NODES]; PREV_COEF_CONTEXTS]; COEF_BANDS];
    REF_TYPES]; BLOCK_TYPES]; TX_SIZES];
pub(crate) type InterModeProbs = [[u8; INTER_MODES - 1]; INTER_MODE_CONTEXTS];
pub(crate) type InterpFilterProbs = [[u8; SWITCHABLE_FILTERS - 1]; INTERP_FILTER_CONTEXTS];
pub(crate) type IsInterProb = [u8; IS_INTER_CONTEXTS];
pub(crate) type CompModeProb = [u8; COMP_MODE_CONTEXTS];
pub(crate) type SingleRefProb = [[u8; 2]; REF_CONTEXTS];
pub(crate) type CompRefProb = [u8; REF_CONTEXTS];
pub(crate) type YModeProbs = [[u8; INTRA_MODES - 1]; BLOCK_SIZE_GROUPS];
pub(crate) type UvModeProbs = [[u8; INTRA_MODES - 1]; INTRA_MODES];
pub(crate) type PartitionProbs = [[u8; PARTITION_TYPES - 1]; PARTITION_CONTEXTS];
pub(crate) type CoefTokenCounts = [[[[[[u32; UNCONSTRAINED_NODES]; PREV_COEF_CONTEXTS]; COEF_BANDS];
    REF_TYPES]; BLOCK_TYPES]; TX_SIZES];
pub(crate) type MoreCoefCounts =
    [[[[[[u32; 2]; PREV_COEF_CONTEXTS]; COEF_BANDS]; REF_TYPES]; BLOCK_TYPES]; TX_SIZES];

const COUNT_SAT: u32 = 20;
const MAX_UPDATE_FACTOR: u32 = 128;
const COEF_COUNT_SAT: u32 = 24;

const BINARY_TREE: [i8; 2] = [0, -1];
const SMALL_TOKEN_TREE: [i8; 6] = [0, 0, 0, 4, -1, -2];
const INTRA_MODE_TREE: [i8; 18] = [
    0, 2, -9, 4, -1, 6, 8, 12, -2, 10, -4, -5, -3, 14, -8, 16, -6, -7,
];
const PARTITION_TREE: [i8; 6] = [0, 2, -1, 4, -2, -3];
const INTER_MODE_TREE: [i8; 6] = [-2, 2, 0, 4, -1, -3];
const INTERP_FILTER_TREE: [i8; 4] = [0, 2, -1, -2];
const TX_SIZE_8_TREE: [i8; 2] = [0, -1];
const TX_SIZE_16_TREE: [i8; 4] = [0, 2, -1, -2];
const TX_SIZE_32_TREE: [i8; 6] = [0, 2, -1, 4, -2, -3];
const MV_JOINT_TREE: [i8; 6] = [0, 2, -1, 4, -2, -3];
const MV_CLASS_TREE: [i8; 20] = [
    0, 2, -1, 4, 6, 8, -2, -3, 10, 12, -4, -5, -6, 14, 16, 18, -7, -8, -9, -10,
];
const MV_FR_TREE: [i8; 6] = [0, 2, -1, 4, -2, -3];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SyntaxCounts {
    pub(crate) counts_intra_mode: [[u32; INTRA_MODES]; BLOCK_SIZE_GROUPS],
    pub(crate) counts_uv_mode: [[u32; INTRA_MODES]; INTRA_MODES],
    pub(crate) counts_partition: [[u32; PARTITION_TYPES]; PARTITION_CONTEXTS],
    pub(crate) counts_interp_filter: [[u32; SWITCHABLE_FILTERS]; INTERP_FILTER_CONTEXTS],
    pub(crate) counts_inter_mode: [[u32; INTER_MODES]; INTER_MODE_CONTEXTS],
    pub(crate) counts_tx_size: [[[u32; TX_SIZES]; TX_SIZE_CONTEXTS]; TX_SIZES],
    pub(crate) counts_is_inter: [[u32; 2]; IS_INTER_CONTEXTS],
    pub(crate) counts_comp_mode: [[u32; 2]; COMP_MODE_CONTEXTS],
    pub(crate) counts_single_ref: [[[u32; 2]; 2]; REF_CONTEXTS],
    pub(crate) counts_comp_ref: [[u32; 2]; REF_CONTEXTS],
    pub(crate) counts_skip: [[u32; 2]; SKIP_CONTEXTS],
    pub(crate) counts_mv_joint: [u32; MV_JOINTS],
    pub(crate) counts_mv_sign: [[u32; 2]; 2],
    pub(crate) counts_mv_class: [[u32; MV_CLASSES]; 2],
    pub(crate) counts_mv_class0_bit: [[u32; CLASS0_SIZE]; 2],
    pub(crate) counts_mv_class0_fr: [[[u32; MV_FR_SIZE]; CLASS0_SIZE]; 2],
    pub(crate) counts_mv_class0_hp: [[u32; 2]; 2],
    pub(crate) counts_mv_bits: [[[u32; 2]; MV_OFFSET_BITS]; 2],
    pub(crate) counts_mv_fr: [[u32; MV_FR_SIZE]; 2],
    pub(crate) counts_mv_hp: [[u32; 2]; 2],
    pub(crate) counts_token: CoefTokenCounts,
    pub(crate) counts_more_coefs: MoreCoefCounts,
}

impl SyntaxCounts {
    /// All-zero counts, usable as a static initializer.
    pub(crate) const ZERO: Self = Self {
        counts_intra_mode: [[0; INTRA_MODES]; BLOCK_SIZE_GROUPS],
        counts_uv_mode: [[0; INTRA_MODES]; INTRA_MODES],
        counts_partition: [[0; PARTITION_TYPES]; PARTITION_CONTEXTS],
        counts_interp_filter: [[0; SWITCHABLE_FILTERS]; INTERP_FILTER_CONTEXTS],
        counts_inter_mode: [[0; INTER_MODES]; INTER_MODE_CONTEXTS],
        counts_tx_size: [[[0; TX_SIZES]; TX_SIZE_CONTEXTS]; TX_SIZES],
        counts_is_inter: [[0; 2]; IS_INTER_CONTEXTS],
        counts_comp_mode: [[0; 2]; COMP_MODE_CONTEXTS],
        counts_single_ref: [[[0; 2]; 2]; REF_CONTEXTS],
        counts_comp_ref: [[0; 2]; REF_CONTEXTS],
        counts_skip: [[0; 2]; SKIP_CONTEXTS],
        counts_mv_joint: [0; MV_JOINTS],
        counts_mv_sign: [[0; 2]; 2],
        counts_mv_class: [[0; MV_CLASSES]; 2],
        counts_mv_class0_bit: [[0; CLASS0_SIZE]; 2],
        counts_mv_class0_fr: [[[0; MV_FR_SIZE]; CLASS0_SIZE]; 2],
        counts_mv_class0_hp: [[0; 2]; 2],
        counts_mv_bits: [[[0; 2]; MV_OFFSET_BITS]; 2],
        counts_mv_fr: [[0; MV_FR_SIZE]; 2],
        counts_mv_hp: [[0; 2]; 2],
        counts_token: [[[[[[0; UNCONSTRAINED_NODES]; PREV_COEF_CONTEXTS]; COEF_BANDS]; REF_TYPES];
            BLOCK_TYPES]; TX_SIZES],
        counts_more_coefs: [[[[[[0; 2]; PREV_COEF_CONTEXTS]; COEF_BANDS]; REF_TYPES]; BLOCK_TYPES];
            TX_SIZES],
    };

    pub(crate) fn clear(&mut self) {
        *self = Self::ZERO;
    }

    pub(crate) fn merge_from(&mut self, other: &Self) {
        self.counts_intra_mode.add_from(&other.counts_intra_mode);
        self.counts_uv_mode.add_from(&other.counts_uv_mode);
        self.counts_partition.add_from(&other.counts_partition);
        self.counts_interp_filter
            .add_from(&other.counts_interp_filter);
        self.counts_inter_mode.add_from(&other.counts_inter_mode);
        self.counts_tx_size.add_from(&other.counts_tx_size);
        self.counts_is_inter.add_from(&other.counts_is_inter);
        self.counts_comp_mode.add_from(&other.counts_comp_mode);
        self.counts_single_ref.add_from(&other.counts_single_ref);
        self.counts_comp_ref.add_from(&other.counts_comp_ref);
        self.counts_skip.add_from(&other.counts_skip);
        self.counts_mv_joint.add_from(&other.counts_mv_joint);
        self.counts_mv_sign.add_from(&other.counts_mv_sign);
        self.counts_mv_class.add_from(&other.counts_mv_class);
        self.counts_mv_class0_bit
            .add_from(&other.counts_mv_class0_bit);
        self.counts_mv_class0_fr
            .add_from(&other.counts_mv_class0_fr);
        self.counts_mv_class0_hp
            .add_from(&other.counts_mv_class0_hp);
        self.counts_mv_bits.add_from(&other.counts_mv_bits);
        self.counts_mv_fr.add_from(&other.counts_mv_fr);
        self.counts_mv_hp.add_from(&other.counts_mv_hp);
        self.counts_token.add_from(&other.counts_token);
        self.counts_more_coefs.add_from(&other.counts_more_coefs);
    }
}

trait AddCounts {
    fn add_from(&mut self, other: &Self);
}

impl AddCounts for u32 {
    fn add_from(&mut self, other: &Self) {
        *self = self.saturating_add(*other);
    }
}

impl<T: AddCounts, const N: usize> AddCounts for [T; N] {
    fn add_from(&mut self, other: &Self) {
        for (dst, src) in self.iter_mut().zip(other) {
            dst.add_from(src);
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NonCoefAdaptationConfig {
    pub(crate) tx_mode_select: bool,
    pub(crate) interpolation_filter_switchable: bool,
    pub(crate) allow_high_precision_mv: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MvProbs {
    pub(crate) joint: [u8; MV_JOINTS - 1],
    pub(crate) sign: [u8; 2],
    pub(crate) class: [[u8; MV_CLASSES - 1]; 2],
    pub(crate) class0_bit: [u8; 2],
    pub(crate) bits: [[u8; MV_OFFSET_BITS]; 2],
    pub(crate) class0_fr: [[[u8; MV_FR_SIZE - 1]; CLASS0_SIZE]; 2],
    pub(crate) fr: [[u8; MV_FR_SIZE - 1]; 2],
    pub(crate) class0_hp: [u8; 2],
    pub(crate) hp: [u8; 2],
}

impl MvProbs {
    const DEFAULT: Self = Self {
        joint: DEFAULT_MV_JOINT_PROBS,
        sign: DEFAULT_MV_SIGN_PROB,
        class: DEFAULT_MV_CLASS_PROBS,
        class0_bit: DEFAULT_MV_CLASS0_BIT_PROB,
        bits: DEFAULT_MV_BITS_PROB,
        class0_fr: DEFAULT_MV_CLASS0_FR_PROBS,
        fr: DEFAULT_MV_FR_PROBS,
        class0_hp: DEFAULT_MV_CLASS0_HP_PROB,
        hp: DEFAULT_MV_HP_PROB,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameContext {
    pub(crate) tx_probs: TxProbs,
    pub(crate) coef_probs: CoefProbs,
    pub(crate) skip_prob: [u8; SKIP_CONTEXTS],
    pub(crate) inter_mode_probs: InterModeProbs,
    pub(crate) interp_filter_probs: InterpFilterProbs,
    pub(crate) is_inter_prob: IsInterProb,
    pub(crate) comp_mode_prob: CompModeProb,
    pub(crate) single_ref_prob: SingleRefProb,
    pub(crate) comp_ref_prob: CompRefProb,
    pub(crate) y_mode_probs: YModeProbs,
    pub(crate) uv_mode_probs: UvModeProbs,
    pub(crate) partition_probs: PartitionProbs,
    pub(crate) mv_probs: MvProbs,
}

impl FrameContext {
    pub(crate) const DEFAULT: Self = Self {
        tx_probs: DEFAULT_TX_PROBS,
        coef_probs: DEFAULT_COEF_PROBS,
        skip_prob: DEFAULT_SKIP_PROB,
        inter_mode_probs: DEFAULT_INTER_MODE_PROBS,
        interp_filter_probs: DEFAULT_INTERP_FILTER_PROBS,
        is_inter_prob: DEFAULT_IS_INTER_PROB,
        comp_mode_prob: DEFAULT_COMP_MODE_PROB,
        single_ref_prob: DEFAULT_SINGLE_REF_PROB,
        comp_ref_prob: DEFAULT_COMP_REF_PROB,
        y_mode_probs: DEFAULT_Y_MODE_PROBS,
        uv_mode_probs: DEFAULT_UV_MODE_PROBS,
        partition_probs: DEFAULT_PARTITION_PROBS,
        mv_probs: MvProbs::DEFAULT,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProbabilityState {
    contexts: [FrameContext; FRAME_CONTEXTS],
    current: FrameContext,
}

impl ProbabilityState {
    pub(crate) const fn new() -> Self {
        Self {
            contexts: [FrameContext::DEFAULT; FRAME_CONTEXTS],
            current: FrameContext::DEFAULT,
        }
    }

    pub(crate) fn setup_past_independence(&mut self) {
        self.current = FrameContext::DEFAULT;
    }

    pub(crate) fn reset_all_contexts(&mut self) {
        self.contexts = [FrameContext::DEFAULT; FRAME_CONTEXTS];
    }

    pub(crate) fn load_probs(&mut self, ctx: u8) -> Result<(), ParserError> {
        let context = self.context(ctx)?;
        self.current.coef_probs = context.coef_probs;
        self.current.inter_mode_probs = context.inter_mode_probs;
        self.current.interp_filter_probs = context.interp_filter_probs;
        self.current.is_inter_prob = context.is_inter_prob;
        self.current.comp_mode_prob = context.comp_mode_prob;
        self.current.single_ref_prob = context.single_ref_prob;
        self.current.comp_ref_prob = context.comp_ref_prob;
        self.current.y_mode_probs = context.y_mode_probs;
        self.current.uv_mode_probs = context.uv_mode_probs;
        self.current.partition_probs = context.partition_probs;
        self.current.mv_probs = context.mv_probs;
        Ok(())
    }

    pub(crate) fn load_probs2(&mut self, ctx: u8) -> Result<(), ParserError> {
        let context = self.context(ctx)?;
        self.current.tx_probs = context.tx_probs;
        self.current.skip_prob = context.skip_prob;
        Ok(())
    }

    pub(crate) fn save_probs(&mut self, ctx: u8) -> Result<(), ParserError> {
        let current = self.current;
        let slot = self.context_mut(ctx)?;
        *slot = current;
        Ok(())
    }

    pub(crate) fn current_mut(&mut self) -> &mut FrameContext {
        &mut self.current
    }

    pub(crate) fn current(&self) -> &FrameContext {
        &self.current
    }

    pub(crate) fn adapt_coef_probs(&mut self, counts: &SyntaxCounts, update_factor: u32) {
        self.current.adapt_coef_probs(counts, update_factor);
    }

    pub(crate) fn adapt_noncoef_probs(
        &mut self,
        counts: &SyntaxCounts,
        config: NonCoefAdaptationConfig,
    ) {
        self.current.adapt_noncoef_probs(counts, config);
    }

    fn context(&self, ctx: u8) -> Result<FrameContext, ParserError> {
        self.contexts
            .get(usize::from(ctx))
            .copied()
            .ok_or(ParserError::InvalidBitstream)
    }

    fn context_mut(&mut self, ctx: u8) -> Result<&mut FrameContext, ParserError> {
        self.contexts
            .get_mut(usize::from(ctx))
            .ok_or(ParserError::InvalidBitstream)
    }
}

impl FrameContext {
    fn adapt_coef_probs(&mut self, counts: &SyntaxCounts, update_factor: u32) {
        for tx_size in 0..TX_SIZES {
            for block_type in 0..BLOCK_TYPES {
                for ref_type in 0..REF_TYPES {
                    for band in 0..COEF_BANDS {
                        let max_l = if band == 0 { 3 } else { PREV_COEF_CONTEXTS };
                        for context in 0..max_l {
                            merge_probs(
                                &SMALL_TOKEN_TREE,
                                2,
                                &mut self.coef_probs[tx_size][block_type][ref_type][band][context],
                                &counts.counts_token[tx_size][block_type][ref_type][band][context],
                                COEF_COUNT_SAT,
                                update_factor,
                            );
                            merge_probs(
                                &BINARY_TREE,
                                0,
                                &mut self.coef_probs[tx_size][block_type][ref_type][band][context],
                                &counts.counts_more_coefs[tx_size][block_type][ref_type][band]
                                    [context],
                                COEF_COUNT_SAT,
                                update_factor,
                            );
                        }
                    }
                }
            }
        }
    }

    fn adapt_noncoef_probs(&mut self, counts: &SyntaxCounts, config: NonCoefAdaptationConfig) {
        for i in 0..IS_INTER_CONTEXTS {
            self.is_inter_prob[i] = adapt_prob(self.is_inter_prob[i], &counts.counts_is_inter[i]);
        }
        for i in 0..COMP_MODE_CONTEXTS {
            self.comp_mode_prob[i] =
                adapt_prob(self.comp_mode_prob[i], &counts.counts_comp_mode[i]);
        }
        for i in 0..REF_CONTEXTS {
            self.comp_ref_prob[i] = adapt_prob(self.comp_ref_prob[i], &counts.counts_comp_ref[i]);
        }
        for i in 0..REF_CONTEXTS {
            for j in 0..2 {
                self.single_ref_prob[i][j] =
                    adapt_prob(self.single_ref_prob[i][j], &counts.counts_single_ref[i][j]);
            }
        }
        for i in 0..INTER_MODE_CONTEXTS {
            adapt_probs(
                &INTER_MODE_TREE,
                &mut self.inter_mode_probs[i],
                &counts.counts_inter_mode[i],
            );
        }
        for i in 0..BLOCK_SIZE_GROUPS {
            adapt_probs(
                &INTRA_MODE_TREE,
                &mut self.y_mode_probs[i],
                &counts.counts_intra_mode[i],
            );
        }
        for i in 0..INTRA_MODES {
            adapt_probs(
                &INTRA_MODE_TREE,
                &mut self.uv_mode_probs[i],
                &counts.counts_uv_mode[i],
            );
        }
        for i in 0..PARTITION_CONTEXTS {
            adapt_probs(
                &PARTITION_TREE,
                &mut self.partition_probs[i],
                &counts.counts_partition[i],
            );
        }
        for i in 0..SKIP_CONTEXTS {
            self.skip_prob[i] = adapt_prob(self.skip_prob[i], &counts.counts_skip[i]);
        }
        if config.interpolation_filter_switchable {
            for i in 0..INTERP_FILTER_CONTEXTS {
                adapt_probs(
                    &INTERP_FILTER_TREE,
                    &mut self.interp_filter_probs[i],
                    &counts.counts_interp_filter[i],
                );
            }
        }
        if config.tx_mode_select {
            for i in 0..TX_SIZE_CONTEXTS {
                adapt_probs(
                    &TX_SIZE_8_TREE,
                    &mut self.tx_probs[1][i],
                    &counts.counts_tx_size[1][i],
                );
                adapt_probs(
                    &TX_SIZE_16_TREE,
                    &mut self.tx_probs[2][i],
                    &counts.counts_tx_size[2][i],
                );
                adapt_probs(
                    &TX_SIZE_32_TREE,
                    &mut self.tx_probs[3][i],
                    &counts.counts_tx_size[3][i],
                );
            }
        }
        adapt_probs(
            &MV_JOINT_TREE,
            &mut self.mv_probs.joint,
            &counts.counts_mv_joint,
        );
        for comp in 0..2 {
            self.mv_probs.sign[comp] =
                adapt_prob(self.mv_probs.sign[comp], &counts.counts_mv_sign[comp]);
            adapt_probs(
                &MV_CLASS_TREE,
                &mut self.mv_probs.class[comp],
                &counts.counts_mv_class[comp],
            );
            self.mv_probs.class0_bit[comp] = adapt_prob(
                self.mv_probs.class0_bit[comp],
                &counts.counts_mv_class0_bit[comp],
            );
            for i in 0..MV_OFFSET_BITS {
                self.mv_probs.bits[comp][i] =
                    adapt_prob(self.mv_probs.bits[comp][i], &counts.counts_mv_bits[comp][i]);
            }
            for i in 0..CLASS0_SIZE {
                adapt_probs(
                    &MV_FR_TREE,
                    &mut self.mv_probs.class0_fr[comp][i],
                    &counts.counts_mv_class0_fr[comp][i],
                );
            }
            adapt_probs(
                &MV_FR_TREE,
                &mut self.mv_probs.fr[comp],
                &counts.counts_mv_fr[comp],
            );
            if config.allow_high_precision_mv {
                self.mv_probs.class0_hp[comp] = adapt_prob(
                    self.mv_probs.class0_hp[comp],
                    &counts.counts_mv_class0_hp[comp],
                );
                self.mv_probs.hp[comp] =
                    adapt_prob(self.mv_probs.hp[comp], &counts.counts_mv_hp[comp]);
            }
        }
    }
}

fn adapt_probs(tree: &[i8], probs: &mut [u8], counts: &[u32]) {
    merge_probs(tree, 0, probs, counts, COUNT_SAT, MAX_UPDATE_FACTOR);
}

fn adapt_prob(prob: u8, counts: &[u32; 2]) -> u8 {
    merge_prob(prob, counts[0], counts[1], COUNT_SAT, MAX_UPDATE_FACTOR)
}

fn merge_probs(
    tree: &[i8],
    index: usize,
    probs: &mut [u8],
    counts: &[u32],
    count_sat: u32,
    max_update_factor: u32,
) -> u32 {
    let left = tree[index];
    let left_count = if left <= 0 {
        counts[usize::from(left.unsigned_abs())]
    } else {
        merge_probs(
            tree,
            usize::from(left as u8),
            probs,
            counts,
            count_sat,
            max_update_factor,
        )
    };

    let right = tree[index + 1];
    let right_count = if right <= 0 {
        counts[usize::from(right.unsigned_abs())]
    } else {
        merge_probs(
            tree,
            usize::from(right as u8),
            probs,
            counts,
            count_sat,
            max_update_factor,
        )
    };

    probs[index >> 1] = merge_prob(
        probs[index >> 1],
        left_count,
        right_count,
        count_sat,
        max_update_factor,
    );
    left_count.saturating_add(right_count)
}

fn merge_prob(pre_prob: u8, ct0: u32, ct1: u32, count_sat: u32, max_update_factor: u32) -> u8 {
    let den = ct0.saturating_add(ct1);
    let prob = if den == 0 {
        128
    } else {
        let estimate = (u64::from(ct0) * 256 + u64::from(den >> 1)) / u64::from(den);
        estimate.clamp(1, 255) as u32
    };
    let count = core::cmp::min(den, count_sat);
    let factor = max_update_factor * count / count_sat;
    let merged = u32::from(pre_prob) * (256 - factor) + prob * factor;
    ((merged + 128) >> 8) as u8
}

const DEFAULT_SKIP_PROB: [u8; SKIP_CONTEXTS] = [192, 128, 64];

const DEFAULT_IS_INTER_PROB: IsInterProb = [9, 102, 187, 225];

const DEFAULT_COMP_MODE_PROB: CompModeProb = [239, 183, 119, 96, 41];

const DEFAULT_COMP_REF_PROB: CompRefProb = [50, 126, 123, 221, 226];

const DEFAULT_SINGLE_REF_PROB: SingleRefProb =
    [[33, 16], [77, 74], [142, 142], [172, 170], [238, 247]];

const DEFAULT_INTER_MODE_PROBS: InterModeProbs = [
    [2, 173, 34],
    [7, 145, 85],
    [7, 166, 63],
    [7, 94, 66],
    [8, 64, 46],
    [17, 81, 31],
    [25, 29, 30],
];

const DEFAULT_INTERP_FILTER_PROBS: InterpFilterProbs = [[235, 162], [36, 255], [34, 3], [149, 144]];

const DEFAULT_Y_MODE_PROBS: YModeProbs = [
    [65, 32, 18, 144, 162, 194, 41, 51, 98],
    [132, 68, 18, 165, 217, 196, 45, 40, 78],
    [173, 80, 19, 176, 240, 193, 64, 35, 46],
    [221, 135, 38, 194, 248, 121, 96, 85, 29],
];

const DEFAULT_UV_MODE_PROBS: UvModeProbs = [
    [120, 7, 76, 176, 208, 126, 28, 54, 103],
    [48, 12, 154, 155, 139, 90, 34, 117, 119],
    [67, 6, 25, 204, 243, 158, 13, 21, 96],
    [97, 5, 44, 131, 176, 139, 48, 68, 97],
    [83, 5, 42, 156, 111, 152, 26, 49, 152],
    [80, 5, 58, 178, 74, 83, 33, 62, 145],
    [86, 5, 32, 154, 192, 168, 14, 22, 163],
    [85, 5, 32, 156, 216, 148, 19, 29, 73],
    [77, 7, 64, 116, 132, 122, 37, 126, 120],
    [101, 21, 107, 181, 192, 103, 19, 67, 125],
];

const DEFAULT_PARTITION_PROBS: PartitionProbs = [
    [199, 122, 141],
    [147, 63, 159],
    [148, 133, 118],
    [121, 104, 114],
    [174, 73, 87],
    [92, 41, 83],
    [82, 99, 50],
    [53, 39, 39],
    [177, 58, 59],
    [68, 26, 63],
    [52, 79, 25],
    [17, 14, 12],
    [222, 34, 30],
    [72, 16, 44],
    [58, 32, 12],
    [10, 7, 6],
];

const DEFAULT_MV_JOINT_PROBS: [u8; MV_JOINTS - 1] = [32, 64, 96];

const DEFAULT_MV_SIGN_PROB: [u8; 2] = [128, 128];

const DEFAULT_MV_CLASS_PROBS: [[u8; MV_CLASSES - 1]; 2] = [
    [224, 144, 192, 168, 192, 176, 192, 198, 198, 245],
    [216, 128, 176, 160, 176, 176, 192, 198, 198, 208],
];

const DEFAULT_MV_CLASS0_BIT_PROB: [u8; 2] = [216, 208];

const DEFAULT_MV_BITS_PROB: [[u8; MV_OFFSET_BITS]; 2] = [
    [136, 140, 148, 160, 176, 192, 224, 234, 234, 240],
    [136, 140, 148, 160, 176, 192, 224, 234, 234, 240],
];

const DEFAULT_MV_CLASS0_FR_PROBS: [[[u8; MV_FR_SIZE - 1]; CLASS0_SIZE]; 2] = [
    [[128, 128, 64], [96, 112, 64]],
    [[128, 128, 64], [96, 112, 64]],
];

const DEFAULT_MV_FR_PROBS: [[u8; MV_FR_SIZE - 1]; 2] = [[64, 96, 64], [64, 96, 64]];

const DEFAULT_MV_CLASS0_HP_PROB: [u8; 2] = [160, 160];

const DEFAULT_MV_HP_PROB: [u8; 2] = [128, 128];

const DEFAULT_TX_PROBS: TxProbs = [
    [[0, 0, 0], [0, 0, 0]],
    [[100, 0, 0], [66, 0, 0]],
    [[20, 152, 0], [15, 101, 0]],
    [[3, 136, 37], [5, 52, 13]],
];

const DEFAULT_COEF_PROBS: CoefProbs = [
    [
        [
            [
                [
                    [195, 29, 183],
                    [84, 49, 136],
                    [8, 42, 71],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [31, 107, 169],
                    [35, 99, 159],
                    [17, 82, 140],
                    [8, 66, 114],
                    [2, 44, 76],
                    [1, 19, 32],
                ],
                [
                    [40, 132, 201],
                    [29, 114, 187],
                    [13, 91, 157],
                    [7, 75, 127],
                    [3, 58, 95],
                    [1, 28, 47],
                ],
                [
                    [69, 142, 221],
                    [42, 122, 201],
                    [15, 91, 159],
                    [6, 67, 121],
                    [1, 42, 77],
                    [1, 17, 31],
                ],
                [
                    [102, 148, 228],
                    [67, 117, 204],
                    [17, 82, 154],
                    [6, 59, 114],
                    [2, 39, 75],
                    [1, 15, 29],
                ],
                [
                    [156, 57, 233],
                    [119, 57, 212],
                    [58, 48, 163],
                    [29, 40, 124],
                    [12, 30, 81],
                    [3, 12, 31],
                ],
            ],
            [
                [
                    [191, 107, 226],
                    [124, 117, 204],
                    [25, 99, 155],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [29, 148, 210],
                    [37, 126, 194],
                    [8, 93, 157],
                    [2, 68, 118],
                    [1, 39, 69],
                    [1, 17, 33],
                ],
                [
                    [41, 151, 213],
                    [27, 123, 193],
                    [3, 82, 144],
                    [1, 58, 105],
                    [1, 32, 60],
                    [1, 13, 26],
                ],
                [
                    [59, 159, 220],
                    [23, 126, 198],
                    [4, 88, 151],
                    [1, 66, 114],
                    [1, 38, 71],
                    [1, 18, 34],
                ],
                [
                    [114, 136, 232],
                    [51, 114, 207],
                    [11, 83, 155],
                    [3, 56, 105],
                    [1, 33, 65],
                    [1, 17, 34],
                ],
                [
                    [149, 65, 234],
                    [121, 57, 215],
                    [61, 49, 166],
                    [28, 36, 114],
                    [12, 25, 76],
                    [3, 16, 42],
                ],
            ],
        ],
        [
            [
                [
                    [214, 49, 220],
                    [132, 63, 188],
                    [42, 65, 137],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [85, 137, 221],
                    [104, 131, 216],
                    [49, 111, 192],
                    [21, 87, 155],
                    [2, 49, 87],
                    [1, 16, 28],
                ],
                [
                    [89, 163, 230],
                    [90, 137, 220],
                    [29, 100, 183],
                    [10, 70, 135],
                    [2, 42, 81],
                    [1, 17, 33],
                ],
                [
                    [108, 167, 237],
                    [55, 133, 222],
                    [15, 97, 179],
                    [4, 72, 135],
                    [1, 45, 85],
                    [1, 19, 38],
                ],
                [
                    [124, 146, 240],
                    [66, 124, 224],
                    [17, 88, 175],
                    [4, 58, 122],
                    [1, 36, 75],
                    [1, 18, 37],
                ],
                [
                    [141, 79, 241],
                    [126, 70, 227],
                    [66, 58, 182],
                    [30, 44, 136],
                    [12, 34, 96],
                    [2, 20, 47],
                ],
            ],
            [
                [
                    [229, 99, 249],
                    [143, 111, 235],
                    [46, 109, 192],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [82, 158, 236],
                    [94, 146, 224],
                    [25, 117, 191],
                    [9, 87, 149],
                    [3, 56, 99],
                    [1, 33, 57],
                ],
                [
                    [83, 167, 237],
                    [68, 145, 222],
                    [10, 103, 177],
                    [2, 72, 131],
                    [1, 41, 79],
                    [1, 20, 39],
                ],
                [
                    [99, 167, 239],
                    [47, 141, 224],
                    [10, 104, 178],
                    [2, 73, 133],
                    [1, 44, 85],
                    [1, 22, 47],
                ],
                [
                    [127, 145, 243],
                    [71, 129, 228],
                    [17, 93, 177],
                    [3, 61, 124],
                    [1, 41, 84],
                    [1, 21, 52],
                ],
                [
                    [157, 78, 244],
                    [140, 72, 231],
                    [69, 58, 184],
                    [31, 44, 137],
                    [14, 38, 105],
                    [8, 23, 61],
                ],
            ],
        ],
    ],
    [
        [
            [
                [
                    [125, 34, 187],
                    [52, 41, 133],
                    [6, 31, 56],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [37, 109, 153],
                    [51, 102, 147],
                    [23, 87, 128],
                    [8, 67, 101],
                    [1, 41, 63],
                    [1, 19, 29],
                ],
                [
                    [31, 154, 185],
                    [17, 127, 175],
                    [6, 96, 145],
                    [2, 73, 114],
                    [1, 51, 82],
                    [1, 28, 45],
                ],
                [
                    [23, 163, 200],
                    [10, 131, 185],
                    [2, 93, 148],
                    [1, 67, 111],
                    [1, 41, 69],
                    [1, 14, 24],
                ],
                [
                    [29, 176, 217],
                    [12, 145, 201],
                    [3, 101, 156],
                    [1, 69, 111],
                    [1, 39, 63],
                    [1, 14, 23],
                ],
                [
                    [57, 192, 233],
                    [25, 154, 215],
                    [6, 109, 167],
                    [3, 78, 118],
                    [1, 48, 69],
                    [1, 21, 29],
                ],
            ],
            [
                [
                    [202, 105, 245],
                    [108, 106, 216],
                    [18, 90, 144],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [33, 172, 219],
                    [64, 149, 206],
                    [14, 117, 177],
                    [5, 90, 141],
                    [2, 61, 95],
                    [1, 37, 57],
                ],
                [
                    [33, 179, 220],
                    [11, 140, 198],
                    [1, 89, 148],
                    [1, 60, 104],
                    [1, 33, 57],
                    [1, 12, 21],
                ],
                [
                    [30, 181, 221],
                    [8, 141, 198],
                    [1, 87, 145],
                    [1, 58, 100],
                    [1, 31, 55],
                    [1, 12, 20],
                ],
                [
                    [32, 186, 224],
                    [7, 142, 198],
                    [1, 86, 143],
                    [1, 58, 100],
                    [1, 31, 55],
                    [1, 12, 22],
                ],
                [
                    [57, 192, 227],
                    [20, 143, 204],
                    [3, 96, 154],
                    [1, 68, 112],
                    [1, 42, 69],
                    [1, 19, 32],
                ],
            ],
        ],
        [
            [
                [
                    [212, 35, 215],
                    [113, 47, 169],
                    [29, 48, 105],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [74, 129, 203],
                    [106, 120, 203],
                    [49, 107, 178],
                    [19, 84, 144],
                    [4, 50, 84],
                    [1, 15, 25],
                ],
                [
                    [71, 172, 217],
                    [44, 141, 209],
                    [15, 102, 173],
                    [6, 76, 133],
                    [2, 51, 89],
                    [1, 24, 42],
                ],
                [
                    [64, 185, 231],
                    [31, 148, 216],
                    [8, 103, 175],
                    [3, 74, 131],
                    [1, 46, 81],
                    [1, 18, 30],
                ],
                [
                    [65, 196, 235],
                    [25, 157, 221],
                    [5, 105, 174],
                    [1, 67, 120],
                    [1, 38, 69],
                    [1, 15, 30],
                ],
                [
                    [65, 204, 238],
                    [30, 156, 224],
                    [7, 107, 177],
                    [2, 70, 124],
                    [1, 42, 73],
                    [1, 18, 34],
                ],
            ],
            [
                [
                    [225, 86, 251],
                    [144, 104, 235],
                    [42, 99, 181],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [85, 175, 239],
                    [112, 165, 229],
                    [29, 136, 200],
                    [12, 103, 162],
                    [6, 77, 123],
                    [2, 53, 84],
                ],
                [
                    [75, 183, 239],
                    [30, 155, 221],
                    [3, 106, 171],
                    [1, 74, 128],
                    [1, 44, 76],
                    [1, 17, 28],
                ],
                [
                    [73, 185, 240],
                    [27, 159, 222],
                    [2, 107, 172],
                    [1, 75, 127],
                    [1, 42, 73],
                    [1, 17, 29],
                ],
                [
                    [62, 190, 238],
                    [21, 159, 222],
                    [2, 107, 172],
                    [1, 72, 122],
                    [1, 40, 71],
                    [1, 18, 32],
                ],
                [
                    [61, 199, 240],
                    [27, 161, 226],
                    [4, 113, 180],
                    [1, 76, 129],
                    [1, 46, 80],
                    [1, 23, 41],
                ],
            ],
        ],
    ],
    [
        [
            [
                [
                    [7, 27, 153],
                    [5, 30, 95],
                    [1, 16, 30],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [50, 75, 127],
                    [57, 75, 124],
                    [27, 67, 108],
                    [10, 54, 86],
                    [1, 33, 52],
                    [1, 12, 18],
                ],
                [
                    [43, 125, 151],
                    [26, 108, 148],
                    [7, 83, 122],
                    [2, 59, 89],
                    [1, 38, 60],
                    [1, 17, 27],
                ],
                [
                    [23, 144, 163],
                    [13, 112, 154],
                    [2, 75, 117],
                    [1, 50, 81],
                    [1, 31, 51],
                    [1, 14, 23],
                ],
                [
                    [18, 162, 185],
                    [6, 123, 171],
                    [1, 78, 125],
                    [1, 51, 86],
                    [1, 31, 54],
                    [1, 14, 23],
                ],
                [
                    [15, 199, 227],
                    [3, 150, 204],
                    [1, 91, 146],
                    [1, 55, 95],
                    [1, 30, 53],
                    [1, 11, 20],
                ],
            ],
            [
                [
                    [19, 55, 240],
                    [19, 59, 196],
                    [3, 52, 105],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [41, 166, 207],
                    [104, 153, 199],
                    [31, 123, 181],
                    [14, 101, 152],
                    [5, 72, 106],
                    [1, 36, 52],
                ],
                [
                    [35, 176, 211],
                    [12, 131, 190],
                    [2, 88, 144],
                    [1, 60, 101],
                    [1, 36, 60],
                    [1, 16, 28],
                ],
                [
                    [28, 183, 213],
                    [8, 134, 191],
                    [1, 86, 142],
                    [1, 56, 96],
                    [1, 30, 53],
                    [1, 12, 20],
                ],
                [
                    [20, 190, 215],
                    [4, 135, 192],
                    [1, 84, 139],
                    [1, 53, 91],
                    [1, 28, 49],
                    [1, 11, 20],
                ],
                [
                    [13, 196, 216],
                    [2, 137, 192],
                    [1, 86, 143],
                    [1, 57, 99],
                    [1, 32, 56],
                    [1, 13, 24],
                ],
            ],
        ],
        [
            [
                [
                    [211, 29, 217],
                    [96, 47, 156],
                    [22, 43, 87],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [78, 120, 193],
                    [111, 116, 186],
                    [46, 102, 164],
                    [15, 80, 128],
                    [2, 49, 76],
                    [1, 18, 28],
                ],
                [
                    [71, 161, 203],
                    [42, 132, 192],
                    [10, 98, 150],
                    [3, 69, 109],
                    [1, 44, 70],
                    [1, 18, 29],
                ],
                [
                    [57, 186, 211],
                    [30, 140, 196],
                    [4, 93, 146],
                    [1, 62, 102],
                    [1, 38, 65],
                    [1, 16, 27],
                ],
                [
                    [47, 199, 217],
                    [14, 145, 196],
                    [1, 88, 142],
                    [1, 57, 98],
                    [1, 36, 62],
                    [1, 15, 26],
                ],
                [
                    [26, 219, 229],
                    [5, 155, 207],
                    [1, 94, 151],
                    [1, 60, 104],
                    [1, 36, 62],
                    [1, 16, 28],
                ],
            ],
            [
                [
                    [233, 29, 248],
                    [146, 47, 220],
                    [43, 52, 140],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [100, 163, 232],
                    [179, 161, 222],
                    [63, 142, 204],
                    [37, 113, 174],
                    [26, 89, 137],
                    [18, 68, 97],
                ],
                [
                    [85, 181, 230],
                    [32, 146, 209],
                    [7, 100, 164],
                    [3, 71, 121],
                    [1, 45, 77],
                    [1, 18, 30],
                ],
                [
                    [65, 187, 230],
                    [20, 148, 207],
                    [2, 97, 159],
                    [1, 68, 116],
                    [1, 40, 70],
                    [1, 14, 29],
                ],
                [
                    [40, 194, 227],
                    [8, 147, 204],
                    [1, 94, 155],
                    [1, 65, 112],
                    [1, 39, 66],
                    [1, 14, 26],
                ],
                [
                    [16, 208, 228],
                    [3, 151, 207],
                    [1, 98, 160],
                    [1, 67, 117],
                    [1, 41, 74],
                    [1, 17, 31],
                ],
            ],
        ],
    ],
    [
        [
            [
                [
                    [17, 38, 140],
                    [7, 34, 80],
                    [1, 17, 29],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [37, 75, 128],
                    [41, 76, 128],
                    [26, 66, 116],
                    [12, 52, 94],
                    [2, 32, 55],
                    [1, 10, 16],
                ],
                [
                    [50, 127, 154],
                    [37, 109, 152],
                    [16, 82, 121],
                    [5, 59, 85],
                    [1, 35, 54],
                    [1, 13, 20],
                ],
                [
                    [40, 142, 167],
                    [17, 110, 157],
                    [2, 71, 112],
                    [1, 44, 72],
                    [1, 27, 45],
                    [1, 11, 17],
                ],
                [
                    [30, 175, 188],
                    [9, 124, 169],
                    [1, 74, 116],
                    [1, 48, 78],
                    [1, 30, 49],
                    [1, 11, 18],
                ],
                [
                    [10, 222, 223],
                    [2, 150, 194],
                    [1, 83, 128],
                    [1, 48, 79],
                    [1, 27, 45],
                    [1, 11, 17],
                ],
            ],
            [
                [
                    [36, 41, 235],
                    [29, 36, 193],
                    [10, 27, 111],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [85, 165, 222],
                    [177, 162, 215],
                    [110, 135, 195],
                    [57, 113, 168],
                    [23, 83, 120],
                    [10, 49, 61],
                ],
                [
                    [85, 190, 223],
                    [36, 139, 200],
                    [5, 90, 146],
                    [1, 60, 103],
                    [1, 38, 65],
                    [1, 18, 30],
                ],
                [
                    [72, 202, 223],
                    [23, 141, 199],
                    [2, 86, 140],
                    [1, 56, 97],
                    [1, 36, 61],
                    [1, 16, 27],
                ],
                [
                    [55, 218, 225],
                    [13, 145, 200],
                    [1, 86, 141],
                    [1, 57, 99],
                    [1, 35, 61],
                    [1, 13, 22],
                ],
                [
                    [15, 235, 212],
                    [1, 132, 184],
                    [1, 84, 139],
                    [1, 57, 97],
                    [1, 34, 56],
                    [1, 14, 23],
                ],
            ],
        ],
        [
            [
                [
                    [181, 21, 201],
                    [61, 37, 123],
                    [10, 38, 71],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [47, 106, 172],
                    [95, 104, 173],
                    [42, 93, 159],
                    [18, 77, 131],
                    [4, 50, 81],
                    [1, 17, 23],
                ],
                [
                    [62, 147, 199],
                    [44, 130, 189],
                    [28, 102, 154],
                    [18, 75, 115],
                    [2, 44, 65],
                    [1, 12, 19],
                ],
                [
                    [55, 153, 210],
                    [24, 130, 194],
                    [3, 93, 146],
                    [1, 61, 97],
                    [1, 31, 50],
                    [1, 10, 16],
                ],
                [
                    [49, 186, 223],
                    [17, 148, 204],
                    [1, 96, 142],
                    [1, 53, 83],
                    [1, 26, 44],
                    [1, 11, 17],
                ],
                [
                    [13, 217, 212],
                    [2, 136, 180],
                    [1, 78, 124],
                    [1, 50, 83],
                    [1, 29, 49],
                    [1, 14, 23],
                ],
            ],
            [
                [
                    [197, 13, 247],
                    [82, 17, 222],
                    [25, 17, 162],
                    [0, 0, 0],
                    [0, 0, 0],
                    [0, 0, 0],
                ],
                [
                    [126, 186, 247],
                    [234, 191, 243],
                    [176, 177, 234],
                    [104, 158, 220],
                    [66, 128, 186],
                    [55, 90, 137],
                ],
                [
                    [111, 197, 242],
                    [46, 158, 219],
                    [9, 104, 171],
                    [2, 65, 125],
                    [1, 44, 80],
                    [1, 17, 91],
                ],
                [
                    [104, 208, 245],
                    [39, 168, 224],
                    [3, 109, 162],
                    [1, 79, 124],
                    [1, 50, 102],
                    [1, 43, 102],
                ],
                [
                    [84, 220, 246],
                    [31, 177, 231],
                    [2, 115, 180],
                    [1, 79, 134],
                    [1, 55, 77],
                    [1, 60, 79],
                ],
                [
                    [43, 243, 240],
                    [8, 180, 217],
                    [1, 115, 166],
                    [1, 84, 121],
                    [1, 51, 67],
                    [1, 16, 6],
                ],
            ],
        ],
    ],
];

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::{COUNT_SAT, MAX_UPDATE_FACTOR, merge_prob};

    #[test]
    fn merge_prob_matches_spec_integer_arithmetic() {
        assert_eq!(merge_prob(128, 0, 0, COUNT_SAT, MAX_UPDATE_FACTOR), 128);
        assert_eq!(merge_prob(128, 20, 0, COUNT_SAT, MAX_UPDATE_FACTOR), 192);
        assert_eq!(merge_prob(200, 0, 20, COUNT_SAT, MAX_UPDATE_FACTOR), 101);
        assert_eq!(merge_prob(128, 1, 1, COUNT_SAT, MAX_UPDATE_FACTOR), 128);
    }
}

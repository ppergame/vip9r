use crate::DecodeError;
use crate::PlaneShape;
use crate::boolcoder::BoolDecoder;
use crate::compressed_header::{
    CompoundReferenceSetup, CompressedHeader, InterReferenceFrame, ReferenceMode, TxMode,
};
use crate::error::ParserError;
use crate::header::{
    InterpolationFilter, LoopFilterParams, MAX_SEGMENTS, SEG_LVL_ALT_L, SEG_LVL_REF_FRAME,
    SEG_LVL_SKIP, SegmentationParams, UncompressedFrameHeader,
};
use crate::probability::{
    CLASS0_SIZE, FrameContext, MV_OFFSET_BITS, SWITCHABLE_FILTERS, SyntaxCounts,
};
use crate::tile::{TileDescriptor, TileLayout};

mod residual;
mod tables;
use residual::{DequantizedCoefficients, FrameDequant, TransformCoefficients};
use tables::*;

// Above contexts are column-indexed, not frame-MI indexed. This fixed storage
// covers the local VP9 large-scaling frontier: the largest expected frame is
// 20400px wide (2550 MI columns), rounded up to 2560 entries for 64x64
// partition contexts. Wider frames need workspace-backed context storage and
// are reported as a resource limit instead of an invalid bitstream.
const MAX_MI_COLS: usize = 2560;
const MAX_4X4_COLS: usize = MAX_MI_COLS * 2;
const MI_SIZE_PIXELS: u32 = 8;
const MI_BLOCK_64: usize = 8;
const PLANES: usize = 3;
const PARTITION_CONTEXTS: usize = 16;
const PARTITION_PROBS: usize = 3;
const INTRA_MODES: usize = 10;
const INTRA_MODE_PROBS: usize = INTRA_MODES - 1;
const BLOCK_SIZES: usize = 13;
const PARTITION_TYPES: usize = 4;
const SUBSAMPLING_X: usize = 1;
const SUBSAMPLING_Y: usize = 1;
const MAX_TX_COEFFS: usize = 1024;
const MAX_TX_WIDTH: usize = 32;
const MAX_INTRA_ABOVE: usize = MAX_TX_WIDTH * 2;
const REF_LISTS: usize = 2;
const SUB_BLOCKS: usize = 4;
const INTRA_FRAME: u8 = 0;
const NONE_FRAME: u8 = 0;
const LAST_FRAME: u8 = 1;
const GOLDEN_FRAME: u8 = 2;
const ALTREF_FRAME: u8 = 3;
const NEARESTMV: u8 = 10;
const NEARMV: u8 = 11;
const ZEROMV: u8 = 12;
const NEWMV: u8 = 13;
const SWITCHABLE_FILTER_SENTINEL: u8 = 3;
const MVREF_NEIGHBOURS: usize = 8;
const MAX_MV_REF_CANDIDATES: usize = 2;
const INTER_MODE_CONTEXTS_U8: u8 = 7;
const MV_BORDER: i32 = 128;
const BORDERINPIXELS: i32 = 160;
const INTERP_EXTEND: i32 = 4;
const INTERP_TAPS: usize = 8;
const MAX_INTER_PRED_SIZE: usize = 64;
const MAX_INTERP_SOURCE_DIM: usize = MAX_INTER_PRED_SIZE + INTERP_TAPS - 1;
const MAX_INTERP_BUFFER: usize = MAX_INTERP_SOURCE_DIM * MAX_INTERP_SOURCE_DIM;
const SUBPEL_BITS: u8 = 4;
const SUBPEL_SHIFTS: i32 = 1 << SUBPEL_BITS;
const SUBPEL_MASK: i32 = SUBPEL_SHIFTS - 1;
const REF_SCALE_SHIFT: u8 = 14;
const COMPANDED_MVREF_THRESH: i32 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileSyntaxError {
    InvalidBitstream,
    ResourceLimit,
    Unimplemented,
}

impl TileSyntaxError {
    pub(crate) const fn into_decode_error(self) -> DecodeError {
        match self {
            Self::InvalidBitstream => DecodeError::InvalidBitstream,
            Self::ResourceLimit => DecodeError::ResourceLimit,
            Self::Unimplemented => DecodeError::Unimplemented,
        }
    }
}

impl From<ParserError> for TileSyntaxError {
    fn from(error: ParserError) -> Self {
        match error {
            ParserError::InvalidBitstream
            | ParserError::UnsupportedProfile(_)
            | ParserError::UnsupportedBitDepth(_) => Self::InvalidBitstream,
        }
    }
}

#[derive(Debug)]
pub(crate) struct CurrentFrameMut<'a> {
    y: CurrentPlaneMut<'a>,
    u: CurrentPlaneMut<'a>,
    v: CurrentPlaneMut<'a>,
}

impl<'a> CurrentFrameMut<'a> {
    pub(crate) const fn new(
        y: CurrentPlaneMut<'a>,
        u: CurrentPlaneMut<'a>,
        v: CurrentPlaneMut<'a>,
    ) -> Self {
        Self { y, u, v }
    }

    fn plane_mut(&mut self, plane: usize) -> Result<&mut CurrentPlaneMut<'a>, TileSyntaxError> {
        match plane {
            0 => Ok(&mut self.y),
            1 => Ok(&mut self.u),
            2 => Ok(&mut self.v),
            _ => Err(TileSyntaxError::InvalidBitstream),
        }
    }
}

#[derive(Debug)]
pub(crate) struct CurrentPlaneMut<'a> {
    data: &'a mut [u8],
    width: usize,
    height: usize,
    stride: usize,
}

impl<'a> CurrentPlaneMut<'a> {
    pub(crate) fn new(data: &'a mut [u8], shape: PlaneShape) -> Result<Self, TileSyntaxError> {
        let width = usize::try_from(shape.width).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let height =
            usize::try_from(shape.height).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        if shape.stride < width {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        let len = shape
            .stride
            .checked_mul(height)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        if data.len() < len {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        Ok(Self {
            data,
            width,
            height,
            stride: shape.stride,
        })
    }

    fn sample_clamped(&self, x: usize, y: usize) -> Result<u8, TileSyntaxError> {
        if self.width == 0 || self.height == 0 {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let x = core::cmp::min(x, self.width - 1);
        let y = core::cmp::min(y, self.height - 1);
        let index = y
            .checked_mul(self.stride)
            .and_then(|row| row.checked_add(x))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        self.data
            .get(index)
            .copied()
            .ok_or(TileSyntaxError::InvalidBitstream)
    }

    fn set_visible(&mut self, x: usize, y: usize, value: u8) -> Result<(), TileSyntaxError> {
        if x >= self.width || y >= self.height {
            return Ok(());
        }

        let index = y
            .checked_mul(self.stride)
            .and_then(|row| row.checked_add(x))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        *self
            .data
            .get_mut(index)
            .ok_or(TileSyntaxError::InvalidBitstream)? = value;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReferenceFrames<'a> {
    frames: [Option<ReferenceFrame<'a>>; 4],
}

impl<'a> ReferenceFrames<'a> {
    pub(crate) const fn new(frames: [Option<ReferenceFrame<'a>>; 4]) -> Self {
        Self { frames }
    }

    fn get(self, ref_frame: u8) -> Result<ReferenceFrame<'a>, TileSyntaxError> {
        self.frames
            .get(usize::from(ref_frame))
            .copied()
            .flatten()
            .ok_or(TileSyntaxError::InvalidBitstream)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReferenceFrame<'a> {
    y: ReferencePlane<'a>,
    u: ReferencePlane<'a>,
    v: ReferencePlane<'a>,
}

impl<'a> ReferenceFrame<'a> {
    pub(crate) const fn new(
        y: ReferencePlane<'a>,
        u: ReferencePlane<'a>,
        v: ReferencePlane<'a>,
    ) -> Self {
        Self { y, u, v }
    }

    fn plane(self, plane: usize) -> Result<ReferencePlane<'a>, TileSyntaxError> {
        match plane {
            0 => Ok(self.y),
            1 => Ok(self.u),
            2 => Ok(self.v),
            _ => Err(TileSyntaxError::InvalidBitstream),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReferencePlane<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    stride: usize,
}

impl<'a> ReferencePlane<'a> {
    pub(crate) fn new(data: &'a [u8], shape: PlaneShape) -> Result<Self, TileSyntaxError> {
        let width = usize::try_from(shape.width).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let height =
            usize::try_from(shape.height).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        if width == 0 || height == 0 || shape.stride < width {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        let len = shape
            .stride
            .checked_mul(height)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        if data.len() < len {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        Ok(Self {
            data,
            width,
            height,
            stride: shape.stride,
        })
    }

    fn sample_clamped(&self, x: i32, y: i32) -> Result<u8, TileSyntaxError> {
        let last_x =
            i32::try_from(self.width - 1).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let last_y =
            i32::try_from(self.height - 1).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let x =
            usize::try_from(clip3(0, last_x, x)).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let y =
            usize::try_from(clip3(0, last_y, y)).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let index = y
            .checked_mul(self.stride)
            .and_then(|row| row.checked_add(x))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        self.data
            .get(index)
            .copied()
            .ok_or(TileSyntaxError::InvalidBitstream)
    }
}

pub(crate) struct TileParseBuffers<'a, 'm, 'f, 'r> {
    counts: &'a mut SyntaxCounts,
    mode_buffers: FrameModeBuffers<'m>,
    current_frame: &'a mut CurrentFrameMut<'f>,
    reference_frames: Option<ReferenceFrames<'r>>,
}

impl<'a, 'm, 'f, 'r> TileParseBuffers<'a, 'm, 'f, 'r> {
    pub(crate) const fn new(
        counts: &'a mut SyntaxCounts,
        mode_buffers: FrameModeBuffers<'m>,
        current_frame: &'a mut CurrentFrameMut<'f>,
    ) -> Self {
        Self {
            counts,
            mode_buffers,
            current_frame,
            reference_frames: None,
        }
    }

    pub(crate) const fn with_references(
        counts: &'a mut SyntaxCounts,
        mode_buffers: FrameModeBuffers<'m>,
        current_frame: &'a mut CurrentFrameMut<'f>,
        reference_frames: ReferenceFrames<'r>,
    ) -> Self {
        Self {
            counts,
            mode_buffers,
            current_frame,
            reference_frames: Some(reference_frames),
        }
    }
}

pub(crate) fn parse_intra_tiles(
    frame: &[u8],
    header: &UncompressedFrameHeader,
    compressed_header: &CompressedHeader,
    probabilities: &FrameContext,
    layout: &TileLayout,
    mut buffers: TileParseBuffers<'_, '_, '_, '_>,
) -> Result<(), TileSyntaxError> {
    if !header.frame_is_intra || header.show_existing_frame {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    let mi_cols = mi_size(header.frame_width)?;
    let mi_rows = mi_size(header.frame_height)?;
    let mut contexts = TileModeContexts::new(mi_cols)?;

    for tile in layout.as_slice() {
        let config = TileParserConfig {
            frame_is_intra: true,
            frame_width: header.frame_width,
            frame_height: header.frame_height,
            tx_mode: compressed_header.tx_mode,
            reference_mode: ReferenceMode::Single,
            compound_reference: None,
            interpolation_filter: None,
            allow_high_precision_mv: false,
            use_prev_frame_mvs: false,
            ref_frame_sign_bias: [false; 4],
            lossless: header.lossless,
            dequant: FrameDequant::from_header(header),
            frame_mis: (mi_rows, mi_cols),
            segmentation: header.segmentation,
            segment_map_reset: true,
        };
        parse_tile(
            frame,
            tile,
            config,
            buffers.mode_buffers.for_tile(),
            TileParseShared {
                probabilities,
                counts: &mut *buffers.counts,
                contexts: &mut contexts,
                current_frame: &mut *buffers.current_frame,
                reference_frames: None,
            },
        )?;
    }

    loop_filter_frame(header, buffers.current_frame, &buffers.mode_buffers)?;

    Ok(())
}

pub(crate) fn parse_inter_tiles(
    frame: &[u8],
    header: &UncompressedFrameHeader,
    compressed_header: &CompressedHeader,
    probabilities: &FrameContext,
    layout: &TileLayout,
    mut buffers: TileParseBuffers<'_, '_, '_, '_>,
) -> Result<(), TileSyntaxError> {
    if header.frame_is_intra || header.show_existing_frame {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    if header.profile != 0 || header.bit_depth != 8 {
        return Err(TileSyntaxError::Unimplemented);
    }

    let mi_cols = mi_size(header.frame_width)?;
    let mi_rows = mi_size(header.frame_height)?;
    let mut contexts = TileModeContexts::new(mi_cols)?;
    let reference_frames = buffers
        .reference_frames
        .ok_or(TileSyntaxError::InvalidBitstream)?;

    for tile in layout.as_slice() {
        let config = TileParserConfig {
            frame_is_intra: false,
            frame_width: header.frame_width,
            frame_height: header.frame_height,
            tx_mode: compressed_header.tx_mode,
            reference_mode: compressed_header.reference_mode,
            compound_reference: compressed_header.compound_reference,
            interpolation_filter: header.interpolation_filter,
            allow_high_precision_mv: header.allow_high_precision_mv,
            use_prev_frame_mvs: buffers.mode_buffers.use_prev_frame_mvs,
            ref_frame_sign_bias: header.ref_frame_sign_bias,
            lossless: header.lossless,
            dequant: FrameDequant::from_header(header),
            frame_mis: (mi_rows, mi_cols),
            segmentation: header.segmentation,
            segment_map_reset: header.error_resilient_mode,
        };
        parse_tile(
            frame,
            tile,
            config,
            buffers.mode_buffers.for_tile(),
            TileParseShared {
                probabilities,
                counts: &mut *buffers.counts,
                contexts: &mut contexts,
                current_frame: &mut *buffers.current_frame,
                reference_frames: Some(reference_frames),
            },
        )?;
    }

    loop_filter_frame(header, buffers.current_frame, &buffers.mode_buffers)?;

    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TileParserConfig {
    frame_is_intra: bool,
    frame_width: u32,
    frame_height: u32,
    tx_mode: TxMode,
    reference_mode: ReferenceMode,
    compound_reference: Option<CompoundReferenceSetup>,
    interpolation_filter: Option<InterpolationFilter>,
    allow_high_precision_mv: bool,
    use_prev_frame_mvs: bool,
    ref_frame_sign_bias: [bool; 4],
    lossless: bool,
    dequant: FrameDequant,
    frame_mis: (usize, usize),
    segmentation: SegmentationParams,
    segment_map_reset: bool,
}

struct TileParseShared<'a, 'f, 'r> {
    probabilities: &'a FrameContext,
    counts: &'a mut SyntaxCounts,
    contexts: &'a mut TileModeContexts,
    current_frame: &'a mut CurrentFrameMut<'f>,
    reference_frames: Option<ReferenceFrames<'r>>,
}

fn parse_tile(
    frame: &[u8],
    tile: &TileDescriptor,
    config: TileParserConfig,
    mode_buffers: FrameModeBuffers<'_>,
    shared: TileParseShared<'_, '_, '_>,
) -> Result<(), TileSyntaxError> {
    let TileParseShared {
        probabilities,
        counts,
        contexts,
        current_frame,
        reference_frames,
    } = shared;
    let (mi_rows, mi_cols) = config.frame_mis;
    let payload = frame
        .get(tile.payload_start..tile.payload_end)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let decoder = BoolDecoder::new(payload)?;
    let tile_row_start =
        usize::try_from(tile.mi_row_start).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let tile_row_end =
        usize::try_from(tile.mi_row_end).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let tile_col_start =
        usize::try_from(tile.mi_col_start).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let tile_col_end =
        usize::try_from(tile.mi_col_end).map_err(|_| TileSyntaxError::InvalidBitstream)?;

    if tile_row_end > mi_rows
        || tile_col_end > mi_cols
        || tile_row_start > tile_row_end
        || tile_col_start > tile_col_end
    {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    let mut parser = TileParser {
        decoder,
        probabilities,
        counts,
        contexts,
        tx_mode: config.tx_mode,
        frame_is_intra: config.frame_is_intra,
        frame_width: config.frame_width,
        frame_height: config.frame_height,
        reference_mode: config.reference_mode,
        compound_reference: config.compound_reference,
        interpolation_filter: config.interpolation_filter,
        allow_high_precision_mv: config.allow_high_precision_mv,
        use_prev_frame_mvs: config.use_prev_frame_mvs,
        ref_frame_sign_bias: config.ref_frame_sign_bias,
        lossless: config.lossless,
        dequant: config.dequant,
        segmentation: config.segmentation,
        segment_map_reset: config.segment_map_reset,
        mi_rows,
        mi_cols,
        tile_col_start,
        tile_col_end,
        left_row_base: tile_row_start,
        prev_frame_modes: mode_buffers.prev_frame_modes,
        current_frame_modes: mode_buffers.current_frame_modes,
        reference_frames,
    };

    let mut row = tile_row_start;
    while row < tile_row_end {
        parser.left_row_base = row;
        parser.contexts.clear_left_context();

        let mut col = tile_col_start;
        while col < tile_col_end {
            parser.decode_partition(row, col, BlockSize::Block64x64, current_frame)?;
            col = col
                .checked_add(MI_BLOCK_64)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
        }

        row = row
            .checked_add(MI_BLOCK_64)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
    }

    parser.decoder.finish()?;
    Ok(())
}

struct TileParser<'a, 'b, 'r> {
    decoder: BoolDecoder<'a>,
    probabilities: &'b FrameContext,
    counts: &'b mut SyntaxCounts,
    contexts: &'b mut TileModeContexts,
    tx_mode: TxMode,
    frame_is_intra: bool,
    frame_width: u32,
    frame_height: u32,
    reference_mode: ReferenceMode,
    compound_reference: Option<CompoundReferenceSetup>,
    interpolation_filter: Option<InterpolationFilter>,
    allow_high_precision_mv: bool,
    use_prev_frame_mvs: bool,
    ref_frame_sign_bias: [bool; 4],
    lossless: bool,
    dequant: FrameDequant,
    segmentation: SegmentationParams,
    segment_map_reset: bool,
    mi_rows: usize,
    mi_cols: usize,
    tile_col_start: usize,
    tile_col_end: usize,
    left_row_base: usize,
    prev_frame_modes: Option<ModeInfoView<'b>>,
    current_frame_modes: Option<ModeInfoViewMut<'b>>,
    reference_frames: Option<ReferenceFrames<'r>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BlockModeContext {
    row: usize,
    col: usize,
    block_size: BlockSize,
    tx_size: TxSize,
    skip: bool,
    segment_id: u8,
    avail_u: bool,
    avail_l: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Sub8x8MvContext {
    row: usize,
    col: usize,
    block_size: BlockSize,
    ref_frame: u8,
    block: usize,
    ref_list: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntraPredictionContext {
    plane: usize,
    start_x: usize,
    start_y: usize,
    have_left: bool,
    have_above: bool,
    not_on_right: bool,
    tx_size: TxSize,
    block_idx: usize,
    mi_size: BlockSize,
    block: DecodedBlockInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntraPredictionRequest {
    mode: IntraMode,
    have_left: bool,
    have_above: bool,
    size: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InterPredictionContext {
    plane: usize,
    mi_row: usize,
    mi_col: usize,
    start_x: usize,
    start_y: usize,
    width: usize,
    height: usize,
    block_idx: usize,
    mi_size: BlockSize,
    block: DecodedBlockInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScaledMotion {
    start_x: i32,
    start_y: i32,
    step_x: i32,
    step_y: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IntraPredictionEdges {
    above_left: u8,
    above_row: [u8; MAX_INTRA_ABOVE],
    left_col: [u8; MAX_TX_WIDTH],
}

impl TileParser<'_, '_, '_> {
    fn decode_partition(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        current_frame: &mut CurrentFrameMut<'_>,
    ) -> Result<(), TileSyntaxError> {
        if row >= self.mi_rows || col >= self.mi_cols {
            return Ok(());
        }

        let num_8x8 = usize::from(block_size.num_8x8_wide());
        let half_block_8x8 = num_8x8 >> 1;
        let has_rows = row
            .checked_add(half_block_8x8)
            .ok_or(TileSyntaxError::InvalidBitstream)?
            < self.mi_rows;
        let has_cols = col
            .checked_add(half_block_8x8)
            .ok_or(TileSyntaxError::InvalidBitstream)?
            < self.mi_cols;
        let partition = self.read_partition(row, col, block_size, has_rows, has_cols)?;
        let subsize = block_size.subsize(partition)?;

        if !subsize.is_at_least_8x8() || partition == PartitionType::None {
            self.decode_block(row, col, subsize, current_frame)?;
        } else if partition == PartitionType::Horz {
            self.decode_block(row, col, subsize, current_frame)?;
            if has_rows {
                self.decode_block(row + half_block_8x8, col, subsize, current_frame)?;
            }
        } else if partition == PartitionType::Vert {
            self.decode_block(row, col, subsize, current_frame)?;
            if has_cols {
                self.decode_block(row, col + half_block_8x8, subsize, current_frame)?;
            }
        } else {
            self.decode_partition(row, col, subsize, current_frame)?;
            self.decode_partition(row, col + half_block_8x8, subsize, current_frame)?;
            self.decode_partition(row + half_block_8x8, col, subsize, current_frame)?;
            self.decode_partition(
                row + half_block_8x8,
                col + half_block_8x8,
                subsize,
                current_frame,
            )?;
        }

        if block_size == BlockSize::Block8x8 || partition != PartitionType::Split {
            self.contexts.update_partition_context(
                self.left_row_base,
                row,
                col,
                block_size,
                subsize,
            )?;
        }

        Ok(())
    }

    fn read_partition(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        has_rows: bool,
        has_cols: bool,
    ) -> Result<PartitionType, TileSyntaxError> {
        let ctx = self
            .contexts
            .partition_context(self.left_row_base, row, col, block_size)?;
        let probs = if self.frame_is_intra {
            &KF_PARTITION_PROBS[ctx]
        } else {
            &self.probabilities.partition_probs[ctx]
        };
        let raw = if has_rows && has_cols {
            self.decoder.read_tree(&PARTITION_TREE, probs)?
        } else if has_cols {
            self.decoder.read_tree(&COLS_PARTITION_TREE, &probs[1..2])?
        } else if has_rows {
            self.decoder.read_tree(&ROWS_PARTITION_TREE, &probs[2..3])?
        } else {
            PartitionType::Split as u8
        };

        let partition = PartitionType::from_raw(raw).ok_or(TileSyntaxError::InvalidBitstream)?;
        increment_count(&mut self.counts.counts_partition[ctx][partition.index()]);
        Ok(partition)
    }

    fn decode_block(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        current_frame: &mut CurrentFrameMut<'_>,
    ) -> Result<(), TileSyntaxError> {
        let avail_u = row > 0;
        let avail_l = col > self.tile_col_start;
        let mut block = if self.frame_is_intra {
            self.intra_frame_mode_info(row, col, block_size, avail_u, avail_l)?
        } else {
            self.inter_frame_mode_info(row, col, block_size, avail_u, avail_l)?
        };
        let has_nonzero_coefficients =
            self.decode_residual(row, col, block_size, block, current_frame)?;
        if block.is_inter && block_size.is_at_least_8x8() && !has_nonzero_coefficients {
            block.skip = true;
        }
        self.contexts
            .update_mode_context(self.left_row_base, row, col, block_size, block)?;
        self.update_current_frame_modes(row, col, block_size, block)?;
        Ok(())
    }

    fn intra_frame_mode_info(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        avail_u: bool,
        avail_l: bool,
    ) -> Result<DecodedBlockInfo, TileSyntaxError> {
        let segment_id = self.intra_segment_id()?;
        let skip = self.read_skip(row, col, avail_u, avail_l, segment_id)?;
        let tx_size = self.read_tx_size(row, col, block_size, true, avail_u, avail_l)?;
        let (y_mode, sub_modes) = self.read_intra_modes(row, col, block_size, avail_u, avail_l)?;
        let uv_mode = self.read_default_uv_mode(y_mode)?;

        Ok(DecodedBlockInfo {
            skip,
            tx_size,
            y_mode: y_mode.raw(),
            uv_mode,
            sub_modes,
            segment_id,
            is_inter: false,
            ref_frames: [INTRA_FRAME, NONE_FRAME],
            interp_filter: SWITCHABLE_FILTER_SENTINEL,
            block_mvs: [[MotionVector::ZERO; SUB_BLOCKS]; REF_LISTS],
        })
    }

    fn inter_frame_mode_info(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        avail_u: bool,
        avail_l: bool,
    ) -> Result<DecodedBlockInfo, TileSyntaxError> {
        let left = if avail_l {
            self.contexts.left_mode(self.left_row_base, row)?
        } else {
            NeighborModeInfo::DEFAULT
        };
        let above = if avail_u {
            self.contexts.above_mode(col)?
        } else {
            NeighborModeInfo::DEFAULT
        };
        let left_intra = left.ref_frames[0] == INTRA_FRAME;
        let above_intra = above.ref_frames[0] == INTRA_FRAME;
        let segment_id = self.inter_segment_id(row, col, block_size)?;
        let skip = self.read_skip(row, col, avail_u, avail_l, segment_id)?;
        let is_inter = self.read_is_inter(avail_u, avail_l, left_intra, above_intra, segment_id)?;
        let tx_size =
            self.read_tx_size(row, col, block_size, !skip || !is_inter, avail_u, avail_l)?;
        let block_context = BlockModeContext {
            row,
            col,
            block_size,
            tx_size,
            skip,
            segment_id,
            avail_u,
            avail_l,
        };

        if is_inter {
            self.inter_block_mode_info(block_context, left, above)
        } else {
            self.intra_block_mode_info(block_context)
        }
    }

    fn intra_segment_id(&mut self) -> Result<u8, TileSyntaxError> {
        if self.segmentation.enabled && self.segmentation.update_map {
            self.read_segment_id()
        } else {
            Ok(0)
        }
    }

    fn inter_segment_id(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
    ) -> Result<u8, TileSyntaxError> {
        if !self.segmentation.enabled {
            return Ok(0);
        }

        if self.segmentation.update_map {
            if self.segmentation.temporal_update {
                let predicted_segment_id = self.predicted_segment_id(row, col, block_size)?;
                let ctx = self
                    .contexts
                    .seg_pred_context(self.left_row_base, row, col)?;
                let seg_id_predicted = self.decoder.read_bool(self.segmentation.pred_probs[ctx])?;
                self.contexts.update_seg_pred_context(
                    self.left_row_base,
                    row,
                    col,
                    block_size,
                    seg_id_predicted,
                )?;
                if seg_id_predicted {
                    Ok(predicted_segment_id)
                } else {
                    self.read_segment_id()
                }
            } else {
                self.read_segment_id()
            }
        } else {
            self.predicted_segment_id(row, col, block_size)
        }
    }

    fn read_segment_id(&mut self) -> Result<u8, TileSyntaxError> {
        let segment_id = self
            .decoder
            .read_tree(&SEGMENT_TREE, &self.segmentation.tree_probs)?;
        if segment_id < 8 {
            Ok(segment_id)
        } else {
            Err(TileSyntaxError::InvalidBitstream)
        }
    }

    fn predicted_segment_id(
        &self,
        row: usize,
        col: usize,
        block_size: BlockSize,
    ) -> Result<u8, TileSyntaxError> {
        if self.segment_map_reset {
            return Ok(0);
        }

        let Some(prev_frame_modes) = self.prev_frame_modes else {
            return Err(TileSyntaxError::Unimplemented);
        };

        let width = usize::from(block_size.num_8x8_wide());
        let height = usize::from(block_size.num_8x8_high());
        let xmis = core::cmp::min(
            self.mi_cols
                .checked_sub(col)
                .ok_or(TileSyntaxError::InvalidBitstream)?,
            width,
        );
        let ymis = core::cmp::min(
            self.mi_rows
                .checked_sub(row)
                .ok_or(TileSyntaxError::InvalidBitstream)?,
            height,
        );
        let mut segment_id = 7u8;
        for y in 0..ymis {
            for x in 0..xmis {
                let mode_row = row
                    .checked_add(y)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                let mode_col = col
                    .checked_add(x)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                let index = mode_row
                    .checked_mul(self.mi_cols)
                    .and_then(|value| value.checked_add(mode_col))
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                let info = prev_frame_modes.get(index)?;
                if !info.valid {
                    return Err(TileSyntaxError::InvalidBitstream);
                }
                segment_id = core::cmp::min(segment_id, info.segment_map_id);
            }
        }
        Ok(segment_id)
    }

    fn read_is_inter(
        &mut self,
        avail_u: bool,
        avail_l: bool,
        left_intra: bool,
        above_intra: bool,
        segment_id: u8,
    ) -> Result<bool, TileSyntaxError> {
        if self.seg_feature_active(segment_id, SEG_LVL_REF_FRAME) {
            let ref_frame = self.seg_feature_data(segment_id, SEG_LVL_REF_FRAME)?;
            return Ok(ref_frame != i16::from(INTRA_FRAME));
        }

        let ctx = if avail_u && avail_l {
            if left_intra && above_intra {
                3
            } else {
                usize::from(left_intra || above_intra)
            }
        } else if avail_u || avail_l {
            2 * usize::from(if avail_u { above_intra } else { left_intra })
        } else {
            0
        };
        let is_inter = self
            .decoder
            .read_bool(self.probabilities.is_inter_prob[ctx])?;
        increment_count(&mut self.counts.counts_is_inter[ctx][bool_index(is_inter)]);
        Ok(is_inter)
    }

    fn update_current_frame_modes(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        block: DecodedBlockInfo,
    ) -> Result<(), TileSyntaxError> {
        let Some(current_frame_modes) = self.current_frame_modes.as_mut() else {
            return Ok(());
        };

        let mut mvs = [MotionVector::ZERO; REF_LISTS];
        for (ref_list, mv) in mvs.iter_mut().enumerate() {
            *mv = block.block_mvs[ref_list][3];
        }
        let mut stored = StoredModeInfo {
            valid: true,
            skip: block.skip,
            tx_size: block.tx_size,
            segment_id: block.segment_id,
            segment_map_id: block.segment_id,
            mi_size: block_size,
            y_mode: block.y_mode,
            ref_frames: block.ref_frames,
            mvs,
            sub_mvs: block.block_mvs,
        };
        let segment_map_updates = self.segmentation.enabled && self.segmentation.update_map;
        let preserve_segment_map = !self.segment_map_reset && !segment_map_updates;
        let prev_frame_modes = self.prev_frame_modes;
        let width = usize::from(block_size.num_8x8_wide());
        let height = usize::from(block_size.num_8x8_high());
        for y in 0..height {
            let mode_row = row
                .checked_add(y)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            if mode_row >= self.mi_rows {
                continue;
            }
            for x in 0..width {
                let mode_col = col
                    .checked_add(x)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                if mode_col >= self.mi_cols {
                    continue;
                }
                let index = mode_row
                    .checked_mul(self.mi_cols)
                    .and_then(|value| value.checked_add(mode_col))
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                stored.segment_map_id = if preserve_segment_map {
                    match prev_frame_modes {
                        Some(prev_frame_modes) => {
                            let previous = prev_frame_modes.get(index)?;
                            if !previous.valid {
                                return Err(TileSyntaxError::InvalidBitstream);
                            }
                            previous.segment_map_id
                        }
                        None => 0,
                    }
                } else if self.segment_map_reset && !segment_map_updates {
                    0
                } else {
                    block.segment_id
                };
                current_frame_modes.set(index, stored)?;
            }
        }
        Ok(())
    }

    fn intra_block_mode_info(
        &mut self,
        context: BlockModeContext,
    ) -> Result<DecodedBlockInfo, TileSyntaxError> {
        let (y_mode, sub_modes) = self.read_intra_block_modes(
            context.row,
            context.col,
            context.block_size,
            context.avail_u,
            context.avail_l,
        )?;
        let uv_mode = self.read_uv_mode(y_mode)?;

        Ok(DecodedBlockInfo {
            skip: context.skip,
            tx_size: context.tx_size,
            y_mode: y_mode.raw(),
            uv_mode,
            sub_modes,
            segment_id: context.segment_id,
            is_inter: false,
            ref_frames: [INTRA_FRAME, NONE_FRAME],
            interp_filter: SWITCHABLE_FILTER_SENTINEL,
            block_mvs: [[MotionVector::ZERO; SUB_BLOCKS]; REF_LISTS],
        })
    }

    fn inter_block_mode_info(
        &mut self,
        context: BlockModeContext,
        left: NeighborModeInfo,
        above: NeighborModeInfo,
    ) -> Result<DecodedBlockInfo, TileSyntaxError> {
        let ref_frames = self.read_ref_frames(
            left,
            above,
            context.col > self.tile_col_start,
            context.row > 0,
            context.segment_id,
        )?;
        let is_compound = ref_frames[1] > INTRA_FRAME;
        let ref_count = 1 + usize::from(is_compound);
        let mut mv_state = [MvRefState::DEFAULT; REF_LISTS];
        for ref_list in 0..ref_count {
            let state = self.find_mv_refs(
                context.row,
                context.col,
                context.block_size,
                ref_frames[ref_list],
                -1,
            )?;
            mv_state[ref_list] =
                self.find_best_ref_mvs(context.row, context.col, context.block_size, state);
        }

        let mut y_mode = ZEROMV;
        let mut block_mvs = [[MotionVector::ZERO; SUB_BLOCKS]; REF_LISTS];
        let seg_skip = self.seg_feature_active(context.segment_id, SEG_LVL_SKIP);
        if seg_skip {
            if context.block_size < BlockSize::Block8x8 {
                return Err(TileSyntaxError::InvalidBitstream);
            }
        } else if context.block_size.is_at_least_8x8() {
            let inter_mode = self.read_inter_mode(mv_state[0].mode_context)?;
            y_mode = inter_mode.y_mode();
        }

        let interp_filter = self.read_block_interp_filter(context.row, context.col, left, above)?;

        if context.block_size < BlockSize::Block8x8 {
            let num4x4w = usize::from(context.block_size.num_4x4_wide());
            let num4x4h = usize::from(context.block_size.num_4x4_high());
            let mut idy = 0usize;
            while idy < 2 {
                let mut idx = 0usize;
                while idx < 2 {
                    let block = idy * 2 + idx;
                    let inter_mode = self.read_inter_mode(mv_state[0].mode_context)?;
                    y_mode = inter_mode.y_mode();
                    if matches!(inter_mode, InterMode::Nearest | InterMode::Near) {
                        for (ref_list, state) in mv_state.iter_mut().enumerate().take(ref_count) {
                            *state = self.append_sub8x8_mvs(
                                Sub8x8MvContext {
                                    row: context.row,
                                    col: context.col,
                                    block_size: context.block_size,
                                    ref_frame: ref_frames[ref_list],
                                    block,
                                    ref_list,
                                },
                                &block_mvs,
                                *state,
                            )?;
                        }
                    }
                    let assigned = self.assign_mv(inter_mode, is_compound, &mv_state)?;
                    for y2 in 0..num4x4h {
                        for x2 in 0..num4x4w {
                            let dst = (idy + y2) * 2 + idx + x2;
                            for ref_list in 0..ref_count {
                                block_mvs[ref_list][dst] = assigned[ref_list];
                            }
                        }
                    }

                    idx = idx
                        .checked_add(num4x4w)
                        .ok_or(TileSyntaxError::InvalidBitstream)?;
                }
                idy = idy
                    .checked_add(num4x4h)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
            }
        } else {
            let inter_mode = InterMode::from_y_mode(y_mode)?;
            let assigned = self.assign_mv(inter_mode, is_compound, &mv_state)?;
            for ref_list in 0..ref_count {
                for mv in block_mvs[ref_list].iter_mut().take(SUB_BLOCKS) {
                    *mv = assigned[ref_list];
                }
            }
        }

        Ok(DecodedBlockInfo {
            skip: context.skip,
            tx_size: context.tx_size,
            y_mode,
            uv_mode: IntraMode::Dc,
            sub_modes: [IntraMode::Dc; SUB_BLOCKS],
            segment_id: context.segment_id,
            is_inter: true,
            ref_frames,
            interp_filter,
            block_mvs,
        })
    }

    fn read_skip(
        &mut self,
        row: usize,
        col: usize,
        avail_u: bool,
        avail_l: bool,
        segment_id: u8,
    ) -> Result<bool, TileSyntaxError> {
        if self.seg_feature_active(segment_id, SEG_LVL_SKIP) {
            return Ok(true);
        }

        let ctx = self
            .contexts
            .skip_context(self.left_row_base, row, col, avail_u, avail_l)?;
        let skip = self.decoder.read_bool(self.probabilities.skip_prob[ctx])?;
        increment_count(&mut self.counts.counts_skip[ctx][bool_index(skip)]);
        Ok(skip)
    }

    fn read_tx_size(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        allow_select: bool,
        avail_u: bool,
        avail_l: bool,
    ) -> Result<TxSize, TileSyntaxError> {
        let max_tx_size = block_size.max_tx_size();
        if allow_select && self.tx_mode == TxMode::Select && block_size.is_at_least_8x8() {
            let ctx = self.contexts.tx_size_context(
                self.left_row_base,
                row,
                col,
                max_tx_size,
                avail_u,
                avail_l,
            )?;
            let probs = &self.probabilities.tx_probs[max_tx_size.index()][ctx];
            let raw = match max_tx_size {
                TxSize::Tx32x32 => self.decoder.read_tree(&TX_SIZE_32_TREE, probs)?,
                TxSize::Tx16x16 => self.decoder.read_tree(&TX_SIZE_16_TREE, &probs[..2])?,
                TxSize::Tx8x8 => self.decoder.read_tree(&TX_SIZE_8_TREE, &probs[..1])?,
                TxSize::Tx4x4 => 0,
            };
            let tx_size = TxSize::from_raw(raw).ok_or(TileSyntaxError::InvalidBitstream)?;
            increment_count(
                &mut self.counts.counts_tx_size[max_tx_size.index()][ctx][tx_size.index()],
            );
            Ok(tx_size)
        } else {
            Ok(TxSize::from_index(core::cmp::min(
                max_tx_size.index(),
                self.tx_mode.biggest_tx_size(),
            )))
        }
    }

    fn read_intra_modes(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        avail_u: bool,
        avail_l: bool,
    ) -> Result<(IntraMode, [IntraMode; 4]), TileSyntaxError> {
        if block_size.is_at_least_8x8() {
            let above_mode = if avail_u {
                self.contexts.above_mode(col)?.sub_modes[2]
            } else {
                IntraMode::Dc
            };
            let left_mode = if avail_l {
                self.contexts.left_mode(self.left_row_base, row)?.sub_modes[1]
            } else {
                IntraMode::Dc
            };
            let y_mode = self.read_default_intra_mode(above_mode, left_mode)?;
            return Ok((y_mode, [y_mode; 4]));
        }

        let num4x4w = usize::from(block_size.num_4x4_wide());
        let num4x4h = usize::from(block_size.num_4x4_high());
        let mut sub_modes = [IntraMode::Dc; 4];
        let mut y_mode = IntraMode::Dc;
        let mut idy = 0usize;
        while idy < 2 {
            let mut idx = 0usize;
            while idx < 2 {
                let above_mode = if idy != 0 {
                    sub_modes[idx]
                } else if avail_u {
                    self.contexts.above_mode(col)?.sub_modes[2 + idx]
                } else {
                    IntraMode::Dc
                };
                let left_mode = if idx != 0 {
                    sub_modes[idy * 2]
                } else if avail_l {
                    self.contexts.left_mode(self.left_row_base, row)?.sub_modes[1 + idy * 2]
                } else {
                    IntraMode::Dc
                };
                y_mode = self.read_default_intra_mode(above_mode, left_mode)?;

                for y2 in 0..num4x4h {
                    for x2 in 0..num4x4w {
                        sub_modes[(idy + y2) * 2 + idx + x2] = y_mode;
                    }
                }

                idx = idx
                    .checked_add(num4x4w)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
            }
            idy = idy
                .checked_add(num4x4h)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
        }

        Ok((y_mode, sub_modes))
    }

    fn read_intra_block_modes(
        &mut self,
        _row: usize,
        _col: usize,
        block_size: BlockSize,
        _avail_u: bool,
        _avail_l: bool,
    ) -> Result<(IntraMode, [IntraMode; 4]), TileSyntaxError> {
        if block_size.is_at_least_8x8() {
            let y_mode = self.read_inter_intra_mode(size_group(block_size))?;
            return Ok((y_mode, [y_mode; 4]));
        }

        let num4x4w = usize::from(block_size.num_4x4_wide());
        let num4x4h = usize::from(block_size.num_4x4_high());
        let mut sub_modes = [IntraMode::Dc; 4];
        let mut y_mode = IntraMode::Dc;
        let mut idy = 0usize;
        while idy < 2 {
            let mut idx = 0usize;
            while idx < 2 {
                y_mode = self.read_inter_intra_mode(0)?;

                for y2 in 0..num4x4h {
                    for x2 in 0..num4x4w {
                        sub_modes[(idy + y2) * 2 + idx + x2] = y_mode;
                    }
                }

                idx = idx
                    .checked_add(num4x4w)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
            }
            idy = idy
                .checked_add(num4x4h)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
        }

        Ok((y_mode, sub_modes))
    }

    fn read_default_intra_mode(
        &mut self,
        above_mode: IntraMode,
        left_mode: IntraMode,
    ) -> Result<IntraMode, TileSyntaxError> {
        let probs = &KF_Y_MODE_PROBS[above_mode.index()][left_mode.index()];
        let raw = self.decoder.read_tree(&INTRA_MODE_TREE, probs)?;
        IntraMode::from_raw(raw).ok_or(TileSyntaxError::InvalidBitstream)
    }

    fn read_inter_intra_mode(&mut self, ctx: usize) -> Result<IntraMode, TileSyntaxError> {
        let probs = self
            .probabilities
            .y_mode_probs
            .get(ctx)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let raw = self.decoder.read_tree(&INTRA_MODE_TREE, probs)?;
        let mode = IntraMode::from_raw(raw).ok_or(TileSyntaxError::InvalidBitstream)?;
        increment_count(&mut self.counts.counts_intra_mode[ctx][mode.index()]);
        Ok(mode)
    }

    fn read_default_uv_mode(&mut self, y_mode: IntraMode) -> Result<IntraMode, TileSyntaxError> {
        let probs = &KF_UV_MODE_PROBS[y_mode.index()];
        let raw = self.decoder.read_tree(&INTRA_MODE_TREE, probs)?;
        IntraMode::from_raw(raw).ok_or(TileSyntaxError::InvalidBitstream)
    }

    fn read_uv_mode(&mut self, y_mode: IntraMode) -> Result<IntraMode, TileSyntaxError> {
        let probs = &self.probabilities.uv_mode_probs[y_mode.index()];
        let raw = self.decoder.read_tree(&INTRA_MODE_TREE, probs)?;
        let mode = IntraMode::from_raw(raw).ok_or(TileSyntaxError::InvalidBitstream)?;
        increment_count(&mut self.counts.counts_uv_mode[y_mode.index()][mode.index()]);
        Ok(mode)
    }

    fn read_ref_frames(
        &mut self,
        left: NeighborModeInfo,
        above: NeighborModeInfo,
        avail_l: bool,
        avail_u: bool,
        segment_id: u8,
    ) -> Result<[u8; REF_LISTS], TileSyntaxError> {
        if self.seg_feature_active(segment_id, SEG_LVL_REF_FRAME) {
            let ref_frame = u8::try_from(self.seg_feature_data(segment_id, SEG_LVL_REF_FRAME)?)
                .map_err(|_| TileSyntaxError::InvalidBitstream)?;
            if ref_frame > ALTREF_FRAME {
                return Err(TileSyntaxError::InvalidBitstream);
            }
            return Ok([ref_frame, NONE_FRAME]);
        }

        if self.reference_mode == ReferenceMode::Select {
            let ctx = self.comp_mode_context(left, above, avail_l, avail_u)?;
            let compound = self
                .decoder
                .read_bool(self.probabilities.comp_mode_prob[ctx])?;
            increment_count(&mut self.counts.counts_comp_mode[ctx][bool_index(compound)]);
            if compound {
                return self.read_compound_ref_frames(left, above, avail_l, avail_u);
            }
        } else if self.reference_mode == ReferenceMode::Compound {
            return self.read_compound_ref_frames(left, above, avail_l, avail_u);
        }

        let ctx = single_ref_p1_context(left, above, avail_l, avail_u);
        let single_ref_p1 = self
            .decoder
            .read_bool(self.probabilities.single_ref_prob[ctx][0])?;
        increment_count(&mut self.counts.counts_single_ref[ctx][0][bool_index(single_ref_p1)]);
        let ref_frame = if single_ref_p1 {
            let ctx = single_ref_p2_context(left, above, avail_l, avail_u);
            let single_ref_p2 = self
                .decoder
                .read_bool(self.probabilities.single_ref_prob[ctx][1])?;
            increment_count(&mut self.counts.counts_single_ref[ctx][1][bool_index(single_ref_p2)]);
            if single_ref_p2 {
                ALTREF_FRAME
            } else {
                GOLDEN_FRAME
            }
        } else {
            LAST_FRAME
        };
        Ok([ref_frame, NONE_FRAME])
    }

    fn read_compound_ref_frames(
        &mut self,
        left: NeighborModeInfo,
        above: NeighborModeInfo,
        avail_l: bool,
        avail_u: bool,
    ) -> Result<[u8; REF_LISTS], TileSyntaxError> {
        let compound = self
            .compound_reference
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let ctx = comp_ref_context(
            compound,
            left,
            above,
            avail_l,
            avail_u,
            self.ref_sign_biases(),
        )?;
        let comp_ref = usize::from(
            self.decoder
                .read_bool(self.probabilities.comp_ref_prob[ctx])?,
        );
        increment_count(&mut self.counts.counts_comp_ref[ctx][comp_ref]);
        let fixed_ref = reference_frame_raw(compound.comp_fixed_ref);
        let variable_ref = reference_frame_raw(compound.comp_var_ref[comp_ref]);
        let fixed_index = usize::from(self.sign_bias(fixed_ref)?);
        let mut ref_frames = [NONE_FRAME; REF_LISTS];
        ref_frames[fixed_index] = fixed_ref;
        ref_frames[1 - fixed_index] = variable_ref;
        Ok(ref_frames)
    }

    fn comp_mode_context(
        &self,
        left: NeighborModeInfo,
        above: NeighborModeInfo,
        avail_l: bool,
        avail_u: bool,
    ) -> Result<usize, TileSyntaxError> {
        let compound = self
            .compound_reference
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let fixed_ref = reference_frame_raw(compound.comp_fixed_ref);
        let left_intra = left.ref_frames[0] == INTRA_FRAME;
        let above_intra = above.ref_frames[0] == INTRA_FRAME;
        let left_single = left.ref_frames[1] == NONE_FRAME;
        let above_single = above.ref_frames[1] == NONE_FRAME;
        let ctx = if avail_u && avail_l {
            if above_single && left_single {
                usize::from((above.ref_frames[0] == fixed_ref) ^ (left.ref_frames[0] == fixed_ref))
            } else if above_single {
                2 + usize::from(above.ref_frames[0] == fixed_ref || above_intra)
            } else if left_single {
                2 + usize::from(left.ref_frames[0] == fixed_ref || left_intra)
            } else {
                4
            }
        } else if avail_u {
            if above_single {
                usize::from(above.ref_frames[0] == fixed_ref)
            } else {
                3
            }
        } else if avail_l {
            if left_single {
                usize::from(left.ref_frames[0] == fixed_ref)
            } else {
                3
            }
        } else {
            1
        };
        Ok(ctx)
    }

    fn read_inter_mode(&mut self, ctx: usize) -> Result<InterMode, TileSyntaxError> {
        let probs = self
            .probabilities
            .inter_mode_probs
            .get(ctx)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let raw = self.decoder.read_tree(&INTER_MODE_TREE, probs)?;
        let mode = InterMode::from_raw(raw).ok_or(TileSyntaxError::InvalidBitstream)?;
        increment_count(&mut self.counts.counts_inter_mode[ctx][mode.index()]);
        Ok(mode)
    }

    fn read_block_interp_filter(
        &mut self,
        row: usize,
        col: usize,
        left: NeighborModeInfo,
        above: NeighborModeInfo,
    ) -> Result<u8, TileSyntaxError> {
        match self
            .interpolation_filter
            .ok_or(TileSyntaxError::InvalidBitstream)?
        {
            InterpolationFilter::Switchable => {
                let left_interp = if col > self.tile_col_start && left.ref_frames[0] > INTRA_FRAME {
                    left.interp_filter
                } else {
                    SWITCHABLE_FILTER_SENTINEL
                };
                let above_interp = if row > 0 && above.ref_frames[0] > INTRA_FRAME {
                    above.interp_filter
                } else {
                    SWITCHABLE_FILTER_SENTINEL
                };
                let ctx = if left_interp == above_interp {
                    left_interp
                } else if left_interp == SWITCHABLE_FILTER_SENTINEL
                    && above_interp != SWITCHABLE_FILTER_SENTINEL
                {
                    above_interp
                } else if left_interp != SWITCHABLE_FILTER_SENTINEL
                    && above_interp == SWITCHABLE_FILTER_SENTINEL
                {
                    left_interp
                } else {
                    SWITCHABLE_FILTER_SENTINEL
                };
                let probs = &self.probabilities.interp_filter_probs[usize::from(ctx)];
                let filter = self.decoder.read_tree(&INTERP_FILTER_TREE, probs)?;
                let filter_index = usize::from(filter);
                if filter_index >= SWITCHABLE_FILTERS {
                    return Err(TileSyntaxError::InvalidBitstream);
                }
                increment_count(
                    &mut self.counts.counts_interp_filter[usize::from(ctx)][filter_index],
                );
                Ok(filter)
            }
            filter => fixed_interp_filter(filter),
        }
    }

    fn assign_mv(
        &mut self,
        inter_mode: InterMode,
        is_compound: bool,
        mv_state: &[MvRefState; REF_LISTS],
    ) -> Result<[MotionVector; REF_LISTS], TileSyntaxError> {
        let mut mvs = [MotionVector::ZERO; REF_LISTS];
        for ref_list in 0..(1 + usize::from(is_compound)) {
            mvs[ref_list] = match inter_mode {
                InterMode::New => self.read_mv(mv_state[ref_list].best)?,
                InterMode::Nearest => mv_state[ref_list].nearest,
                InterMode::Near => mv_state[ref_list].near,
                InterMode::Zero => MotionVector::ZERO,
            };
        }
        Ok(mvs)
    }

    fn read_mv(&mut self, best_mv: MotionVector) -> Result<MotionVector, TileSyntaxError> {
        let use_hp = self.allow_high_precision_mv && use_mv_hp(best_mv);
        let joint = self
            .decoder
            .read_tree(&MV_JOINT_TREE, &self.probabilities.mv_probs.joint)?;
        increment_count(&mut self.counts.counts_mv_joint[usize::from(joint)]);
        let mut diff_row = 0i32;
        let mut diff_col = 0i32;
        if matches!(joint, 2 | 3) {
            diff_row = self.read_mv_component(0, use_hp)?;
        }
        if matches!(joint, 1 | 3) {
            diff_col = self.read_mv_component(1, use_hp)?;
        }
        let diff = MotionVector::new(diff_row, diff_col)?;
        best_mv.add(diff)
    }

    fn read_mv_component(&mut self, comp: usize, use_hp: bool) -> Result<i32, TileSyntaxError> {
        let probs = &self.probabilities.mv_probs;
        let sign = self.decoder.read_bool(probs.sign[comp])?;
        increment_count(&mut self.counts.counts_mv_sign[comp][bool_index(sign)]);
        let mv_class = usize::from(self.decoder.read_tree(&MV_CLASS_TREE, &probs.class[comp])?);
        increment_count(&mut self.counts.counts_mv_class[comp][mv_class]);
        let mag = if mv_class == 0 {
            let class0_bit = usize::from(self.decoder.read_bool(probs.class0_bit[comp])?);
            increment_count(&mut self.counts.counts_mv_class0_bit[comp][class0_bit]);
            let class0_fr = usize::from(
                self.decoder
                    .read_tree(&MV_FR_TREE, &probs.class0_fr[comp][class0_bit])?,
            );
            increment_count(&mut self.counts.counts_mv_class0_fr[comp][class0_bit][class0_fr]);
            let class0_hp = if use_hp {
                usize::from(self.decoder.read_bool(probs.class0_hp[comp])?)
            } else {
                1
            };
            increment_count(&mut self.counts.counts_mv_class0_hp[comp][class0_hp]);
            ((class0_bit << 3) | (class0_fr << 1) | class0_hp) + 1
        } else {
            let mut d = 0usize;
            for i in 0..mv_class {
                if i >= MV_OFFSET_BITS {
                    return Err(TileSyntaxError::InvalidBitstream);
                }
                let mv_bit = self.decoder.read_bool(probs.bits[comp][i])?;
                increment_count(&mut self.counts.counts_mv_bits[comp][i][bool_index(mv_bit)]);
                if mv_bit {
                    d |= 1usize << i;
                }
            }
            let mv_fr = usize::from(self.decoder.read_tree(&MV_FR_TREE, &probs.fr[comp])?);
            increment_count(&mut self.counts.counts_mv_fr[comp][mv_fr]);
            let mv_hp = if use_hp {
                usize::from(self.decoder.read_bool(probs.hp[comp])?)
            } else {
                1
            };
            increment_count(&mut self.counts.counts_mv_hp[comp][mv_hp]);
            (CLASS0_SIZE << (mv_class + 2)) + ((d << 3) | (mv_fr << 1) | mv_hp) + 1
        };
        let mag = i32::try_from(mag).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        Ok(if sign { -mag } else { mag })
    }

    fn find_mv_refs(
        &self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        ref_frame: u8,
        block: i8,
    ) -> Result<MvRefState, TileSyntaxError> {
        let mut state = MvRefState::DEFAULT;
        let mut different_ref_found = false;
        let mut context_counter = 0usize;
        let search = &MV_REF_BLOCKS[block_size.index()];

        for candidate in search.iter().take(2) {
            if let Some(info) = self.mv_ref_candidate(row, col, candidate)? {
                different_ref_found = true;
                context_counter = context_counter
                    .checked_add(usize::from(MODE_2_COUNTER[usize::from(info.y_mode)]))
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                for ref_list in 0..REF_LISTS {
                    if info.ref_frames[ref_list] == ref_frame {
                        let mv = get_sub_block_mv(info, ref_list, candidate[1], block)?;
                        state.add_mv_ref(mv);
                        break;
                    }
                }
            }
        }

        for candidate in search.iter().skip(2) {
            if let Some(info) = self.mv_ref_candidate(row, col, candidate)? {
                different_ref_found = true;
                if_same_ref_frame_add_mv(&mut state, info, ref_frame);
            }
        }

        if self.use_prev_frame_mvs {
            if_same_prev_frame_add_mv(&mut state, self.prev_mv_ref_candidate(row, col)?, ref_frame);
        }
        if different_ref_found {
            for candidate in search {
                if let Some(info) = self.mv_ref_candidate(row, col, candidate)? {
                    if_diff_ref_frame_add_mv(&mut state, info, ref_frame, self.ref_sign_biases())?;
                }
            }
        }
        if self.use_prev_frame_mvs {
            if_diff_prev_frame_add_mv(
                &mut state,
                self.prev_mv_ref_candidate(row, col)?,
                ref_frame,
                self.ref_sign_biases(),
            )?;
        }

        let mode_context = *COUNTER_TO_CONTEXT
            .get(context_counter)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        if mode_context >= INTER_MODE_CONTEXTS_U8 {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        state.mode_context = usize::from(mode_context);
        for mv in &mut state.ref_list {
            *mv = self.clamp_mv_ref(row, col, block_size, *mv, MV_BORDER)?;
        }
        Ok(state)
    }

    fn append_sub8x8_mvs(
        &self,
        context: Sub8x8MvContext,
        block_mvs: &[[MotionVector; SUB_BLOCKS]; REF_LISTS],
        mut state: MvRefState,
    ) -> Result<MvRefState, TileSyntaxError> {
        let found = self.find_mv_refs(
            context.row,
            context.col,
            context.block_size,
            context.ref_frame,
            i8::try_from(context.block).map_err(|_| TileSyntaxError::InvalidBitstream)?,
        )?;
        state.mode_context = found.mode_context;
        let mut sub8x8 = [MotionVector::ZERO; MAX_MV_REF_CANDIDATES];
        let mut dst = 0usize;
        if context.block == 0 {
            sub8x8 = found.ref_list;
            dst = MAX_MV_REF_CANDIDATES;
        } else if context.block <= 2 {
            sub8x8[dst] = block_mvs[context.ref_list][0];
            dst += 1;
        } else {
            sub8x8[dst] = block_mvs[context.ref_list][2];
            dst += 1;
            for idx in (0..=1).rev() {
                let mv = block_mvs[context.ref_list][idx];
                if dst < MAX_MV_REF_CANDIDATES && mv != sub8x8[0] {
                    sub8x8[dst] = mv;
                    dst += 1;
                }
            }
        }
        for mv in found.ref_list {
            if dst < MAX_MV_REF_CANDIDATES && mv != sub8x8[0] {
                sub8x8[dst] = mv;
                dst += 1;
            }
        }
        if dst < MAX_MV_REF_CANDIDATES {
            sub8x8[dst] = MotionVector::ZERO;
        }
        state.nearest = sub8x8[0];
        state.near = sub8x8[1];
        Ok(state)
    }

    fn find_best_ref_mvs(
        &self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        mut state: MvRefState,
    ) -> MvRefState {
        for mv in &mut state.ref_list {
            if !self.allow_high_precision_mv || !use_mv_hp(*mv) {
                mv.row = i16::try_from(lower_mv_precision(i32::from(mv.row))).unwrap_or(0);
                mv.col = i16::try_from(lower_mv_precision(i32::from(mv.col))).unwrap_or(0);
            }
            *mv = self
                .clamp_mv_ref(
                    row,
                    col,
                    block_size,
                    *mv,
                    (BORDERINPIXELS - INTERP_EXTEND) << 3,
                )
                .unwrap_or(MotionVector::ZERO);
        }
        state.nearest = state.ref_list[0];
        state.near = state.ref_list[1];
        state.best = state.ref_list[0];
        state
    }

    fn mv_ref_candidate(
        &self,
        row: usize,
        col: usize,
        candidate: &[i8; 2],
    ) -> Result<Option<CandidateModeInfo>, TileSyntaxError> {
        let candidate_r = isize::try_from(row)
            .map_err(|_| TileSyntaxError::InvalidBitstream)?
            .checked_add(isize::from(candidate[0]))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let candidate_c = isize::try_from(col)
            .map_err(|_| TileSyntaxError::InvalidBitstream)?
            .checked_add(isize::from(candidate[1]))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        if candidate_r < 0
            || candidate_c
                < isize::try_from(self.tile_col_start)
                    .map_err(|_| TileSyntaxError::InvalidBitstream)?
            || candidate_c
                >= isize::try_from(self.tile_col_end)
                    .map_err(|_| TileSyntaxError::InvalidBitstream)?
            || candidate_r
                >= isize::try_from(self.mi_rows).map_err(|_| TileSyntaxError::InvalidBitstream)?
        {
            return Ok(None);
        }

        let candidate_r =
            usize::try_from(candidate_r).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let candidate_c =
            usize::try_from(candidate_c).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        // MV reference searches can reach several rows/columns away from the
        // current block.  The 1-D above/left probability contexts only describe
        // immediate neighbours, so use the exact decoded mode grid when it is
        // available.
        if let Some(current_frame_modes) = self.current_frame_modes.as_ref() {
            let index = candidate_r
                .checked_mul(self.mi_cols)
                .and_then(|value| value.checked_add(candidate_c))
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let info = current_frame_modes.get(index)?;
            return Ok(info.valid.then_some(CandidateModeInfo::from(info)));
        }
        if candidate_r < self.left_row_base {
            return self
                .contexts
                .above_mode(candidate_c)
                .map(|info| Some(CandidateModeInfo::from(info)));
        }
        if candidate_c < col {
            let row_offset = row_offset(self.left_row_base, candidate_r)?;
            return Ok(self
                .contexts
                .left_mode
                .get(row_offset)
                .copied()
                .map(CandidateModeInfo::from));
        }
        if candidate_r < row {
            return self
                .contexts
                .above_mode(candidate_c)
                .map(|info| Some(CandidateModeInfo::from(info)));
        }
        Ok(None)
    }

    fn prev_mv_ref_candidate(
        &self,
        row: usize,
        col: usize,
    ) -> Result<Option<StoredModeInfo>, TileSyntaxError> {
        let Some(prev_frame_modes) = self.prev_frame_modes else {
            return Ok(None);
        };
        if row >= self.mi_rows || col >= self.mi_cols {
            return Ok(None);
        }
        let index = row
            .checked_mul(self.mi_cols)
            .and_then(|value| value.checked_add(col))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let info = prev_frame_modes.get(index)?;
        Ok(info.valid.then_some(info))
    }

    fn clamp_mv_ref(
        &self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        mv: MotionVector,
        border: i32,
    ) -> Result<MotionVector, TileSyntaxError> {
        let bh = i32::from(block_size.num_8x8_high());
        let bw = i32::from(block_size.num_8x8_wide());
        let row = i32::try_from(row).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let col = i32::try_from(col).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let mi_rows = i32::try_from(self.mi_rows).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let mi_cols = i32::try_from(self.mi_cols).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let top = -(row * 64);
        let bottom = (mi_rows - bh - row) * 64;
        let left = -(col * 64);
        let right = (mi_cols - bw - col) * 64;
        MotionVector::new(
            clip3(top - border, bottom + border, i32::from(mv.row)),
            clip3(left - border, right + border, i32::from(mv.col)),
        )
    }

    fn ref_sign_biases(&self) -> [bool; 4] {
        // The uncompressed header only defines sign bias for inter references.
        // INTRA/NONE is always treated as unbiased for context calculations.
        [
            false,
            self.sign_bias(LAST_FRAME).unwrap_or(false),
            self.sign_bias(GOLDEN_FRAME).unwrap_or(false),
            self.sign_bias(ALTREF_FRAME).unwrap_or(false),
        ]
    }

    fn sign_bias(&self, ref_frame: u8) -> Result<bool, TileSyntaxError> {
        self.sign_bias_from_header(ref_frame)
    }

    fn sign_bias_from_header(&self, ref_frame: u8) -> Result<bool, TileSyntaxError> {
        // Tile parsing receives sign-bias values through the uncompressed header.
        // The four-element layout is INTRA/LAST/GOLDEN/ALTREF.
        // Stored as a helper so all raw reference-frame indexing stays checked.
        self.header_sign_bias(ref_frame)
    }

    fn header_sign_bias(&self, ref_frame: u8) -> Result<bool, TileSyntaxError> {
        self.ref_frame_sign_bias
            .get(usize::from(ref_frame))
            .copied()
            .ok_or(TileSyntaxError::InvalidBitstream)
    }

    fn seg_feature_active(&self, segment_id: u8, feature: usize) -> bool {
        self.segmentation.feature_active(segment_id, feature)
    }

    fn seg_feature_data(&self, segment_id: u8, feature: usize) -> Result<i16, TileSyntaxError> {
        self.segmentation
            .feature_data(segment_id, feature)
            .ok_or(TileSyntaxError::InvalidBitstream)
    }

    fn decode_residual(
        &mut self,
        row: usize,
        col: usize,
        mi_size: BlockSize,
        block: DecodedBlockInfo,
        current_frame: &mut CurrentFrameMut<'_>,
    ) -> Result<bool, TileSyntaxError> {
        let bsize = if mi_size < BlockSize::Block8x8 {
            BlockSize::Block8x8
        } else {
            mi_size
        };
        let mut any_nonzero = false;

        for plane in 0..PLANES {
            let tx_size = if plane > 0 {
                get_uv_tx_size(mi_size, block.tx_size)?
            } else {
                block.tx_size
            };
            let step = 1usize << tx_size.index();
            let plane_size = get_plane_block_size(bsize, plane)?;
            let num4x4w = usize::from(plane_size.num_4x4_wide());
            let num4x4h = usize::from(plane_size.num_4x4_high());
            let sub_x = subsampling_x(plane);
            let sub_y = subsampling_y(plane);
            let base_x = col
                .checked_mul(8)
                .map(|value| value >> sub_x)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let base_y = row
                .checked_mul(8)
                .map(|value| value >> sub_y)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let max_x = self
                .mi_cols
                .checked_mul(8)
                .map(|value| value >> sub_x)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let max_y = self
                .mi_rows
                .checked_mul(8)
                .map(|value| value >> sub_y)
                .ok_or(TileSyntaxError::InvalidBitstream)?;

            if block.is_inter {
                if mi_size < BlockSize::Block8x8 {
                    for y in 0..num4x4h {
                        for x in 0..num4x4w {
                            self.predict_inter(
                                current_frame,
                                InterPredictionContext {
                                    plane,
                                    mi_row: row,
                                    mi_col: col,
                                    start_x: base_x
                                        .checked_add(
                                            x.checked_mul(4)
                                                .ok_or(TileSyntaxError::InvalidBitstream)?,
                                        )
                                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                                    start_y: base_y
                                        .checked_add(
                                            y.checked_mul(4)
                                                .ok_or(TileSyntaxError::InvalidBitstream)?,
                                        )
                                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                                    width: 4,
                                    height: 4,
                                    block_idx: y
                                        .checked_mul(num4x4w)
                                        .and_then(|value| value.checked_add(x))
                                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                                    mi_size,
                                    block,
                                },
                            )?;
                        }
                    }
                } else {
                    self.predict_inter(
                        current_frame,
                        InterPredictionContext {
                            plane,
                            mi_row: row,
                            mi_col: col,
                            start_x: base_x,
                            start_y: base_y,
                            width: num4x4w
                                .checked_mul(4)
                                .ok_or(TileSyntaxError::InvalidBitstream)?,
                            height: num4x4h
                                .checked_mul(4)
                                .ok_or(TileSyntaxError::InvalidBitstream)?,
                            block_idx: 0,
                            mi_size,
                            block,
                        },
                    )?;
                }
            }

            let mut block_idx = 0usize;
            let mut y = 0usize;
            while y < num4x4h {
                let mut x = 0usize;
                while x < num4x4w {
                    let start_x = base_x
                        .checked_add(x.checked_mul(4).ok_or(TileSyntaxError::InvalidBitstream)?)
                        .ok_or(TileSyntaxError::InvalidBitstream)?;
                    let start_y = base_y
                        .checked_add(y.checked_mul(4).ok_or(TileSyntaxError::InvalidBitstream)?)
                        .ok_or(TileSyntaxError::InvalidBitstream)?;
                    let mut nonzero = false;
                    if start_x < max_x && start_y < max_y {
                        if !block.is_inter {
                            self.predict_intra(
                                current_frame,
                                IntraPredictionContext {
                                    plane,
                                    start_x,
                                    start_y,
                                    have_left: col > self.tile_col_start || x > 0,
                                    have_above: row > 0 || y > 0,
                                    not_on_right: x
                                        .checked_add(step)
                                        .ok_or(TileSyntaxError::InvalidBitstream)?
                                        < num4x4w,
                                    tx_size,
                                    block_idx,
                                    mi_size,
                                    block,
                                },
                            )?;
                        }

                        if !block.skip {
                            let coefficients = self.tokens(
                                plane,
                                (start_x, start_y),
                                tx_size,
                                block_idx,
                                mi_size,
                                block,
                            )?;
                            nonzero = coefficients.nonzero_context();
                            let mut dequantized =
                                self.dequant.dequantize(&coefficients, block.segment_id);
                            dequantized.inverse_transform(self.lossless)?;
                            reconstruct(current_frame, &dequantized)?;
                        }
                    }

                    self.contexts.update_nonzero_context(
                        self.left_row_base,
                        plane,
                        start_x,
                        start_y,
                        step,
                        nonzero,
                    )?;
                    any_nonzero |= nonzero;
                    block_idx = block_idx
                        .checked_add(1)
                        .ok_or(TileSyntaxError::InvalidBitstream)?;
                    x = x
                        .checked_add(step)
                        .ok_or(TileSyntaxError::InvalidBitstream)?;
                }
                y = y
                    .checked_add(step)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
            }
        }

        Ok(any_nonzero)
    }

    fn predict_intra(
        &self,
        current_frame: &mut CurrentFrameMut<'_>,
        context: IntraPredictionContext,
    ) -> Result<(), TileSyntaxError> {
        let mode = if context.plane > 0 {
            context.block.uv_mode
        } else if context.mi_size.is_at_least_8x8() {
            IntraMode::from_raw(context.block.y_mode).ok_or(TileSyntaxError::InvalidBitstream)?
        } else {
            *context
                .block
                .sub_modes
                .get(context.block_idx)
                .ok_or(TileSyntaxError::InvalidBitstream)?
        };
        let size = transform_width(context.tx_size);
        let plane = current_frame.plane_mut(context.plane)?;
        let edges = intra_prediction_edges(plane, context)?;
        let mut pred = [0u8; MAX_TX_COEFFS];
        intra_predict_block(
            IntraPredictionRequest {
                mode,
                have_left: context.have_left,
                have_above: context.have_above,
                size,
            },
            &edges,
            &mut pred,
        )?;
        write_prediction_block(plane, context.start_x, context.start_y, size, &pred)
    }

    fn predict_inter(
        &self,
        current_frame: &mut CurrentFrameMut<'_>,
        context: InterPredictionContext,
    ) -> Result<(), TileSyntaxError> {
        let reference_frames = self
            .reference_frames
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let is_compound = context.block.ref_frames[1] > NONE_FRAME;
        let ref_count = 1 + usize::from(is_compound);
        let mut refs = [None; REF_LISTS];
        let mut scaled = [ScaledMotion {
            start_x: 0,
            start_y: 0,
            step_x: SUBPEL_SHIFTS,
            step_y: SUBPEL_SHIFTS,
        }; REF_LISTS];

        for ref_list in 0..ref_count {
            let ref_frame = context.block.ref_frames[ref_list];
            let reference = reference_frames.get(ref_frame)?;
            let mv = select_inter_mv(
                context.plane,
                ref_list,
                context.block_idx,
                context.mi_size,
                context.block,
            )?;
            let clamped_mv = self.clamp_inter_mv(context, mv)?;
            scaled[ref_list] = self.scale_inter_mv(
                reference,
                context.plane,
                context.start_x,
                context.start_y,
                clamped_mv,
            )?;
            refs[ref_list] = Some(reference.plane(context.plane)?);
        }

        let interp_filter = usize::from(context.block.interp_filter);
        if interp_filter >= SUBPEL_FILTERS.len() {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let plane = current_frame.plane_mut(context.plane)?;
        let unscaled = scaled[..ref_count]
            .iter()
            .all(|motion| motion.step_x == SUBPEL_SHIFTS && motion.step_y == SUBPEL_SHIFTS);
        if unscaled {
            inter_predict_unscaled_block(
                refs[0].ok_or(TileSyntaxError::InvalidBitstream)?,
                scaled[0],
                interp_filter,
                plane,
                context,
                InterPredictionWrite::Store,
            )?;
            if is_compound {
                inter_predict_unscaled_block(
                    refs[1].ok_or(TileSyntaxError::InvalidBitstream)?,
                    scaled[1],
                    interp_filter,
                    plane,
                    context,
                    InterPredictionWrite::Average,
                )?;
            }
            return Ok(());
        }

        for row in 0..context.height {
            let y = context
                .start_y
                .checked_add(row)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            for col in 0..context.width {
                let x = context
                    .start_x
                    .checked_add(col)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                let pred0 = inter_predict_sample(
                    refs[0].ok_or(TileSyntaxError::InvalidBitstream)?,
                    scaled[0],
                    interp_filter,
                    row,
                    col,
                )?;
                let pred = if is_compound {
                    let pred1 = inter_predict_sample(
                        refs[1].ok_or(TileSyntaxError::InvalidBitstream)?,
                        scaled[1],
                        interp_filter,
                        row,
                        col,
                    )?;
                    avg2(pred0, pred1)
                } else {
                    pred0
                };
                plane.set_visible(x, y, pred)?;
            }
        }

        Ok(())
    }

    fn clamp_inter_mv(
        &self,
        context: InterPredictionContext,
        mv: MotionVector,
    ) -> Result<MotionVector, TileSyntaxError> {
        let sx = subsampling_x(context.plane);
        let sy = subsampling_y(context.plane);
        let bh = i32::from(context.mi_size.num_8x8_high());
        let bw = i32::from(context.mi_size.num_8x8_wide());
        let row = i32::try_from(context.mi_row).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let col = i32::try_from(context.mi_col).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let mi_rows = i32::try_from(self.mi_rows).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let mi_cols = i32::try_from(self.mi_cols).map_err(|_| TileSyntaxError::InvalidBitstream)?;

        let mb_to_top_edge = -(((row * MI_SIZE_PIXELS as i32) * SUBPEL_SHIFTS) >> sy);
        let mb_to_bottom_edge =
            (((mi_rows - bh - row) * MI_SIZE_PIXELS as i32) * SUBPEL_SHIFTS) >> sy;
        let mb_to_left_edge = -(((col * MI_SIZE_PIXELS as i32) * SUBPEL_SHIFTS) >> sx);
        let mb_to_right_edge =
            (((mi_cols - bw - col) * MI_SIZE_PIXELS as i32) * SUBPEL_SHIFTS) >> sx;
        let spel_left = (INTERP_EXTEND + ((bw * MI_SIZE_PIXELS as i32) >> sx)) << SUBPEL_BITS;
        let spel_right = spel_left - SUBPEL_SHIFTS;
        let spel_top = (INTERP_EXTEND + ((bh * MI_SIZE_PIXELS as i32) >> sy)) << SUBPEL_BITS;
        let spel_bottom = spel_top - SUBPEL_SHIFTS;

        MotionVector::new(
            clip3(
                mb_to_top_edge - spel_top,
                mb_to_bottom_edge + spel_bottom,
                (2 * i32::from(mv.row)) >> sy,
            ),
            clip3(
                mb_to_left_edge - spel_left,
                mb_to_right_edge + spel_right,
                (2 * i32::from(mv.col)) >> sx,
            ),
        )
    }

    fn scale_inter_mv(
        &self,
        reference: ReferenceFrame<'_>,
        plane: usize,
        x: usize,
        y: usize,
        clamped_mv: MotionVector,
    ) -> Result<ScaledMotion, TileSyntaxError> {
        reference.plane(plane)?;
        let ref_width =
            i64::try_from(reference.y.width).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let ref_height =
            i64::try_from(reference.y.height).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let frame_width = i64::from(self.frame_width);
        let frame_height = i64::from(self.frame_height);
        if frame_width <= 0 || frame_height <= 0 {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let x_scale = (ref_width << REF_SCALE_SHIFT) / frame_width;
        let y_scale = (ref_height << REF_SCALE_SHIFT) / frame_height;
        let x = i64::try_from(x).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let y = i64::try_from(y).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let base_x = (x * x_scale) >> REF_SCALE_SHIFT;
        let base_y = (y * y_scale) >> REF_SCALE_SHIFT;
        let luma_x = if plane > 0 { x << SUBSAMPLING_X } else { x };
        let luma_y = if plane > 0 { y << SUBSAMPLING_Y } else { y };
        let frac_x =
            ((SUBPEL_SHIFTS as i64 * luma_x * x_scale) >> REF_SCALE_SHIFT) & i64::from(SUBPEL_MASK);
        let frac_y =
            ((SUBPEL_SHIFTS as i64 * luma_y * y_scale) >> REF_SCALE_SHIFT) & i64::from(SUBPEL_MASK);
        let dx = ((i64::from(clamped_mv.col) * x_scale) >> REF_SCALE_SHIFT) + frac_x;
        let dy = ((i64::from(clamped_mv.row) * y_scale) >> REF_SCALE_SHIFT) + frac_y;
        let step_x = (SUBPEL_SHIFTS as i64 * x_scale) >> REF_SCALE_SHIFT;
        let step_y = (SUBPEL_SHIFTS as i64 * y_scale) >> REF_SCALE_SHIFT;

        Ok(ScaledMotion {
            start_x: i32::try_from((base_x << SUBPEL_BITS) + dx)
                .map_err(|_| TileSyntaxError::InvalidBitstream)?,
            start_y: i32::try_from((base_y << SUBPEL_BITS) + dy)
                .map_err(|_| TileSyntaxError::InvalidBitstream)?,
            step_x: i32::try_from(step_x).map_err(|_| TileSyntaxError::InvalidBitstream)?,
            step_y: i32::try_from(step_y).map_err(|_| TileSyntaxError::InvalidBitstream)?,
        })
    }

    fn tokens(
        &mut self,
        plane: usize,
        start: (usize, usize),
        tx_size: TxSize,
        block_idx: usize,
        mi_size: BlockSize,
        block: DecodedBlockInfo,
    ) -> Result<TransformCoefficients, TileSyntaxError> {
        let seg_eob = 16usize << (tx_size.index() << 1);
        let tx_type = self.get_tx_type(plane, tx_size, block_idx, mi_size, block)?;
        let mut coefficients = TransformCoefficients::new(plane, start, tx_size, tx_type)?;
        let mut token_cache = [0u8; MAX_TX_COEFFS];
        let mut check_eob = true;
        let mut c = 0usize;
        let scan = scan_table(tx_size, tx_type);
        let coef_bands = coef_band_table(tx_size);
        let dc_ctx = self.contexts.coef_context(
            self.left_row_base,
            plane,
            start,
            tx_size,
            (self.mi_rows, self.mi_cols),
        )?;
        let tx_index = tx_size.index();
        let plane_type = usize::from(plane > 0);
        let ref_type = usize::from(block.is_inter);
        let coef_probs = &self.probabilities.coef_probs[tx_index][plane_type][ref_type];
        let counts = &mut *self.counts;
        let counts_more_coefs = &mut counts.counts_more_coefs[tx_index][plane_type][ref_type];
        let counts_token = &mut counts.counts_token[tx_index][plane_type][ref_type];
        let decoder = &mut self.decoder;

        while c < seg_eob {
            let pos = usize::from(scan[c]);
            let band = usize::from(coef_bands[c]);
            let ctx = if c == 0 {
                dc_ctx
            } else {
                coefficient_token_context(pos, tx_size, tx_type, &token_cache)?
            };
            let probability_row = &coef_probs[band][ctx];

            if check_eob
                && !read_more_coefs(decoder, probability_row, &mut counts_more_coefs[band][ctx])?
            {
                break;
            }

            let token = read_token(decoder, probability_row, &mut counts_token[band][ctx])?;
            token_cache[pos] = ENERGY_CLASS[token.index()];
            if token == CoefToken::Zero {
                check_eob = false;
            } else {
                let coef = read_coef(decoder, token)?;
                let sign_bit = decoder.read_literal(1)?;
                coefficients.set_signed(pos, coef, sign_bit)?;
                check_eob = true;
            }

            c = c.checked_add(1).ok_or(TileSyntaxError::InvalidBitstream)?;
        }

        coefficients.set_eob(c)?;
        Ok(coefficients)
    }

    fn get_tx_type(
        &self,
        plane: usize,
        tx_size: TxSize,
        block_idx: usize,
        mi_size: BlockSize,
        block: DecodedBlockInfo,
    ) -> Result<TxType, TileSyntaxError> {
        if plane > 0 || tx_size == TxSize::Tx32x32 || self.lossless {
            return Ok(TxType::DctDct);
        }
        if block.is_inter {
            return Ok(TxType::DctDct);
        }

        let mode = if tx_size == TxSize::Tx4x4 && mi_size < BlockSize::Block8x8 {
            *block
                .sub_modes
                .get(block_idx)
                .ok_or(TileSyntaxError::InvalidBitstream)?
        } else {
            IntraMode::from_raw(block.y_mode).ok_or(TileSyntaxError::InvalidBitstream)?
        };
        Ok(MODE_TO_TXFM_MAP[mode.index()])
    }
}

fn select_inter_mv(
    plane: usize,
    ref_list: usize,
    block_idx: usize,
    mi_size: BlockSize,
    block: DecodedBlockInfo,
) -> Result<MotionVector, TileSyntaxError> {
    if ref_list >= REF_LISTS || block_idx >= SUB_BLOCKS {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    let sx = subsampling_x(plane);
    let sy = subsampling_y(plane);
    if plane == 0 || mi_size.is_at_least_8x8() || (sx == 0 && sy == 0) {
        return Ok(block.block_mvs[ref_list][block_idx]);
    }

    let component = |idx: usize, comp: usize| -> Result<i32, TileSyntaxError> {
        let mv = *block
            .block_mvs
            .get(ref_list)
            .and_then(|mvs| mvs.get(idx))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        Ok(if comp == 0 {
            i32::from(mv.row)
        } else {
            i32::from(mv.col)
        })
    };

    let (row, col) = if sx == 0 {
        (
            round_mv_comp_q2(component(block_idx, 0)? + component(block_idx + 2, 0)?),
            round_mv_comp_q2(component(block_idx, 1)? + component(block_idx + 2, 1)?),
        )
    } else if sy == 0 {
        (
            round_mv_comp_q2(component(block_idx, 0)? + component(block_idx + 1, 0)?),
            round_mv_comp_q2(component(block_idx, 1)? + component(block_idx + 1, 1)?),
        )
    } else {
        let mut row = 0i32;
        let mut col = 0i32;
        for idx in 0..SUB_BLOCKS {
            row += component(idx, 0)?;
            col += component(idx, 1)?;
        }
        (round_mv_comp_q4(row), round_mv_comp_q4(col))
    };
    MotionVector::new(row, col)
}

fn round_mv_comp_q2(value: i32) -> i32 {
    if value < 0 {
        (value - 1) / 2
    } else {
        (value + 1) / 2
    }
}

fn round_mv_comp_q4(value: i32) -> i32 {
    if value < 0 {
        (value - 2) / 4
    } else {
        (value + 2) / 4
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InterPredictionWrite {
    Store,
    Average,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UnscaledInterPrediction {
    src_x: i32,
    src_y: i32,
    x_phase: usize,
    y_phase: usize,
    context: InterPredictionContext,
    write: InterPredictionWrite,
}

#[inline(never)]
fn inter_predict_unscaled_block(
    reference: ReferencePlane<'_>,
    scaled: ScaledMotion,
    interp_filter: usize,
    plane: &mut CurrentPlaneMut<'_>,
    context: InterPredictionContext,
    write: InterPredictionWrite,
) -> Result<(), TileSyntaxError> {
    if context.width == 0
        || context.height == 0
        || context.width > MAX_INTER_PRED_SIZE
        || context.height > MAX_INTER_PRED_SIZE
    {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    let request = UnscaledInterPrediction {
        src_x: scaled.start_x >> SUBPEL_BITS,
        src_y: scaled.start_y >> SUBPEL_BITS,
        x_phase: usize::try_from(scaled.start_x & SUBPEL_MASK)
            .map_err(|_| TileSyntaxError::InvalidBitstream)?,
        y_phase: usize::try_from(scaled.start_y & SUBPEL_MASK)
            .map_err(|_| TileSyntaxError::InvalidBitstream)?,
        context,
        write,
    };

    if request.x_phase == 0 && request.y_phase == 0 {
        return inter_predict_integer_unscaled_block(reference, plane, request);
    }

    let x_filter = SUBPEL_FILTERS
        .get(interp_filter)
        .and_then(|filters| filters.get(request.x_phase))
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let y_filter = SUBPEL_FILTERS
        .get(interp_filter)
        .and_then(|filters| filters.get(request.y_phase))
        .ok_or(TileSyntaxError::InvalidBitstream)?;

    inter_predict_subpel_unscaled_block(reference, plane, request, x_filter, y_filter)
}

#[inline(never)]
fn inter_predict_integer_unscaled_block(
    reference: ReferencePlane<'_>,
    plane: &mut CurrentPlaneMut<'_>,
    request: UnscaledInterPrediction,
) -> Result<(), TileSyntaxError> {
    let context = request.context;
    if reference_rect_inside(
        reference,
        request.src_x,
        request.src_y,
        context.width,
        context.height,
    )? {
        let src_x =
            usize::try_from(request.src_x).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let src_y =
            usize::try_from(request.src_y).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        for row in 0..context.height {
            let src_start = src_y
                .checked_add(row)
                .and_then(|y| y.checked_mul(reference.stride))
                .and_then(|base| base.checked_add(src_x))
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let src_end = src_start
                .checked_add(context.width)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let prediction = reference
                .data
                .get(src_start..src_end)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let dst_y = context
                .start_y
                .checked_add(row)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            write_inter_prediction_row(plane, context.start_x, dst_y, prediction, request.write)?;
        }
        return Ok(());
    }

    inter_predict_integer_edge_block(reference, plane, request)
}

#[inline(never)]
fn inter_predict_integer_edge_block(
    reference: ReferencePlane<'_>,
    plane: &mut CurrentPlaneMut<'_>,
    request: UnscaledInterPrediction,
) -> Result<(), TileSyntaxError> {
    let context = request.context;
    let mut buffer = [0u8; MAX_INTERP_BUFFER];
    gather_clamped_reference_rect(
        reference,
        request.src_x,
        request.src_y,
        context.width,
        context.height,
        &mut buffer,
    )?;

    for row in 0..context.height {
        let offset = row
            .checked_mul(MAX_INTERP_SOURCE_DIM)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let prediction = buffer
            .get(offset..offset + context.width)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let dst_y = context
            .start_y
            .checked_add(row)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        write_inter_prediction_row(plane, context.start_x, dst_y, prediction, request.write)?;
    }

    Ok(())
}

#[inline(never)]
fn inter_predict_subpel_unscaled_block(
    reference: ReferencePlane<'_>,
    plane: &mut CurrentPlaneMut<'_>,
    request: UnscaledInterPrediction,
    x_filter: &[i16; INTERP_TAPS],
    y_filter: &[i16; INTERP_TAPS],
) -> Result<(), TileSyntaxError> {
    let context = request.context;
    let source_left = request
        .src_x
        .checked_sub(3)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let source_top = request
        .src_y
        .checked_sub(3)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let source_width = context
        .width
        .checked_add(INTERP_TAPS - 1)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let source_height = context
        .height
        .checked_add(INTERP_TAPS - 1)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let mut buffer = [0u8; MAX_INTERP_BUFFER];

    if reference_rect_inside(
        reference,
        source_left,
        source_top,
        source_width,
        source_height,
    )? {
        let left = usize::try_from(source_left).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let top = usize::try_from(source_top).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        horizontal_filter_reference_rect(
            reference,
            (left, top),
            (context.width, source_height),
            request.x_phase,
            x_filter,
            &mut buffer,
        )?;
    } else {
        gather_clamped_reference_rect(
            reference,
            source_left,
            source_top,
            source_width,
            source_height,
            &mut buffer,
        )?;
        horizontal_filter_buffer_in_place(
            context.width,
            source_height,
            request.x_phase,
            x_filter,
            &mut buffer,
        )?;
    }

    write_vertical_filtered_block(plane, request, y_filter, &buffer)
}

fn reference_rect_inside(
    reference: ReferencePlane<'_>,
    left: i32,
    top: i32,
    width: usize,
    height: usize,
) -> Result<bool, TileSyntaxError> {
    let right = i64::from(left)
        .checked_add(i64::try_from(width).map_err(|_| TileSyntaxError::InvalidBitstream)?)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let bottom = i64::from(top)
        .checked_add(i64::try_from(height).map_err(|_| TileSyntaxError::InvalidBitstream)?)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let reference_width =
        i64::try_from(reference.width).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let reference_height =
        i64::try_from(reference.height).map_err(|_| TileSyntaxError::InvalidBitstream)?;

    Ok(left >= 0 && top >= 0 && right <= reference_width && bottom <= reference_height)
}

fn gather_clamped_reference_rect(
    reference: ReferencePlane<'_>,
    left: i32,
    top: i32,
    width: usize,
    height: usize,
    buffer: &mut [u8; MAX_INTERP_BUFFER],
) -> Result<(), TileSyntaxError> {
    let last_x =
        i32::try_from(reference.width - 1).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let last_y =
        i32::try_from(reference.height - 1).map_err(|_| TileSyntaxError::InvalidBitstream)?;

    for row in 0..height {
        let src_y = usize::try_from(clip3(
            0,
            last_y,
            top.checked_add(i32::try_from(row).map_err(|_| TileSyntaxError::InvalidBitstream)?)
                .ok_or(TileSyntaxError::InvalidBitstream)?,
        ))
        .map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let src_row = src_y
            .checked_mul(reference.stride)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let dst_row = row
            .checked_mul(MAX_INTERP_SOURCE_DIM)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        for col in 0..width {
            let src_x = usize::try_from(clip3(
                0,
                last_x,
                left.checked_add(
                    i32::try_from(col).map_err(|_| TileSyntaxError::InvalidBitstream)?,
                )
                .ok_or(TileSyntaxError::InvalidBitstream)?,
            ))
            .map_err(|_| TileSyntaxError::InvalidBitstream)?;
            let sample_index = src_row
                .checked_add(src_x)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            buffer[dst_row + col] = *reference
                .data
                .get(sample_index)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
        }
    }

    Ok(())
}

fn horizontal_filter_reference_rect(
    reference: ReferencePlane<'_>,
    origin: (usize, usize),
    size: (usize, usize),
    x_phase: usize,
    filter: &[i16; INTERP_TAPS],
    buffer: &mut [u8; MAX_INTERP_BUFFER],
) -> Result<(), TileSyntaxError> {
    let (left, top) = origin;
    let (width, height) = size;
    let source_width = width
        .checked_add(INTERP_TAPS - 1)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    for row in 0..height {
        let src_start = top
            .checked_add(row)
            .and_then(|y| y.checked_mul(reference.stride))
            .and_then(|base| base.checked_add(left))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let src_end = src_start
            .checked_add(source_width)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let src = reference
            .data
            .get(src_start..src_end)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let dst_start = row
            .checked_mul(MAX_INTERP_SOURCE_DIM)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let dst_end = dst_start
            .checked_add(width)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let dst = buffer
            .get_mut(dst_start..dst_end)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        horizontal_filter_row_to_buffer(src, width, x_phase, filter, dst);
    }

    Ok(())
}

fn horizontal_filter_buffer_in_place(
    width: usize,
    height: usize,
    x_phase: usize,
    filter: &[i16; INTERP_TAPS],
    buffer: &mut [u8; MAX_INTERP_BUFFER],
) -> Result<(), TileSyntaxError> {
    let source_width = width
        .checked_add(INTERP_TAPS - 1)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    for row in 0..height {
        let row_start = row
            .checked_mul(MAX_INTERP_SOURCE_DIM)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let row_end = row_start
            .checked_add(source_width)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let source_row = buffer
            .get_mut(row_start..row_end)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        horizontal_filter_row_in_place(source_row, width, x_phase, filter);
    }

    Ok(())
}

#[inline(always)]
fn horizontal_filter_row_to_buffer(
    src: &[u8],
    width: usize,
    x_phase: usize,
    filter: &[i16; INTERP_TAPS],
    dst: &mut [u8],
) {
    if x_phase == 0 {
        dst.copy_from_slice(&src[3..3 + width]);
        return;
    }

    let c0 = i32::from(filter[0]);
    let c1 = i32::from(filter[1]);
    let c2 = i32::from(filter[2]);
    let c3 = i32::from(filter[3]);
    let c4 = i32::from(filter[4]);
    let c5 = i32::from(filter[5]);
    let c6 = i32::from(filter[6]);
    let c7 = i32::from(filter[7]);

    for col in 0..width {
        let sum = c0 * i32::from(src[col])
            + c1 * i32::from(src[col + 1])
            + c2 * i32::from(src[col + 2])
            + c3 * i32::from(src[col + 3])
            + c4 * i32::from(src[col + 4])
            + c5 * i32::from(src[col + 5])
            + c6 * i32::from(src[col + 6])
            + c7 * i32::from(src[col + 7]);
        dst[col] = clip1(round2_i32(sum, 7));
    }
}

#[inline(always)]
fn horizontal_filter_row_in_place(
    row: &mut [u8],
    width: usize,
    x_phase: usize,
    filter: &[i16; INTERP_TAPS],
) {
    if x_phase == 0 {
        row.copy_within(3..3 + width, 0);
        return;
    }

    let c0 = i32::from(filter[0]);
    let c1 = i32::from(filter[1]);
    let c2 = i32::from(filter[2]);
    let c3 = i32::from(filter[3]);
    let c4 = i32::from(filter[4]);
    let c5 = i32::from(filter[5]);
    let c6 = i32::from(filter[6]);
    let c7 = i32::from(filter[7]);

    for col in 0..width {
        let sum = c0 * i32::from(row[col])
            + c1 * i32::from(row[col + 1])
            + c2 * i32::from(row[col + 2])
            + c3 * i32::from(row[col + 3])
            + c4 * i32::from(row[col + 4])
            + c5 * i32::from(row[col + 5])
            + c6 * i32::from(row[col + 6])
            + c7 * i32::from(row[col + 7]);
        row[col] = clip1(round2_i32(sum, 7));
    }
}

fn write_vertical_filtered_block(
    plane: &mut CurrentPlaneMut<'_>,
    request: UnscaledInterPrediction,
    filter: &[i16; INTERP_TAPS],
    buffer: &[u8; MAX_INTERP_BUFFER],
) -> Result<(), TileSyntaxError> {
    let context = request.context;
    if request.y_phase == 0 {
        for row in 0..context.height {
            let prediction_start = row
                .checked_add(3)
                .and_then(|y| y.checked_mul(MAX_INTERP_SOURCE_DIM))
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let prediction_end = prediction_start
                .checked_add(context.width)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let prediction = buffer
                .get(prediction_start..prediction_end)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let dst_y = context
                .start_y
                .checked_add(row)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            write_inter_prediction_row(plane, context.start_x, dst_y, prediction, request.write)?;
        }
        return Ok(());
    }

    let c0 = i32::from(filter[0]);
    let c1 = i32::from(filter[1]);
    let c2 = i32::from(filter[2]);
    let c3 = i32::from(filter[3]);
    let c4 = i32::from(filter[4]);
    let c5 = i32::from(filter[5]);
    let c6 = i32::from(filter[6]);
    let c7 = i32::from(filter[7]);

    for row in 0..context.height {
        let dst_y = context
            .start_y
            .checked_add(row)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let Some(dst) = inter_prediction_row_mut(plane, context.start_x, dst_y, context.width)?
        else {
            continue;
        };
        let row_start = row
            .checked_mul(MAX_INTERP_SOURCE_DIM)
            .ok_or(TileSyntaxError::InvalidBitstream)?;

        match request.write {
            InterPredictionWrite::Store => {
                for (col, dst_sample) in dst.iter_mut().enumerate() {
                    let base = row_start + col;
                    let sum = c0 * i32::from(buffer[base])
                        + c1 * i32::from(buffer[base + MAX_INTERP_SOURCE_DIM])
                        + c2 * i32::from(buffer[base + 2 * MAX_INTERP_SOURCE_DIM])
                        + c3 * i32::from(buffer[base + 3 * MAX_INTERP_SOURCE_DIM])
                        + c4 * i32::from(buffer[base + 4 * MAX_INTERP_SOURCE_DIM])
                        + c5 * i32::from(buffer[base + 5 * MAX_INTERP_SOURCE_DIM])
                        + c6 * i32::from(buffer[base + 6 * MAX_INTERP_SOURCE_DIM])
                        + c7 * i32::from(buffer[base + 7 * MAX_INTERP_SOURCE_DIM]);
                    *dst_sample = clip1(round2_i32(sum, 7));
                }
            }
            InterPredictionWrite::Average => {
                for (col, dst_sample) in dst.iter_mut().enumerate() {
                    let base = row_start + col;
                    let sum = c0 * i32::from(buffer[base])
                        + c1 * i32::from(buffer[base + MAX_INTERP_SOURCE_DIM])
                        + c2 * i32::from(buffer[base + 2 * MAX_INTERP_SOURCE_DIM])
                        + c3 * i32::from(buffer[base + 3 * MAX_INTERP_SOURCE_DIM])
                        + c4 * i32::from(buffer[base + 4 * MAX_INTERP_SOURCE_DIM])
                        + c5 * i32::from(buffer[base + 5 * MAX_INTERP_SOURCE_DIM])
                        + c6 * i32::from(buffer[base + 6 * MAX_INTERP_SOURCE_DIM])
                        + c7 * i32::from(buffer[base + 7 * MAX_INTERP_SOURCE_DIM]);
                    *dst_sample = avg2(*dst_sample, clip1(round2_i32(sum, 7)));
                }
            }
        }
    }

    Ok(())
}

fn write_inter_prediction_row(
    plane: &mut CurrentPlaneMut<'_>,
    x: usize,
    y: usize,
    prediction: &[u8],
    write: InterPredictionWrite,
) -> Result<(), TileSyntaxError> {
    let Some(dst) = inter_prediction_row_mut(plane, x, y, prediction.len())? else {
        return Ok(());
    };

    match write {
        InterPredictionWrite::Store => dst.copy_from_slice(&prediction[..dst.len()]),
        InterPredictionWrite::Average => {
            for (dst, &prediction) in dst.iter_mut().zip(prediction.iter()) {
                *dst = avg2(*dst, prediction);
            }
        }
    }

    Ok(())
}

fn inter_prediction_row_mut<'a>(
    plane: &'a mut CurrentPlaneMut<'_>,
    x: usize,
    y: usize,
    width: usize,
) -> Result<Option<&'a mut [u8]>, TileSyntaxError> {
    if width == 0 || y >= plane.height || x >= plane.width {
        return Ok(None);
    }

    let width = core::cmp::min(width, plane.width - x);
    let start = y
        .checked_mul(plane.stride)
        .and_then(|base| base.checked_add(x))
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let end = start
        .checked_add(width)
        .ok_or(TileSyntaxError::InvalidBitstream)?;

    plane
        .data
        .get_mut(start..end)
        .map(Some)
        .ok_or(TileSyntaxError::InvalidBitstream)
}

fn inter_predict_sample(
    reference: ReferencePlane<'_>,
    scaled: ScaledMotion,
    interp_filter: usize,
    row: usize,
    col: usize,
) -> Result<u8, TileSyntaxError> {
    let row = i32::try_from(row).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let col = i32::try_from(col).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let x = scaled
        .start_x
        .checked_add(
            scaled
                .step_x
                .checked_mul(col)
                .ok_or(TileSyntaxError::InvalidBitstream)?,
        )
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let y = scaled
        .start_y
        .checked_add(
            scaled
                .step_y
                .checked_mul(row)
                .ok_or(TileSyntaxError::InvalidBitstream)?,
        )
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    if x & SUBPEL_MASK == 0 && y & SUBPEL_MASK == 0 {
        return reference.sample_clamped(x >> SUBPEL_BITS, y >> SUBPEL_BITS);
    }
    let x_filter = subpel_filter(interp_filter, x)?;
    let y_filter = subpel_filter(interp_filter, y)?;
    let mut sum = 0i32;

    for (t, &coeff) in y_filter.iter().enumerate() {
        let t = i32::try_from(t).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let intermediate =
            horizontal_intermediate_sample(reference, x, (y >> SUBPEL_BITS) + t - 3, x_filter)?;
        sum += i32::from(coeff) * i32::from(intermediate);
    }

    Ok(clip1(round2_i32(sum, 7)))
}

fn horizontal_intermediate_sample(
    reference: ReferencePlane<'_>,
    x: i32,
    y: i32,
    filter: &[i16; 8],
) -> Result<u8, TileSyntaxError> {
    let mut sum = 0i32;
    for (t, &coeff) in filter.iter().enumerate() {
        let t = i32::try_from(t).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let sample = reference.sample_clamped((x >> SUBPEL_BITS) + t - 3, y)?;
        sum += i32::from(coeff) * i32::from(sample);
    }
    Ok(clip1(round2_i32(sum, 7)))
}

fn subpel_filter(
    interp_filter: usize,
    position: i32,
) -> Result<&'static [i16; 8], TileSyntaxError> {
    let subpel =
        usize::try_from(position & SUBPEL_MASK).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    SUBPEL_FILTERS
        .get(interp_filter)
        .and_then(|filters| filters.get(subpel))
        .ok_or(TileSyntaxError::InvalidBitstream)
}

fn intra_prediction_edges(
    plane: &CurrentPlaneMut<'_>,
    context: IntraPredictionContext,
) -> Result<IntraPredictionEdges, TileSyntaxError> {
    let size = transform_width(context.tx_size);
    let mut edges = IntraPredictionEdges {
        above_left: 127,
        above_row: [127; MAX_INTRA_ABOVE],
        left_col: [129; MAX_TX_WIDTH],
    };

    if context.have_above {
        let above_y = context
            .start_y
            .checked_sub(1)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        for i in 0..size {
            edges.above_row[i] = plane.sample_clamped(
                context
                    .start_x
                    .checked_add(i)
                    .ok_or(TileSyntaxError::InvalidBitstream)?,
                above_y,
            )?;
        }
        for i in size..(2 * size) {
            edges.above_row[i] = if context.not_on_right && context.tx_size == TxSize::Tx4x4 {
                plane.sample_clamped(
                    context
                        .start_x
                        .checked_add(i)
                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                    above_y,
                )?
            } else {
                edges.above_row[size - 1]
            };
        }
        edges.above_left = if context.have_left {
            plane.sample_clamped(
                context
                    .start_x
                    .checked_sub(1)
                    .ok_or(TileSyntaxError::InvalidBitstream)?,
                above_y,
            )?
        } else {
            129
        };
    }

    if context.have_left {
        let left_x = context
            .start_x
            .checked_sub(1)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        for i in 0..size {
            edges.left_col[i] = plane.sample_clamped(
                left_x,
                context
                    .start_y
                    .checked_add(i)
                    .ok_or(TileSyntaxError::InvalidBitstream)?,
            )?;
        }
    }

    Ok(edges)
}

fn intra_predict_block(
    request: IntraPredictionRequest,
    edges: &IntraPredictionEdges,
    pred: &mut [u8; MAX_TX_COEFFS],
) -> Result<(), TileSyntaxError> {
    let size = request.size;
    if !matches!(size, 4 | 8 | 16 | 32) {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    match request.mode {
        IntraMode::Dc => dc_predict(request, edges, pred),
        IntraMode::V => {
            for row in 0..size {
                for col in 0..size {
                    pred[row * size + col] = edges.above_row[col];
                }
            }
        }
        IntraMode::H => {
            for row in 0..size {
                for col in 0..size {
                    pred[row * size + col] = edges.left_col[row];
                }
            }
        }
        IntraMode::D45 => {
            for row in 0..size {
                for col in 0..size {
                    let index = row
                        .checked_add(col)
                        .ok_or(TileSyntaxError::InvalidBitstream)?;
                    pred[row * size + col] = if index + 2 < size * 2 {
                        avg3(
                            edges.above_row[index],
                            edges.above_row[index + 1],
                            edges.above_row[index + 2],
                        )
                    } else {
                        edges.above_row[2 * size - 1]
                    };
                }
            }
        }
        IntraMode::D135 => {
            pred[0] = avg3(edges.left_col[0], edges.above_left, edges.above_row[0]);
            for (col, slot) in pred.iter_mut().enumerate().take(size).skip(1) {
                *slot = avg3(
                    above_with_left(edges, col as isize - 2),
                    above_with_left(edges, col as isize - 1),
                    edges.above_row[col],
                );
            }
            if size > 1 {
                pred[size] = avg3(edges.above_left, edges.left_col[0], edges.left_col[1]);
            }
            for row in 2..size {
                pred[row * size] = avg3(
                    edges.left_col[row - 2],
                    edges.left_col[row - 1],
                    edges.left_col[row],
                );
            }
            for row in 1..size {
                for col in 1..size {
                    pred[row * size + col] = pred[(row - 1) * size + col - 1];
                }
            }
        }
        IntraMode::D117 => {
            for (col, slot) in pred.iter_mut().enumerate().take(size) {
                *slot = avg2(
                    above_with_left(edges, col as isize - 1),
                    edges.above_row[col],
                );
            }
            if size > 1 {
                pred[size] = avg3(edges.left_col[0], edges.above_left, edges.above_row[0]);
                for col in 1..size {
                    pred[size + col] = avg3(
                        above_with_left(edges, col as isize - 2),
                        above_with_left(edges, col as isize - 1),
                        edges.above_row[col],
                    );
                }
            }
            if size > 2 {
                pred[2 * size] = avg3(edges.above_left, edges.left_col[0], edges.left_col[1]);
            }
            for row in 3..size {
                pred[row * size] = avg3(
                    edges.left_col[row - 3],
                    edges.left_col[row - 2],
                    edges.left_col[row - 1],
                );
            }
            for row in 2..size {
                for col in 1..size {
                    pred[row * size + col] = pred[(row - 2) * size + col - 1];
                }
            }
        }
        IntraMode::D153 => {
            pred[0] = avg2(edges.left_col[0], edges.above_left);
            for row in 1..size {
                pred[row * size] = avg2(edges.left_col[row - 1], edges.left_col[row]);
            }
            if size > 1 {
                pred[1] = avg3(edges.left_col[0], edges.above_left, edges.above_row[0]);
                pred[size + 1] = avg3(edges.above_left, edges.left_col[0], edges.left_col[1]);
                for row in 2..size {
                    pred[row * size + 1] = avg3(
                        edges.left_col[row - 2],
                        edges.left_col[row - 1],
                        edges.left_col[row],
                    );
                }
            }
            for (col, slot) in pred.iter_mut().enumerate().take(size).skip(2) {
                *slot = avg3(
                    above_with_left(edges, col as isize - 3),
                    above_with_left(edges, col as isize - 2),
                    above_with_left(edges, col as isize - 1),
                );
            }
            for row in 1..size {
                for col in 2..size {
                    pred[row * size + col] = pred[(row - 1) * size + col - 2];
                }
            }
        }
        IntraMode::D207 => {
            for col in 0..size {
                pred[(size - 1) * size + col] = edges.left_col[size - 1];
            }
            for row in 0..size - 1 {
                pred[row * size] = avg2(edges.left_col[row], edges.left_col[row + 1]);
            }
            for row in 0..size - 2 {
                pred[row * size + 1] = avg3(
                    edges.left_col[row],
                    edges.left_col[row + 1],
                    edges.left_col[row + 2],
                );
            }
            pred[(size - 2) * size + 1] =
                avg3_last_weighted(edges.left_col[size - 2], edges.left_col[size - 1]);
            for col in 2..size {
                for row in (0..=size - 2).rev() {
                    pred[row * size + col] = pred[(row + 1) * size + col - 2];
                }
            }
        }
        IntraMode::D63 => {
            for row in 0..size {
                for col in 0..size {
                    let index = row / 2 + col;
                    pred[row * size + col] = if row & 1 != 0 {
                        avg3(
                            edges.above_row[index],
                            edges.above_row[index + 1],
                            edges.above_row[index + 2],
                        )
                    } else {
                        avg2(edges.above_row[index], edges.above_row[index + 1])
                    };
                }
            }
        }
        IntraMode::Tm => {
            for row in 0..size {
                for col in 0..size {
                    pred[row * size + col] = clip1(
                        i32::from(edges.above_row[col]) + i32::from(edges.left_col[row])
                            - i32::from(edges.above_left),
                    );
                }
            }
        }
    }

    Ok(())
}

fn dc_predict(
    request: IntraPredictionRequest,
    edges: &IntraPredictionEdges,
    pred: &mut [u8; MAX_TX_COEFFS],
) {
    let size = request.size;
    let log2_size = tx_width_log2(size);
    let value = if request.have_left && request.have_above {
        let mut sum = 0u32;
        for i in 0..size {
            sum += u32::from(edges.left_col[i]) + u32::from(edges.above_row[i]);
        }
        ((sum + size as u32) >> (log2_size + 1)) as u8
    } else if request.have_left {
        let mut sum = 0u32;
        for i in 0..size {
            sum += u32::from(edges.left_col[i]);
        }
        ((sum + (1u32 << (log2_size - 1))) >> log2_size) as u8
    } else if request.have_above {
        let mut sum = 0u32;
        for i in 0..size {
            sum += u32::from(edges.above_row[i]);
        }
        ((sum + (1u32 << (log2_size - 1))) >> log2_size) as u8
    } else {
        128
    };

    for row in 0..size {
        for col in 0..size {
            pred[row * size + col] = value;
        }
    }
}

fn write_prediction_block(
    plane: &mut CurrentPlaneMut<'_>,
    start_x: usize,
    start_y: usize,
    size: usize,
    pred: &[u8; MAX_TX_COEFFS],
) -> Result<(), TileSyntaxError> {
    for row in 0..size {
        let y = start_y
            .checked_add(row)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        for col in 0..size {
            let x = start_x
                .checked_add(col)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            plane.set_visible(x, y, pred[row * size + col])?;
        }
    }
    Ok(())
}

fn reconstruct(
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

fn add_residual_block(
    plane: &mut CurrentPlaneMut<'_>,
    start: (usize, usize),
    tx_size: TxSize,
    residuals: &[i32; MAX_TX_COEFFS],
) -> Result<(), TileSyntaxError> {
    let size = transform_width(tx_size);
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

#[derive(Clone, Copy, Debug)]
struct LoopFilterConfig<'a> {
    params: LoopFilterParams,
    segmentation: SegmentationParams,
    modes: ModeInfoView<'a>,
    mi_rows: usize,
    mi_cols: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LoopFilterStrength {
    lvl: u8,
    limit: u8,
    blimit: u8,
    thresh: u8,
}

impl LoopFilterStrength {
    const ZERO: Self = Self {
        lvl: 0,
        limit: 0,
        blimit: 0,
        thresh: 0,
    };
}

const LOOP_FILTER_REF_FRAMES: usize = 4;
const LOOP_FILTER_MODE_TYPES: usize = 2;
const LOOP_FILTER_SB_MIS: usize = MI_BLOCK_64 * MI_BLOCK_64;

#[derive(Clone, Copy, Debug)]
struct LoopFilterStrengthLut {
    by_level: [LoopFilterStrength; 64],
    by_segment_ref_mode:
        [[[LoopFilterStrength; LOOP_FILTER_MODE_TYPES]; LOOP_FILTER_REF_FRAMES]; MAX_SEGMENTS],
}

#[derive(Clone, Copy, Debug)]
struct LoopFilterMiInfo {
    valid: bool,
    skip: bool,
    tx_size: TxSize,
    uv_tx_size: TxSize,
    mi_size: BlockSize,
    ref_frame: u8,
    strength: LoopFilterStrength,
}

impl LoopFilterMiInfo {
    const INVALID: Self = Self {
        valid: false,
        skip: false,
        tx_size: TxSize::Tx4x4,
        uv_tx_size: TxSize::Tx4x4,
        mi_size: BlockSize::Block8x8,
        ref_frame: INTRA_FRAME,
        strength: LoopFilterStrength::ZERO,
    };

    const fn tx_size_for_plane(self, plane_index: usize) -> TxSize {
        if plane_index == 0 {
            self.tx_size
        } else {
            self.uv_tx_size
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LoopFilterSuperblockInfo {
    mi: [LoopFilterMiInfo; LOOP_FILTER_SB_MIS],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LoopFilterMasks {
    hev: bool,
    filter: bool,
    flat: bool,
    flat2: bool,
}

fn loop_filter_frame(
    header: &UncompressedFrameHeader,
    current_frame: &mut CurrentFrameMut<'_>,
    mode_buffers: &FrameModeBuffers<'_>,
) -> Result<(), TileSyntaxError> {
    if header.loop_filter.level == 0 {
        return Ok(());
    }
    if header.profile != 0 || header.bit_depth != 8 {
        return Err(TileSyntaxError::Unimplemented);
    }

    let modes = mode_buffers
        .current_frame_modes
        .as_ref()
        .ok_or(TileSyntaxError::InvalidBitstream)?
        .as_view();
    let config = LoopFilterConfig {
        params: header.loop_filter,
        segmentation: header.segmentation,
        modes,
        mi_rows: mi_size(header.frame_height)?,
        mi_cols: mi_size(header.frame_width)?,
    };
    let strengths = loop_filter_strength_lut(config.params, config.segmentation)?;

    let mut row = 0usize;
    while row < config.mi_rows {
        let mut col = 0usize;
        while col < config.mi_cols {
            let sb_info = loop_filter_superblock_info(config, &strengths, row, col)?;
            for plane in 0..PLANES {
                for pass in 0..2 {
                    loop_filter_superblock(current_frame, config, &sb_info, plane, pass, row, col)?;
                }
            }
            col = col
                .checked_add(MI_BLOCK_64)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
        }
        row = row
            .checked_add(MI_BLOCK_64)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
    }

    Ok(())
}

fn loop_filter_strength_lut(
    params: LoopFilterParams,
    segmentation: SegmentationParams,
) -> Result<LoopFilterStrengthLut, TileSyntaxError> {
    let mut by_level = [LoopFilterStrength::ZERO; 64];
    for (lvl, slot) in by_level.iter_mut().enumerate() {
        *slot = loop_filter_strength_from_level(params, lvl as u8)?;
    }

    let mut by_segment_ref_mode = [[[LoopFilterStrength::ZERO; LOOP_FILTER_MODE_TYPES];
        LOOP_FILTER_REF_FRAMES]; MAX_SEGMENTS];
    for (segment_id, segment_levels) in by_segment_ref_mode.iter_mut().enumerate() {
        for (ref_frame, ref_levels) in segment_levels.iter_mut().enumerate() {
            for (mode_type, slot) in ref_levels.iter_mut().enumerate() {
                let lvl = loop_filter_level(
                    params,
                    segmentation,
                    segment_id as u8,
                    ref_frame as u8,
                    mode_type != 0,
                )?;
                *slot = by_level[usize::from(lvl)];
            }
        }
    }

    Ok(LoopFilterStrengthLut {
        by_level,
        by_segment_ref_mode,
    })
}

fn loop_filter_superblock_info(
    config: LoopFilterConfig<'_>,
    strengths: &LoopFilterStrengthLut,
    row: usize,
    col: usize,
) -> Result<LoopFilterSuperblockInfo, TileSyntaxError> {
    let mut mi = [LoopFilterMiInfo::INVALID; LOOP_FILTER_SB_MIS];
    let rows = core::cmp::min(
        MI_BLOCK_64,
        config
            .mi_rows
            .checked_sub(row)
            .ok_or(TileSyntaxError::InvalidBitstream)?,
    );
    let cols = core::cmp::min(
        MI_BLOCK_64,
        config
            .mi_cols
            .checked_sub(col)
            .ok_or(TileSyntaxError::InvalidBitstream)?,
    );

    for local_row in 0..rows {
        let mode_row = row
            .checked_add(local_row)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        for local_col in 0..cols {
            let mode_col = col
                .checked_add(local_col)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let info = loop_filter_mode_info(config, mode_row, mode_col)?;
            let ref_frame = info.ref_frames[0];
            let mode_type = loop_filter_mode_type(info.y_mode);
            let strength = if usize::from(info.segment_id) < MAX_SEGMENTS
                && usize::from(ref_frame) < LOOP_FILTER_REF_FRAMES
            {
                strengths.by_segment_ref_mode[usize::from(info.segment_id)][usize::from(ref_frame)]
                    [bool_index(mode_type)]
            } else {
                let lvl = loop_filter_level(
                    config.params,
                    config.segmentation,
                    info.segment_id,
                    ref_frame,
                    mode_type,
                )?;
                strengths.by_level[usize::from(lvl)]
            };
            let index = local_row
                .checked_mul(MI_BLOCK_64)
                .and_then(|base| base.checked_add(local_col))
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            mi[index] = LoopFilterMiInfo {
                valid: true,
                skip: info.skip,
                tx_size: info.tx_size,
                uv_tx_size: get_uv_tx_size(info.mi_size, info.tx_size)?,
                mi_size: info.mi_size,
                ref_frame,
                strength,
            };
        }
    }

    Ok(LoopFilterSuperblockInfo { mi })
}

fn loop_filter_superblock_mi(
    sb_info: &LoopFilterSuperblockInfo,
    sb_row: usize,
    sb_col: usize,
    row: usize,
    col: usize,
) -> Result<LoopFilterMiInfo, TileSyntaxError> {
    let local_row = row
        .checked_sub(sb_row)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let local_col = col
        .checked_sub(sb_col)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    if local_row >= MI_BLOCK_64 || local_col >= MI_BLOCK_64 {
        return Err(TileSyntaxError::InvalidBitstream);
    }
    let index = local_row
        .checked_mul(MI_BLOCK_64)
        .and_then(|base| base.checked_add(local_col))
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let info = sb_info
        .mi
        .get(index)
        .copied()
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    if !info.valid {
        return Err(TileSyntaxError::InvalidBitstream);
    }
    Ok(info)
}

fn loop_filter_superblock(
    current_frame: &mut CurrentFrameMut<'_>,
    config: LoopFilterConfig<'_>,
    sb_info: &LoopFilterSuperblockInfo,
    plane_index: usize,
    pass: usize,
    row: usize,
    col: usize,
) -> Result<(), TileSyntaxError> {
    let sub_x = subsampling_x(plane_index);
    let sub_y = subsampling_y(plane_index);
    let (sub, edge_len) = if pass == 0 {
        (sub_x, 64usize >> sub_y)
    } else {
        (sub_y, 64usize >> sub_x)
    };
    let edge_count = 16usize >> sub;
    let mi_cols_luma = config
        .mi_cols
        .checked_mul(8)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let mi_rows_luma = config
        .mi_rows
        .checked_mul(8)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let sb_x = col
        .checked_mul(8)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let sb_y = row
        .checked_mul(8)
        .ok_or(TileSyntaxError::InvalidBitstream)?;

    let plane = current_frame.plane_mut(plane_index)?;
    for edge in 0..edge_count {
        let fixed_x = if pass == 0 {
            Some(
                sb_x.checked_add(
                    edge.checked_mul(4 << sub_x)
                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                )
                .ok_or(TileSyntaxError::InvalidBitstream)?,
            )
        } else {
            None
        };
        let fixed_y = if pass == 1 {
            Some(
                sb_y.checked_add(
                    edge.checked_mul(4 << sub_y)
                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                )
                .ok_or(TileSyntaxError::InvalidBitstream)?,
            )
        } else {
            None
        };

        if pass == 0 {
            let x = fixed_x.ok_or(TileSyntaxError::InvalidBitstream)?;
            if !loop_filter_on_screen(pass, x, sb_y, mi_cols_luma, mi_rows_luma) {
                continue;
            }
        } else {
            let y = fixed_y.ok_or(TileSyntaxError::InvalidBitstream)?;
            if !loop_filter_on_screen(pass, sb_x, y, mi_cols_luma, mi_rows_luma) {
                continue;
            }
        }

        let guard_varies_with_i =
            pass == 1 && sub_x == 1 && !config.mi_cols.is_multiple_of(2) && !edge.is_multiple_of(2);
        let mut i = 0usize;
        while i < edge_len {
            let (x, y) = if pass == 0 {
                let x = fixed_x.ok_or(TileSyntaxError::InvalidBitstream)?;
                (
                    x,
                    sb_y.checked_add(i << sub_y)
                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                )
            } else {
                let y = fixed_y.ok_or(TileSyntaxError::InvalidBitstream)?;
                (
                    sb_x.checked_add(i << sub_x)
                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                    y,
                )
            };

            if pass == 0 {
                if y >= mi_rows_luma {
                    break;
                }
            } else if x >= mi_cols_luma {
                break;
            }

            let mut segment_len = core::cmp::min(8 - (i & 7), edge_len - i);
            let visible_len = if pass == 0 {
                let step = 1usize << sub_y;
                let remaining = mi_rows_luma
                    .checked_sub(y)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                remaining.div_ceil(step)
            } else {
                let step = 1usize << sub_x;
                let remaining = mi_cols_luma
                    .checked_sub(x)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                remaining.div_ceil(step)
            };
            segment_len = core::cmp::min(segment_len, visible_len);
            if guard_varies_with_i {
                segment_len = 1;
            }

            let loop_col = ((x >> 3) >> sub_x) << sub_x;
            let loop_row = ((y >> 3) >> sub_y) << sub_y;
            let info = loop_filter_superblock_mi(sb_info, row, col, loop_row, loop_col)?;
            let tx_sz = info.tx_size_for_plane(plane_index);
            let sb_size = if sub == 0 {
                info.mi_size
            } else {
                core::cmp::max(BlockSize::Block16x16, info.mi_size)
            };
            let is_block_edge = loop_filter_is_block_edge(pass, x, y, sb_size);
            let is_tx_edge =
                loop_filter_is_tx_edge(pass, edge, x, tx_sz, sub_x, config.mi_cols, mi_cols_luma)?;
            let apply_filter =
                is_block_edge || (is_tx_edge && (info.ref_frame == INTRA_FRAME || !info.skip));
            if !apply_filter {
                i += segment_len;
                continue;
            }

            let filter_size = loop_filter_size(LoopFilterSizeInput {
                tx_size: tx_sz,
                is_32_edge: edge.is_multiple_of(8),
                pass,
                x,
                y,
                sub_x,
                sub_y,
                mi_rows: config.mi_rows,
                mi_cols: config.mi_cols,
            });
            let strength = info.strength;
            if strength.lvl == 0 {
                i += segment_len;
                continue;
            }

            loop_filter_segment(
                plane,
                pass,
                x >> sub_x,
                y >> sub_y,
                segment_len,
                filter_size,
                strength,
            )?;
            i += segment_len;
        }
    }

    Ok(())
}

fn loop_filter_on_screen(
    pass: usize,
    x: usize,
    y: usize,
    mi_cols_luma: usize,
    mi_rows_luma: usize,
) -> bool {
    if x >= mi_cols_luma || y >= mi_rows_luma {
        return false;
    }
    if pass == 0 && x == 0 {
        return false;
    }
    if pass == 1 && y == 0 {
        return false;
    }
    true
}

fn loop_filter_mode_info(
    config: LoopFilterConfig<'_>,
    row: usize,
    col: usize,
) -> Result<StoredModeInfo, TileSyntaxError> {
    if row >= config.mi_rows || col >= config.mi_cols {
        return Err(TileSyntaxError::InvalidBitstream);
    }
    let index = row
        .checked_mul(config.mi_cols)
        .and_then(|value| value.checked_add(col))
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let info = config.modes.get(index)?;
    if !info.valid {
        return Err(TileSyntaxError::InvalidBitstream);
    }
    Ok(info)
}

fn loop_filter_is_block_edge(pass: usize, x: usize, y: usize, sb_size: BlockSize) -> bool {
    if pass == 0 {
        x.is_multiple_of(8 * usize::from(sb_size.num_8x8_wide()))
    } else {
        y.is_multiple_of(8 * usize::from(sb_size.num_8x8_high()))
    }
}

fn loop_filter_is_tx_edge(
    pass: usize,
    edge: usize,
    x: usize,
    tx_size: TxSize,
    sub_x: usize,
    mi_cols: usize,
    mi_cols_luma: usize,
) -> Result<bool, TileSyntaxError> {
    if pass == 1
        && sub_x == 1
        && !mi_cols.is_multiple_of(2)
        && !edge.is_multiple_of(2)
        && x.checked_add(8).ok_or(TileSyntaxError::InvalidBitstream)? >= mi_cols_luma
    {
        return Ok(false);
    }
    Ok(edge.is_multiple_of(1usize << tx_size.index()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LoopFilterSizeInput {
    tx_size: TxSize,
    is_32_edge: bool,
    pass: usize,
    x: usize,
    y: usize,
    sub_x: usize,
    sub_y: usize,
    mi_rows: usize,
    mi_cols: usize,
}

fn loop_filter_size(input: LoopFilterSizeInput) -> TxSize {
    let base_size = if input.tx_size == TxSize::Tx4x4 && input.is_32_edge {
        TxSize::Tx8x8
    } else {
        TxSize::from_index(core::cmp::min(
            TxSize::Tx16x16.index(),
            input.tx_size.index(),
        ))
    };

    let crosses_chroma_right =
        input.pass == 0 && input.sub_x == 1 && (input.x >> 3) == input.mi_cols - 1;
    let crosses_chroma_bottom =
        input.pass == 1 && input.sub_y == 1 && (input.y >> 3) == input.mi_rows - 1;
    if base_size == TxSize::Tx16x16 && (crosses_chroma_right || crosses_chroma_bottom) {
        TxSize::Tx8x8
    } else {
        base_size
    }
}

fn loop_filter_level(
    params: LoopFilterParams,
    segmentation: SegmentationParams,
    segment_id: u8,
    ref_frame: u8,
    mode_type: bool,
) -> Result<u8, TileSyntaxError> {
    let mut lvl = i32::from(params.level);
    if segmentation.feature_active(segment_id, SEG_LVL_ALT_L) {
        let mut data = i32::from(
            segmentation
                .feature_data(segment_id, SEG_LVL_ALT_L)
                .ok_or(TileSyntaxError::InvalidBitstream)?,
        );
        if !segmentation.abs_or_delta_update {
            data += lvl;
        }
        lvl = clip3(0, 63, data);
    }
    if params.delta_enabled {
        let n_shift = i32::from(params.level >> 5);
        let ref_delta = params
            .ref_deltas
            .get(usize::from(ref_frame))
            .copied()
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        lvl += i32::from(ref_delta) << n_shift;
        if ref_frame > INTRA_FRAME {
            let mode_delta = params.mode_deltas[bool_index(mode_type)];
            lvl += i32::from(mode_delta) << n_shift;
        }
        lvl = clip3(0, 63, lvl);
    }
    u8::try_from(lvl).map_err(|_| TileSyntaxError::InvalidBitstream)
}

fn loop_filter_strength_from_level(
    params: LoopFilterParams,
    lvl: u8,
) -> Result<LoopFilterStrength, TileSyntaxError> {
    let shift = if params.sharpness > 4 {
        2
    } else if params.sharpness > 0 {
        1
    } else {
        0
    };
    let limit = if params.sharpness > 0 {
        clip3(1, i32::from(9 - params.sharpness), i32::from(lvl >> shift))
    } else {
        core::cmp::max(1, i32::from(lvl >> shift))
    };
    let blimit = 2 * (i32::from(lvl) + 2) + limit;
    Ok(LoopFilterStrength {
        lvl,
        limit: u8::try_from(limit).map_err(|_| TileSyntaxError::InvalidBitstream)?,
        blimit: u8::try_from(blimit).map_err(|_| TileSyntaxError::InvalidBitstream)?,
        thresh: lvl >> 4,
    })
}

fn loop_filter_mode_type(y_mode: u8) -> bool {
    matches!(y_mode, NEARESTMV | NEARMV | NEWMV)
}

fn loop_filter_segment(
    plane: &mut CurrentPlaneMut<'_>,
    pass: usize,
    x: usize,
    y: usize,
    len: usize,
    filter_size: TxSize,
    strength: LoopFilterStrength,
) -> Result<(), TileSyntaxError> {
    if len == 0 {
        return Ok(());
    }

    if pass == 0 {
        if x >= 8
            && x.checked_add(7).ok_or(TileSyntaxError::InvalidBitstream)? < plane.width
            && y.checked_add(len)
                .ok_or(TileSyntaxError::InvalidBitstream)?
                <= plane.height
        {
            let mut base = y
                .checked_mul(plane.stride)
                .and_then(|row| row.checked_add(x))
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            for _ in 0..len {
                sample_filter_direct(plane.data, base, 1, filter_size, strength);
                base = base
                    .checked_add(plane.stride)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
            }
            return Ok(());
        }

        if y >= plane.height {
            return Ok(());
        }
        let x = i32::try_from(x).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        for offset in 0..len {
            let sample_y = y
                .checked_add(offset)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            sample_filter(
                plane,
                x,
                i32::try_from(sample_y).map_err(|_| TileSyntaxError::InvalidBitstream)?,
                1,
                0,
                filter_size,
                strength,
            )?;
        }
    } else {
        if y >= 8
            && y.checked_add(7).ok_or(TileSyntaxError::InvalidBitstream)? < plane.height
            && x.checked_add(len)
                .ok_or(TileSyntaxError::InvalidBitstream)?
                <= plane.width
        {
            let mut base = y
                .checked_mul(plane.stride)
                .and_then(|row| row.checked_add(x))
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            for _ in 0..len {
                sample_filter_direct(plane.data, base, plane.stride, filter_size, strength);
                base = base
                    .checked_add(1)
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
            }
            return Ok(());
        }

        if x >= plane.width {
            return Ok(());
        }
        let y = i32::try_from(y).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        for offset in 0..len {
            let sample_x = x
                .checked_add(offset)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            sample_filter(
                plane,
                i32::try_from(sample_x).map_err(|_| TileSyntaxError::InvalidBitstream)?,
                y,
                0,
                1,
                filter_size,
                strength,
            )?;
        }
    }

    Ok(())
}

#[inline(always)]
fn sample_filter_direct(
    data: &mut [u8],
    base: usize,
    step: usize,
    filter_size: TxSize,
    strength: LoopFilterStrength,
) {
    let q0 = data[base];
    let q1 = data[base + step];
    let q2 = data[base + 2 * step];
    let q3 = data[base + 3 * step];
    let p0 = data[base - step];
    let p1 = data[base - 2 * step];
    let p2 = data[base - 3 * step];
    let p3 = data[base - 4 * step];

    let hev = abs_diff(p1, p0) > i32::from(strength.thresh)
        || abs_diff(q1, q0) > i32::from(strength.thresh);
    let limit = i32::from(strength.limit);
    let blimit = i32::from(strength.blimit);
    let mask = abs_diff(p3, p2) > limit
        || abs_diff(p2, p1) > limit
        || abs_diff(p1, p0) > limit
        || abs_diff(q1, q0) > limit
        || abs_diff(q2, q1) > limit
        || abs_diff(q3, q2) > limit
        || abs_diff(p0, q0) * 2 + abs_diff(p1, q1) / 2 > blimit;
    if mask {
        return;
    }

    let flat = filter_size >= TxSize::Tx8x8
        && abs_diff(p1, p0) <= 1
        && abs_diff(q1, q0) <= 1
        && abs_diff(p2, p0) <= 1
        && abs_diff(q2, q0) <= 1
        && abs_diff(p3, p0) <= 1
        && abs_diff(q3, q0) <= 1;

    if filter_size == TxSize::Tx4x4 || !flat {
        narrow_filter_direct(data, base, step, [p1, p0, q0, q1], hev);
        return;
    }

    let mut p = [0u8; 8];
    let mut q = [0u8; 8];
    p[0] = p0;
    p[1] = p1;
    p[2] = p2;
    p[3] = p3;
    q[0] = q0;
    q[1] = q1;
    q[2] = q2;
    q[3] = q3;

    if filter_size == TxSize::Tx8x8 {
        wide_filter_direct(data, base, step, &p, &q, 3);
        return;
    }

    q[4] = data[base + 4 * step];
    q[5] = data[base + 5 * step];
    q[6] = data[base + 6 * step];
    q[7] = data[base + 7 * step];
    p[4] = data[base - 5 * step];
    p[5] = data[base - 6 * step];
    p[6] = data[base - 7 * step];
    p[7] = data[base - 8 * step];

    let flat2 = abs_diff(p[7], p0) <= 1
        && abs_diff(q[7], q0) <= 1
        && abs_diff(p[6], p0) <= 1
        && abs_diff(q[6], q0) <= 1
        && abs_diff(p[5], p0) <= 1
        && abs_diff(q[5], q0) <= 1
        && abs_diff(p[4], p0) <= 1
        && abs_diff(q[4], q0) <= 1;
    if flat2 {
        wide_filter_direct(data, base, step, &p, &q, 4);
    } else {
        wide_filter_direct(data, base, step, &p, &q, 3);
    }
}

#[inline(always)]
fn narrow_filter_direct(data: &mut [u8], base: usize, step: usize, samples: [u8; 4], hev: bool) {
    let [p1, p0, q0, q1] = samples;
    let ps1 = i32::from(p1) - 128;
    let ps0 = i32::from(p0) - 128;
    let qs0 = i32::from(q0) - 128;
    let qs1 = i32::from(q1) - 128;
    let mut filter = if hev { filter4_clamp(ps1 - qs1) } else { 0 };
    filter = filter4_clamp(filter + 3 * (qs0 - ps0));
    let filter1 = filter4_clamp(filter + 4) >> 3;
    let filter2 = filter4_clamp(filter + 3) >> 3;
    let oq0 = filter4_clamp(qs0 - filter1) + 128;
    let op0 = filter4_clamp(ps0 + filter2) + 128;
    data[base] = oq0 as u8;
    data[base - step] = op0 as u8;

    if !hev {
        filter = round2_i32(filter1, 1);
        let oq1 = filter4_clamp(qs1 - filter) + 128;
        let op1 = filter4_clamp(ps1 + filter) + 128;
        data[base + step] = oq1 as u8;
        data[base - 2 * step] = op1 as u8;
    }
}

#[inline(always)]
fn wide_filter_direct(
    data: &mut [u8],
    base: usize,
    step: usize,
    p: &[u8; 8],
    q: &[u8; 8],
    log2_size: u8,
) {
    let n = (1i32 << (log2_size - 1)) - 1;
    let mut filtered = [0u8; 14];
    let min_sample = -(n + 1);
    let max_sample = n;
    // Same clipped window as the VP9 wide-filter definition, but slide it one
    // output sample at a time instead of rebuilding the sum from scratch.
    let mut t = i32::from(loop_filter_direct_sample(p, q, -n));
    for offset in -2 * n..=0 {
        t += i32::from(loop_filter_direct_sample(
            p,
            q,
            clip3(min_sample, max_sample, offset),
        ));
    }
    for i in -n..n {
        filtered[(i + n) as usize] = clip1(round2_i32(t, log2_size));
        if i + 1 < n {
            t += i32::from(loop_filter_direct_sample(
                p,
                q,
                clip3(min_sample, max_sample, i + n + 1),
            )) + i32::from(loop_filter_direct_sample(p, q, i + 1))
                - i32::from(loop_filter_direct_sample(
                    p,
                    q,
                    clip3(min_sample, max_sample, i - n),
                ))
                - i32::from(loop_filter_direct_sample(p, q, i));
        }
    }
    for i in -n..n {
        data[loop_filter_direct_index(base, step, i)] = filtered[(i + n) as usize];
    }
}

#[inline(always)]
fn loop_filter_direct_sample(p: &[u8; 8], q: &[u8; 8], offset: i32) -> u8 {
    if offset >= 0 {
        q[offset as usize]
    } else {
        p[(-offset - 1) as usize]
    }
}

#[inline(always)]
fn loop_filter_direct_index(base: usize, step: usize, offset: i32) -> usize {
    if offset >= 0 {
        base + offset as usize * step
    } else {
        base - ((-offset) as usize) * step
    }
}

fn sample_filter(
    plane: &mut CurrentPlaneMut<'_>,
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    filter_size: TxSize,
    strength: LoopFilterStrength,
) -> Result<(), TileSyntaxError> {
    let masks = filter_masks(plane, x, y, dx, dy, filter_size, strength)?;
    if !masks.filter {
        return Ok(());
    }
    if filter_size == TxSize::Tx4x4 || !masks.flat {
        narrow_filter(plane, x, y, dx, dy, masks.hev)
    } else if filter_size == TxSize::Tx8x8 || !masks.flat2 {
        wide_filter(plane, x, y, dx, dy, 3)
    } else {
        wide_filter(plane, x, y, dx, dy, 4)
    }
}

fn filter_masks(
    plane: &CurrentPlaneMut<'_>,
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    filter_size: TxSize,
    strength: LoopFilterStrength,
) -> Result<LoopFilterMasks, TileSyntaxError> {
    let mut q = [0u8; 8];
    let mut p = [0u8; 8];
    for k in 0..8i32 {
        q[usize::try_from(k).map_err(|_| TileSyntaxError::InvalidBitstream)?] =
            loop_filter_sample(plane, x + dx * k, y + dy * k)?;
        let pk = k + 1;
        p[usize::try_from(k).map_err(|_| TileSyntaxError::InvalidBitstream)?] =
            loop_filter_sample(plane, x - dx * pk, y - dy * pk)?;
    }

    let hev = abs_diff(p[1], p[0]) > i32::from(strength.thresh)
        || abs_diff(q[1], q[0]) > i32::from(strength.thresh);
    let limit = i32::from(strength.limit);
    let blimit = i32::from(strength.blimit);
    let mask = abs_diff(p[3], p[2]) > limit
        || abs_diff(p[2], p[1]) > limit
        || abs_diff(p[1], p[0]) > limit
        || abs_diff(q[1], q[0]) > limit
        || abs_diff(q[2], q[1]) > limit
        || abs_diff(q[3], q[2]) > limit
        || abs_diff(p[0], q[0]) * 2 + abs_diff(p[1], q[1]) / 2 > blimit;
    let filter = !mask;

    let mut flat = false;
    if filter_size >= TxSize::Tx8x8 {
        flat = abs_diff(p[1], p[0]) <= 1
            && abs_diff(q[1], q[0]) <= 1
            && abs_diff(p[2], p[0]) <= 1
            && abs_diff(q[2], q[0]) <= 1
            && abs_diff(p[3], p[0]) <= 1
            && abs_diff(q[3], q[0]) <= 1;
    }

    let mut flat2 = false;
    if filter_size >= TxSize::Tx16x16 {
        flat2 = abs_diff(p[7], p[0]) <= 1
            && abs_diff(q[7], q[0]) <= 1
            && abs_diff(p[6], p[0]) <= 1
            && abs_diff(q[6], q[0]) <= 1
            && abs_diff(p[5], p[0]) <= 1
            && abs_diff(q[5], q[0]) <= 1
            && abs_diff(p[4], p[0]) <= 1
            && abs_diff(q[4], q[0]) <= 1;
    }

    Ok(LoopFilterMasks {
        hev,
        filter,
        flat,
        flat2,
    })
}

fn narrow_filter(
    plane: &mut CurrentPlaneMut<'_>,
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    hev: bool,
) -> Result<(), TileSyntaxError> {
    let q0 = loop_filter_sample(plane, x, y)?;
    let q1 = loop_filter_sample(plane, x + dx, y + dy)?;
    let p0 = loop_filter_sample(plane, x - dx, y - dy)?;
    let p1 = loop_filter_sample(plane, x - 2 * dx, y - 2 * dy)?;
    let ps1 = i32::from(p1) - 128;
    let ps0 = i32::from(p0) - 128;
    let qs0 = i32::from(q0) - 128;
    let qs1 = i32::from(q1) - 128;
    let mut filter = if hev { filter4_clamp(ps1 - qs1) } else { 0 };
    filter = filter4_clamp(filter + 3 * (qs0 - ps0));
    let filter1 = filter4_clamp(filter + 4) >> 3;
    let filter2 = filter4_clamp(filter + 3) >> 3;
    let oq0 = filter4_clamp(qs0 - filter1) + 128;
    let op0 = filter4_clamp(ps0 + filter2) + 128;
    loop_filter_set(
        plane,
        x,
        y,
        u8::try_from(oq0).map_err(|_| TileSyntaxError::InvalidBitstream)?,
    )?;
    loop_filter_set(
        plane,
        x - dx,
        y - dy,
        u8::try_from(op0).map_err(|_| TileSyntaxError::InvalidBitstream)?,
    )?;

    if !hev {
        filter = round2_i32(filter1, 1);
        let oq1 = filter4_clamp(qs1 - filter) + 128;
        let op1 = filter4_clamp(ps1 + filter) + 128;
        loop_filter_set(
            plane,
            x + dx,
            y + dy,
            u8::try_from(oq1).map_err(|_| TileSyntaxError::InvalidBitstream)?,
        )?;
        loop_filter_set(
            plane,
            x - 2 * dx,
            y - 2 * dy,
            u8::try_from(op1).map_err(|_| TileSyntaxError::InvalidBitstream)?,
        )?;
    }

    Ok(())
}

fn wide_filter(
    plane: &mut CurrentPlaneMut<'_>,
    x: i32,
    y: i32,
    dx: i32,
    dy: i32,
    log2_size: u8,
) -> Result<(), TileSyntaxError> {
    let n = (1i32 << (log2_size - 1)) - 1;
    let count = usize::try_from(2 * n).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let mut filtered = [0u8; 14];
    for i in -n..n {
        let mut t = i32::from(loop_filter_sample(plane, x + i * dx, y + i * dy)?);
        for j in -n..=n {
            let p = clip3(-(n + 1), n, i + j);
            t += i32::from(loop_filter_sample(plane, x + p * dx, y + p * dy)?);
        }
        let index = usize::try_from(i + n).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        filtered[index] = clip1(round2_i32(t, log2_size));
    }
    for i in -n..n {
        let index = usize::try_from(i + n).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        if index >= count {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        loop_filter_set(plane, x + i * dx, y + i * dy, filtered[index])?;
    }
    Ok(())
}

fn loop_filter_sample(plane: &CurrentPlaneMut<'_>, x: i32, y: i32) -> Result<u8, TileSyntaxError> {
    if plane.width == 0 || plane.height == 0 {
        return Err(TileSyntaxError::InvalidBitstream);
    }
    let max_x = i32::try_from(plane.width - 1).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let max_y = i32::try_from(plane.height - 1).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let x = usize::try_from(clip3(0, max_x, x)).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let y = usize::try_from(clip3(0, max_y, y)).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    plane.sample_clamped(x, y)
}

fn loop_filter_set(
    plane: &mut CurrentPlaneMut<'_>,
    x: i32,
    y: i32,
    value: u8,
) -> Result<(), TileSyntaxError> {
    if x < 0 || y < 0 {
        return Ok(());
    }
    let x = usize::try_from(x).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let y = usize::try_from(y).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    plane.set_visible(x, y, value)
}

fn abs_diff(a: u8, b: u8) -> i32 {
    (i32::from(a) - i32::from(b)).abs()
}

fn filter4_clamp(value: i32) -> i32 {
    clip3(-128, 127, value)
}

fn above_with_left(edges: &IntraPredictionEdges, index: isize) -> u8 {
    if index < 0 {
        edges.above_left
    } else {
        edges.above_row[index as usize]
    }
}

fn avg2(a: u8, b: u8) -> u8 {
    ((u16::from(a) + u16::from(b) + 1) >> 1) as u8
}

fn avg3(a: u8, b: u8, c: u8) -> u8 {
    ((u16::from(a) + 2 * u16::from(b) + u16::from(c) + 2) >> 2) as u8
}

fn avg3_last_weighted(a: u8, b: u8) -> u8 {
    ((u16::from(a) + 3 * u16::from(b) + 2) >> 2) as u8
}

fn clip1(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

fn round2_i32(value: i32, bits: u8) -> i32 {
    (value + (1 << (bits - 1))) >> bits
}

const fn transform_width(tx_size: TxSize) -> usize {
    4 << tx_size.index()
}

fn tx_width_log2(size: usize) -> u32 {
    match size {
        4 => 2,
        8 => 3,
        16 => 4,
        32 => 5,
        _ => 0,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DecodedBlockInfo {
    skip: bool,
    tx_size: TxSize,
    y_mode: u8,
    uv_mode: IntraMode,
    sub_modes: [IntraMode; 4],
    segment_id: u8,
    is_inter: bool,
    ref_frames: [u8; REF_LISTS],
    interp_filter: u8,
    block_mvs: [[MotionVector; SUB_BLOCKS]; REF_LISTS],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NeighborModeInfo {
    skip: bool,
    tx_size: TxSize,
    y_mode: u8,
    sub_modes: [IntraMode; 4],
    ref_frames: [u8; REF_LISTS],
    interp_filter: u8,
    mvs: [MotionVector; REF_LISTS],
}

impl NeighborModeInfo {
    const DEFAULT: Self = Self {
        skip: false,
        tx_size: TxSize::Tx4x4,
        y_mode: IntraMode::Dc as u8,
        sub_modes: [IntraMode::Dc; 4],
        ref_frames: [INTRA_FRAME, NONE_FRAME],
        interp_filter: SWITCHABLE_FILTER_SENTINEL,
        mvs: [MotionVector::ZERO; REF_LISTS],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CandidateModeInfo {
    y_mode: u8,
    ref_frames: [u8; REF_LISTS],
    interp_filter: u8,
    mvs: [MotionVector; REF_LISTS],
    sub_mvs: [[MotionVector; SUB_BLOCKS]; REF_LISTS],
}

impl From<NeighborModeInfo> for CandidateModeInfo {
    fn from(info: NeighborModeInfo) -> Self {
        Self {
            y_mode: info.y_mode,
            ref_frames: info.ref_frames,
            interp_filter: info.interp_filter,
            mvs: info.mvs,
            sub_mvs: [[info.mvs[0]; SUB_BLOCKS], [info.mvs[1]; SUB_BLOCKS]],
        }
    }
}

impl From<StoredModeInfo> for CandidateModeInfo {
    fn from(info: StoredModeInfo) -> Self {
        Self {
            y_mode: info.y_mode,
            ref_frames: info.ref_frames,
            interp_filter: SWITCHABLE_FILTER_SENTINEL,
            mvs: info.mvs,
            sub_mvs: info.sub_mvs,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StoredModeInfo {
    valid: bool,
    skip: bool,
    tx_size: TxSize,
    // Current frame's SegmentIds entry, used by loop filtering and other
    // current-frame syntax consumers.
    segment_id: u8,
    // Persistent PrevSegmentIds entry after this frame. This can differ from
    // segment_id when segmentation_update_map is false: current block syntax
    // uses get_segment_id(), but the saved segmentation map is not refreshed.
    segment_map_id: u8,
    mi_size: BlockSize,
    y_mode: u8,
    ref_frames: [u8; REF_LISTS],
    mvs: [MotionVector; REF_LISTS],
    sub_mvs: [[MotionVector; SUB_BLOCKS]; REF_LISTS],
}

const STORED_MODE_INFO_VALID_OFFSET: usize = 0;
const STORED_MODE_INFO_Y_MODE_OFFSET: usize = 1;
const STORED_MODE_INFO_REF_FRAMES_OFFSET: usize = 2;
const STORED_MODE_INFO_MVS_OFFSET: usize = 4;
const STORED_MODE_INFO_SUB_MVS_OFFSET: usize = 12;
const STORED_MODE_INFO_SKIP_OFFSET: usize = 44;
const STORED_MODE_INFO_TX_SIZE_OFFSET: usize = 45;
const STORED_MODE_INFO_SEGMENT_ID_OFFSET: usize = 46;
const STORED_MODE_INFO_MI_SIZE_OFFSET: usize = 47;
const STORED_MODE_INFO_SEGMENT_MAP_ID_OFFSET: usize = 48;

pub(crate) const STORED_MODE_INFO_BYTES: usize = 49;

pub(crate) fn mode_info_byte_len(mi_count: usize) -> Option<usize> {
    mi_count.checked_mul(STORED_MODE_INFO_BYTES)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModeInfoView<'a> {
    data: &'a [u8],
}

impl<'a> ModeInfoView<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Result<Self, DecodeError> {
        if !data.len().is_multiple_of(STORED_MODE_INFO_BYTES) {
            return Err(DecodeError::InvalidConfig);
        }
        Ok(Self { data })
    }

    fn get(self, index: usize) -> Result<StoredModeInfo, TileSyntaxError> {
        let start = mode_info_offset(index)?;
        let end = start
            .checked_add(STORED_MODE_INFO_BYTES)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let entry = self
            .data
            .get(start..end)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        decode_stored_mode_info(entry)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ModeInfoViewMut<'a> {
    data: &'a mut [u8],
}

impl<'a> ModeInfoViewMut<'a> {
    pub(crate) fn new(data: &'a mut [u8]) -> Result<Self, DecodeError> {
        if !data.len().is_multiple_of(STORED_MODE_INFO_BYTES) {
            return Err(DecodeError::InvalidConfig);
        }
        Ok(Self { data })
    }

    pub(crate) fn clear(&mut self) {
        self.data.fill(0);
    }

    fn as_view(&self) -> ModeInfoView<'_> {
        ModeInfoView { data: self.data }
    }

    fn reborrow(&mut self) -> ModeInfoViewMut<'_> {
        ModeInfoViewMut { data: self.data }
    }

    fn get(&self, index: usize) -> Result<StoredModeInfo, TileSyntaxError> {
        self.as_view().get(index)
    }

    fn set(&mut self, index: usize, info: StoredModeInfo) -> Result<(), TileSyntaxError> {
        let start = mode_info_offset(index)?;
        let end = start
            .checked_add(STORED_MODE_INFO_BYTES)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let entry = self
            .data
            .get_mut(start..end)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        encode_stored_mode_info(info, entry);
        Ok(())
    }
}

pub(crate) struct FrameModeBuffers<'a> {
    use_prev_frame_mvs: bool,
    prev_frame_modes: Option<ModeInfoView<'a>>,
    current_frame_modes: Option<ModeInfoViewMut<'a>>,
}

impl<'a> FrameModeBuffers<'a> {
    #[cfg(feature = "wasm-tests")]
    pub(crate) const fn current(current_frame_modes: Option<ModeInfoViewMut<'a>>) -> Self {
        Self {
            use_prev_frame_mvs: false,
            prev_frame_modes: None,
            current_frame_modes,
        }
    }

    pub(crate) const fn new(
        use_prev_frame_mvs: bool,
        prev_frame_modes: Option<ModeInfoView<'a>>,
        current_frame_modes: Option<ModeInfoViewMut<'a>>,
    ) -> Self {
        Self {
            use_prev_frame_mvs,
            prev_frame_modes,
            current_frame_modes,
        }
    }

    fn for_tile(&mut self) -> FrameModeBuffers<'_> {
        FrameModeBuffers {
            use_prev_frame_mvs: self.use_prev_frame_mvs,
            prev_frame_modes: self.prev_frame_modes,
            current_frame_modes: self
                .current_frame_modes
                .as_mut()
                .map(ModeInfoViewMut::reborrow),
        }
    }
}

fn mode_info_offset(index: usize) -> Result<usize, TileSyntaxError> {
    index
        .checked_mul(STORED_MODE_INFO_BYTES)
        .ok_or(TileSyntaxError::InvalidBitstream)
}

fn decode_stored_mode_info(bytes: &[u8]) -> Result<StoredModeInfo, TileSyntaxError> {
    if bytes.len() != STORED_MODE_INFO_BYTES {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    let valid = bytes[STORED_MODE_INFO_VALID_OFFSET] != 0;
    let y_mode = bytes[STORED_MODE_INFO_Y_MODE_OFFSET];
    let tx_size = TxSize::from_raw(bytes[STORED_MODE_INFO_TX_SIZE_OFFSET])
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let mi_size = BlockSize::from_raw(bytes[STORED_MODE_INFO_MI_SIZE_OFFSET])
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let ref_frames = [
        bytes[STORED_MODE_INFO_REF_FRAMES_OFFSET],
        bytes[STORED_MODE_INFO_REF_FRAMES_OFFSET + 1],
    ];

    let mut mvs = [MotionVector::ZERO; REF_LISTS];
    let mut offset = STORED_MODE_INFO_MVS_OFFSET;
    for mv in &mut mvs {
        *mv = decode_motion_vector(bytes, offset)?;
        offset += 4;
    }

    let mut sub_mvs = [[MotionVector::ZERO; SUB_BLOCKS]; REF_LISTS];
    offset = STORED_MODE_INFO_SUB_MVS_OFFSET;
    for ref_mvs in &mut sub_mvs {
        for mv in ref_mvs {
            *mv = decode_motion_vector(bytes, offset)?;
            offset += 4;
        }
    }

    Ok(StoredModeInfo {
        valid,
        skip: bytes[STORED_MODE_INFO_SKIP_OFFSET] != 0,
        tx_size,
        segment_id: bytes[STORED_MODE_INFO_SEGMENT_ID_OFFSET],
        segment_map_id: bytes[STORED_MODE_INFO_SEGMENT_MAP_ID_OFFSET],
        mi_size,
        y_mode,
        ref_frames,
        mvs,
        sub_mvs,
    })
}

fn encode_stored_mode_info(info: StoredModeInfo, bytes: &mut [u8]) {
    debug_assert_eq!(bytes.len(), STORED_MODE_INFO_BYTES);
    bytes.fill(0);
    bytes[STORED_MODE_INFO_VALID_OFFSET] = u8::from(info.valid);
    bytes[STORED_MODE_INFO_Y_MODE_OFFSET] = info.y_mode;
    bytes[STORED_MODE_INFO_SKIP_OFFSET] = u8::from(info.skip);
    bytes[STORED_MODE_INFO_TX_SIZE_OFFSET] = info.tx_size as u8;
    bytes[STORED_MODE_INFO_SEGMENT_ID_OFFSET] = info.segment_id;
    bytes[STORED_MODE_INFO_SEGMENT_MAP_ID_OFFSET] = info.segment_map_id;
    bytes[STORED_MODE_INFO_MI_SIZE_OFFSET] = info.mi_size as u8;
    bytes[STORED_MODE_INFO_REF_FRAMES_OFFSET] = info.ref_frames[0];
    bytes[STORED_MODE_INFO_REF_FRAMES_OFFSET + 1] = info.ref_frames[1];

    let mut offset = STORED_MODE_INFO_MVS_OFFSET;
    for mv in info.mvs {
        encode_motion_vector(mv, bytes, offset);
        offset += 4;
    }

    offset = STORED_MODE_INFO_SUB_MVS_OFFSET;
    for ref_mvs in info.sub_mvs {
        for mv in ref_mvs {
            encode_motion_vector(mv, bytes, offset);
            offset += 4;
        }
    }
}

fn decode_motion_vector(bytes: &[u8], offset: usize) -> Result<MotionVector, TileSyntaxError> {
    let row = read_i16_le(bytes, offset)?;
    let col_offset = offset
        .checked_add(2)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let col = read_i16_le(bytes, col_offset)?;
    Ok(MotionVector { row, col })
}

fn encode_motion_vector(mv: MotionVector, bytes: &mut [u8], offset: usize) {
    write_i16_le(mv.row, bytes, offset);
    write_i16_le(mv.col, bytes, offset + 2);
}

fn read_i16_le(bytes: &[u8], offset: usize) -> Result<i16, TileSyntaxError> {
    let end = offset
        .checked_add(2)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let raw = bytes
        .get(offset..end)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    Ok(i16::from_le_bytes([raw[0], raw[1]]))
}

fn write_i16_le(value: i16, bytes: &mut [u8], offset: usize) {
    let raw = value.to_le_bytes();
    bytes[offset] = raw[0];
    bytes[offset + 1] = raw[1];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MotionVector {
    row: i16,
    col: i16,
}

impl MotionVector {
    const ZERO: Self = Self { row: 0, col: 0 };

    fn add(self, other: Self) -> Result<Self, TileSyntaxError> {
        Self::new(
            i32::from(self.row)
                .checked_add(i32::from(other.row))
                .ok_or(TileSyntaxError::InvalidBitstream)?,
            i32::from(self.col)
                .checked_add(i32::from(other.col))
                .ok_or(TileSyntaxError::InvalidBitstream)?,
        )
    }

    fn new(row: i32, col: i32) -> Result<Self, TileSyntaxError> {
        Ok(Self {
            row: i16::try_from(row).map_err(|_| TileSyntaxError::InvalidBitstream)?,
            col: i16::try_from(col).map_err(|_| TileSyntaxError::InvalidBitstream)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MvRefState {
    ref_list: [MotionVector; MAX_MV_REF_CANDIDATES],
    nearest: MotionVector,
    near: MotionVector,
    best: MotionVector,
    mode_context: usize,
    count: usize,
}

impl MvRefState {
    const DEFAULT: Self = Self {
        ref_list: [MotionVector::ZERO; MAX_MV_REF_CANDIDATES],
        nearest: MotionVector::ZERO,
        near: MotionVector::ZERO,
        best: MotionVector::ZERO,
        mode_context: 0,
        count: 0,
    };

    fn add_mv_ref(&mut self, mv: MotionVector) {
        if self.count >= MAX_MV_REF_CANDIDATES {
            return;
        }
        if self.count > 0 && self.ref_list[0] == mv {
            return;
        }
        self.ref_list[self.count] = mv;
        self.count += 1;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TileModeContexts {
    above_partition: [u8; MAX_MI_COLS],
    above_mode: [NeighborModeInfo; MAX_MI_COLS],
    above_nonzero: [[u8; MAX_4X4_COLS]; PLANES],
    above_seg_pred: [u8; MAX_MI_COLS],
    left_partition: [u8; MI_BLOCK_64],
    left_mode: [NeighborModeInfo; MI_BLOCK_64],
    left_nonzero: [[u8; MI_BLOCK_64 * 2]; PLANES],
    left_seg_pred: [u8; MI_BLOCK_64],
    mi_cols: usize,
    partition_cols: usize,
}

impl TileModeContexts {
    fn new(mi_cols: usize) -> Result<Self, TileSyntaxError> {
        let partition_cols = mi_cols
            .checked_add(MI_BLOCK_64 - 1)
            .map(|cols| (cols / MI_BLOCK_64) * MI_BLOCK_64)
            .ok_or(TileSyntaxError::ResourceLimit)?;
        if mi_cols == 0 {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        if mi_cols > MAX_MI_COLS || partition_cols > MAX_MI_COLS {
            return Err(TileSyntaxError::ResourceLimit);
        }

        Ok(Self {
            above_partition: [0; MAX_MI_COLS],
            above_mode: [NeighborModeInfo::DEFAULT; MAX_MI_COLS],
            above_nonzero: [[0; MAX_4X4_COLS]; PLANES],
            above_seg_pred: [0; MAX_MI_COLS],
            left_partition: [0; MI_BLOCK_64],
            left_mode: [NeighborModeInfo::DEFAULT; MI_BLOCK_64],
            left_nonzero: [[0; MI_BLOCK_64 * 2]; PLANES],
            left_seg_pred: [0; MI_BLOCK_64],
            mi_cols,
            partition_cols,
        })
    }

    fn clear_left_context(&mut self) {
        self.left_partition = [0; MI_BLOCK_64];
        self.left_mode = [NeighborModeInfo::DEFAULT; MI_BLOCK_64];
        self.left_nonzero = [[0; MI_BLOCK_64 * 2]; PLANES];
        self.left_seg_pred = [0; MI_BLOCK_64];
    }

    fn partition_context(
        &self,
        left_row_base: usize,
        row: usize,
        col: usize,
        block_size: BlockSize,
    ) -> Result<usize, TileSyntaxError> {
        let num_8x8 = usize::from(block_size.num_8x8_wide());
        let row_offset = row_offset(left_row_base, row)?;
        let mut above = 0u8;
        let mut left = 0u8;
        for i in 0..num_8x8 {
            above |= *self
                .above_partition
                .get(
                    col.checked_add(i)
                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                )
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            left |= *self
                .left_partition
                .get(
                    row_offset
                        .checked_add(i)
                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                )
                .ok_or(TileSyntaxError::InvalidBitstream)?;
        }

        let bsl = block_size.mi_width_log2();
        let boffset = BlockSize::Block64x64.mi_width_log2() - bsl;
        let above = usize::from((above & (1 << boffset)) != 0);
        let left = usize::from((left & (1 << boffset)) != 0);
        let ctx = usize::from(bsl) * 4 + left * 2 + above;
        if ctx >= PARTITION_CONTEXTS {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        Ok(ctx)
    }

    fn update_partition_context(
        &mut self,
        left_row_base: usize,
        row: usize,
        col: usize,
        block_size: BlockSize,
        subsize: BlockSize,
    ) -> Result<(), TileSyntaxError> {
        let num_8x8 = usize::from(block_size.num_8x8_wide());
        let row_offset = row_offset(left_row_base, row)?;
        let above_value = 15 >> subsize.b_width_log2();
        let left_value = 15 >> subsize.b_height_log2();

        for i in 0..num_8x8 {
            let above_index = col
                .checked_add(i)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            if above_index >= self.partition_cols {
                return Err(TileSyntaxError::InvalidBitstream);
            }
            self.above_partition[above_index] = above_value;

            let left_index = row_offset
                .checked_add(i)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            if left_index >= self.left_partition.len() {
                return Err(TileSyntaxError::InvalidBitstream);
            }
            self.left_partition[left_index] = left_value;
        }

        Ok(())
    }

    fn skip_context(
        &self,
        left_row_base: usize,
        row: usize,
        col: usize,
        avail_u: bool,
        avail_l: bool,
    ) -> Result<usize, TileSyntaxError> {
        let mut ctx = 0usize;
        if avail_u && self.above_mode(col)?.skip {
            ctx += 1;
        }
        if avail_l && self.left_mode(left_row_base, row)?.skip {
            ctx += 1;
        }
        Ok(ctx)
    }

    fn seg_pred_context(
        &self,
        left_row_base: usize,
        row: usize,
        col: usize,
    ) -> Result<usize, TileSyntaxError> {
        let row_offset = row_offset(left_row_base, row)?;
        let left = *self
            .left_seg_pred
            .get(row_offset)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let above = *self
            .above_seg_pred
            .get(col)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        Ok(usize::from(left) + usize::from(above))
    }

    fn update_seg_pred_context(
        &mut self,
        left_row_base: usize,
        row: usize,
        col: usize,
        block_size: BlockSize,
        seg_id_predicted: bool,
    ) -> Result<(), TileSyntaxError> {
        let value = u8::from(seg_id_predicted);
        let width = usize::from(block_size.num_8x8_wide());
        let height = usize::from(block_size.num_8x8_high());
        let row_offset = row_offset(left_row_base, row)?;

        for y in 0..height {
            let left_index = row_offset
                .checked_add(y)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            if left_index < self.left_seg_pred.len() {
                self.left_seg_pred[left_index] = value;
            }
        }

        for x in 0..width {
            let above_index = col
                .checked_add(x)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            if above_index < self.mi_cols {
                self.above_seg_pred[above_index] = value;
            }
        }

        Ok(())
    }

    fn tx_size_context(
        &self,
        left_row_base: usize,
        row: usize,
        col: usize,
        max_tx_size: TxSize,
        avail_u: bool,
        avail_l: bool,
    ) -> Result<usize, TileSyntaxError> {
        let mut above = max_tx_size.index();
        let mut left = max_tx_size.index();
        if avail_u {
            let above_mode = self.above_mode(col)?;
            if !above_mode.skip {
                above = above_mode.tx_size.index();
            }
        }
        if avail_l {
            let left_mode = self.left_mode(left_row_base, row)?;
            if !left_mode.skip {
                left = left_mode.tx_size.index();
            }
        }
        if !avail_l {
            left = above;
        }
        if !avail_u {
            above = left;
        }
        let ctx = usize::from((above + left) > max_tx_size.index());
        Ok(ctx)
    }

    fn coef_context(
        &self,
        left_row_base: usize,
        plane: usize,
        start: (usize, usize),
        tx_size: TxSize,
        frame_mis: (usize, usize),
    ) -> Result<usize, TileSyntaxError> {
        let (start_x, start_y) = start;
        let (mi_rows, mi_cols) = frame_mis;
        let sx = subsampling_x(plane);
        let sy = subsampling_y(plane);
        let max_x = mi_cols
            .checked_mul(2)
            .map(|value| value >> sx)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let max_y = mi_rows
            .checked_mul(2)
            .map(|value| value >> sy)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let numpts = 1usize << tx_size.index();
        let x4 = start_x >> 2;
        let y4 = start_y >> 2;
        let mut above = 0u8;
        let mut left = 0u8;

        for i in 0..numpts {
            let above_index = x4.checked_add(i).ok_or(TileSyntaxError::InvalidBitstream)?;
            if above_index < max_x {
                above |= *self
                    .above_nonzero
                    .get(plane)
                    .and_then(|plane_context| plane_context.get(above_index))
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
            }

            let y_index = y4.checked_add(i).ok_or(TileSyntaxError::InvalidBitstream)?;
            if y_index < max_y {
                let left_index = local_4x4_index(left_row_base, plane, y_index)?;
                left |= *self
                    .left_nonzero
                    .get(plane)
                    .and_then(|plane_context| plane_context.get(left_index))
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
            }
        }

        Ok(usize::from(above != 0) + usize::from(left != 0))
    }

    fn update_nonzero_context(
        &mut self,
        left_row_base: usize,
        plane: usize,
        start_x: usize,
        start_y: usize,
        step: usize,
        nonzero: bool,
    ) -> Result<(), TileSyntaxError> {
        let value = u8::from(nonzero);
        let x4 = start_x >> 2;
        let y4 = start_y >> 2;

        for i in 0..step {
            let above_index = x4.checked_add(i).ok_or(TileSyntaxError::InvalidBitstream)?;
            *self
                .above_nonzero
                .get_mut(plane)
                .and_then(|plane_context| plane_context.get_mut(above_index))
                .ok_or(TileSyntaxError::InvalidBitstream)? = value;

            let y_index = y4.checked_add(i).ok_or(TileSyntaxError::InvalidBitstream)?;
            let left_index = local_4x4_index(left_row_base, plane, y_index)?;
            *self
                .left_nonzero
                .get_mut(plane)
                .and_then(|plane_context| plane_context.get_mut(left_index))
                .ok_or(TileSyntaxError::InvalidBitstream)? = value;
        }

        Ok(())
    }

    fn update_mode_context(
        &mut self,
        left_row_base: usize,
        row: usize,
        col: usize,
        block_size: BlockSize,
        block: DecodedBlockInfo,
    ) -> Result<(), TileSyntaxError> {
        let _ = block.y_mode;
        let _ = block.uv_mode;
        let _ = block.segment_id;
        let mut mvs = [MotionVector::ZERO; REF_LISTS];
        for (ref_list, mv) in mvs.iter_mut().enumerate() {
            *mv = block.block_mvs[ref_list][3];
        }
        let mode_info = NeighborModeInfo {
            skip: block.skip,
            tx_size: block.tx_size,
            y_mode: block.y_mode,
            sub_modes: block.sub_modes,
            ref_frames: block.ref_frames,
            interp_filter: block.interp_filter,
            mvs,
        };
        let width = usize::from(block_size.num_8x8_wide());
        let height = usize::from(block_size.num_8x8_high());
        let row_offset = row_offset(left_row_base, row)?;

        for y in 0..height {
            let left_index = row_offset
                .checked_add(y)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            if left_index < self.left_mode.len() {
                self.left_mode[left_index] = mode_info;
            }
        }

        for x in 0..width {
            let above_index = col
                .checked_add(x)
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            if above_index < self.mi_cols {
                self.above_mode[above_index] = mode_info;
            }
        }

        Ok(())
    }

    fn above_mode(&self, col: usize) -> Result<NeighborModeInfo, TileSyntaxError> {
        self.above_mode
            .get(col)
            .copied()
            .ok_or(TileSyntaxError::InvalidBitstream)
    }

    fn left_mode(
        &self,
        left_row_base: usize,
        row: usize,
    ) -> Result<NeighborModeInfo, TileSyntaxError> {
        self.left_mode
            .get(row_offset(left_row_base, row)?)
            .copied()
            .ok_or(TileSyntaxError::InvalidBitstream)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum PartitionType {
    None = 0,
    Horz = 1,
    Vert = 2,
    Split = 3,
}

impl PartitionType {
    fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::None),
            1 => Some(Self::Horz),
            2 => Some(Self::Vert),
            3 => Some(Self::Split),
            _ => None,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u8)]
enum BlockSize {
    Block4x4 = 0,
    Block4x8 = 1,
    Block8x4 = 2,
    Block8x8 = 3,
    Block8x16 = 4,
    Block16x8 = 5,
    Block16x16 = 6,
    Block16x32 = 7,
    Block32x16 = 8,
    Block32x32 = 9,
    Block32x64 = 10,
    Block64x32 = 11,
    Block64x64 = 12,
}

impl BlockSize {
    fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Block4x4),
            1 => Some(Self::Block4x8),
            2 => Some(Self::Block8x4),
            3 => Some(Self::Block8x8),
            4 => Some(Self::Block8x16),
            5 => Some(Self::Block16x8),
            6 => Some(Self::Block16x16),
            7 => Some(Self::Block16x32),
            8 => Some(Self::Block32x16),
            9 => Some(Self::Block32x32),
            10 => Some(Self::Block32x64),
            11 => Some(Self::Block64x32),
            12 => Some(Self::Block64x64),
            _ => None,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }

    fn subsize(self, partition: PartitionType) -> Result<Self, TileSyntaxError> {
        SUBSIZE_LOOKUP[partition.index()][self.index()].ok_or(TileSyntaxError::InvalidBitstream)
    }

    const fn is_at_least_8x8(self) -> bool {
        self as u8 >= Self::Block8x8 as u8
    }

    const fn b_width_log2(self) -> u8 {
        B_WIDTH_LOG2_LOOKUP[self.index()]
    }

    const fn b_height_log2(self) -> u8 {
        B_HEIGHT_LOG2_LOOKUP[self.index()]
    }

    const fn num_4x4_wide(self) -> u8 {
        NUM_4X4_BLOCKS_WIDE_LOOKUP[self.index()]
    }

    const fn num_4x4_high(self) -> u8 {
        NUM_4X4_BLOCKS_HIGH_LOOKUP[self.index()]
    }

    const fn mi_width_log2(self) -> u8 {
        MI_WIDTH_LOG2_LOOKUP[self.index()]
    }

    const fn num_8x8_wide(self) -> u8 {
        NUM_8X8_BLOCKS_WIDE_LOOKUP[self.index()]
    }

    const fn num_8x8_high(self) -> u8 {
        NUM_8X8_BLOCKS_HIGH_LOOKUP[self.index()]
    }

    const fn max_tx_size(self) -> TxSize {
        MAX_TXSIZE_LOOKUP[self.index()]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum IntraMode {
    Dc = 0,
    V = 1,
    H = 2,
    D45 = 3,
    D135 = 4,
    D117 = 5,
    D153 = 6,
    D207 = 7,
    D63 = 8,
    Tm = 9,
}

impl IntraMode {
    fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Dc),
            1 => Some(Self::V),
            2 => Some(Self::H),
            3 => Some(Self::D45),
            4 => Some(Self::D135),
            5 => Some(Self::D117),
            6 => Some(Self::D153),
            7 => Some(Self::D207),
            8 => Some(Self::D63),
            9 => Some(Self::Tm),
            _ => None,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }

    const fn raw(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum InterMode {
    Nearest = 0,
    Near = 1,
    Zero = 2,
    New = 3,
}

impl InterMode {
    fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Nearest),
            1 => Some(Self::Near),
            2 => Some(Self::Zero),
            3 => Some(Self::New),
            _ => None,
        }
    }

    fn from_y_mode(y_mode: u8) -> Result<Self, TileSyntaxError> {
        y_mode
            .checked_sub(NEARESTMV)
            .and_then(Self::from_raw)
            .ok_or(TileSyntaxError::InvalidBitstream)
    }

    const fn y_mode(self) -> u8 {
        NEARESTMV + self as u8
    }

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u8)]
enum TxSize {
    Tx4x4 = 0,
    Tx8x8 = 1,
    Tx16x16 = 2,
    Tx32x32 = 3,
}

impl TxSize {
    fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Tx4x4),
            1 => Some(Self::Tx8x8),
            2 => Some(Self::Tx16x16),
            3 => Some(Self::Tx32x32),
            _ => None,
        }
    }

    const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Tx4x4,
            1 => Self::Tx8x8,
            2 => Self::Tx16x16,
            _ => Self::Tx32x32,
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum TxType {
    DctDct = 0,
    AdstDct = 1,
    DctAdst = 2,
    AdstAdst = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum CoefToken {
    Zero = 0,
    One = 1,
    Two = 2,
    Three = 3,
    Four = 4,
    DctValCategory1 = 5,
    DctValCategory2 = 6,
    DctValCategory3 = 7,
    DctValCategory4 = 8,
    DctValCategory5 = 9,
    DctValCategory6 = 10,
}

impl CoefToken {
    const fn index(self) -> usize {
        self as usize
    }
}

fn mi_size(pixels: u32) -> Result<usize, TileSyntaxError> {
    let mis = pixels
        .checked_add(MI_SIZE_PIXELS - 1)
        .map(|value| value / MI_SIZE_PIXELS)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    usize::try_from(mis).map_err(|_| TileSyntaxError::InvalidBitstream)
}

const fn bool_index(value: bool) -> usize {
    if value { 1 } else { 0 }
}

fn increment_count(count: &mut u32) {
    *count = count.saturating_add(1);
}

fn row_offset(left_row_base: usize, row: usize) -> Result<usize, TileSyntaxError> {
    row.checked_sub(left_row_base)
        .ok_or(TileSyntaxError::InvalidBitstream)
}

fn local_4x4_index(
    left_row_base: usize,
    plane: usize,
    y4: usize,
) -> Result<usize, TileSyntaxError> {
    let sy = subsampling_y(plane);
    let base_y4 = left_row_base
        .checked_mul(2)
        .map(|value| value >> sy)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    y4.checked_sub(base_y4)
        .ok_or(TileSyntaxError::InvalidBitstream)
}

const fn subsampling_x(plane: usize) -> usize {
    if plane > 0 { SUBSAMPLING_X } else { 0 }
}

const fn subsampling_y(plane: usize) -> usize {
    if plane > 0 { SUBSAMPLING_Y } else { 0 }
}

fn get_uv_tx_size(mi_size: BlockSize, tx_size: TxSize) -> Result<TxSize, TileSyntaxError> {
    if mi_size < BlockSize::Block8x8 {
        return Ok(TxSize::Tx4x4);
    }
    let plane_size = get_plane_block_size(mi_size, 1)?;
    Ok(TxSize::from_index(core::cmp::min(
        tx_size.index(),
        plane_size.max_tx_size().index(),
    )))
}

fn get_plane_block_size(subsize: BlockSize, plane: usize) -> Result<BlockSize, TileSyntaxError> {
    let sub_x = subsampling_x(plane);
    let sub_y = subsampling_y(plane);
    SS_SIZE_LOOKUP
        .get(subsize.index())
        .and_then(|by_x| by_x.get(sub_x))
        .and_then(|by_y| by_y.get(sub_y))
        .copied()
        .flatten()
        .ok_or(TileSyntaxError::InvalidBitstream)
}

fn size_group(block_size: BlockSize) -> usize {
    SIZE_GROUP_LOOKUP[block_size.index()]
}

fn fixed_interp_filter(filter: InterpolationFilter) -> Result<u8, TileSyntaxError> {
    match filter {
        InterpolationFilter::EightTap => Ok(0),
        InterpolationFilter::EightTapSmooth => Ok(1),
        InterpolationFilter::EightTapSharp => Ok(2),
        InterpolationFilter::Bilinear => Ok(3),
        InterpolationFilter::Switchable => Err(TileSyntaxError::InvalidBitstream),
    }
}

fn reference_frame_raw(reference: InterReferenceFrame) -> u8 {
    match reference {
        InterReferenceFrame::Last => LAST_FRAME,
        InterReferenceFrame::Golden => GOLDEN_FRAME,
        InterReferenceFrame::Altref => ALTREF_FRAME,
    }
}

fn is_intra(info: NeighborModeInfo) -> bool {
    info.ref_frames[0] == INTRA_FRAME
}

fn is_single(info: NeighborModeInfo) -> bool {
    info.ref_frames[1] == NONE_FRAME
}

fn single_ref_p1_context(
    left: NeighborModeInfo,
    above: NeighborModeInfo,
    avail_l: bool,
    avail_u: bool,
) -> usize {
    let left_intra = is_intra(left);
    let above_intra = is_intra(above);
    let left_single = is_single(left);
    let above_single = is_single(above);

    if avail_u && avail_l {
        if above_intra && left_intra {
            2
        } else if left_intra {
            if above_single {
                4 * usize::from(above.ref_frames[0] == LAST_FRAME)
            } else {
                1 + usize::from(
                    above.ref_frames[0] == LAST_FRAME || above.ref_frames[1] == LAST_FRAME,
                )
            }
        } else if above_intra {
            if left_single {
                4 * usize::from(left.ref_frames[0] == LAST_FRAME)
            } else {
                1 + usize::from(
                    left.ref_frames[0] == LAST_FRAME || left.ref_frames[1] == LAST_FRAME,
                )
            }
        } else if above_single && left_single {
            2 * usize::from(above.ref_frames[0] == LAST_FRAME)
                + 2 * usize::from(left.ref_frames[0] == LAST_FRAME)
        } else if !above_single && !left_single {
            1 + usize::from(
                above.ref_frames[0] == LAST_FRAME
                    || above.ref_frames[1] == LAST_FRAME
                    || left.ref_frames[0] == LAST_FRAME
                    || left.ref_frames[1] == LAST_FRAME,
            )
        } else {
            let rfs = if above_single {
                above.ref_frames[0]
            } else {
                left.ref_frames[0]
            };
            let crf1 = if above_single {
                left.ref_frames[0]
            } else {
                above.ref_frames[0]
            };
            let crf2 = if above_single {
                left.ref_frames[1]
            } else {
                above.ref_frames[1]
            };
            if rfs == LAST_FRAME {
                3 + usize::from(crf1 == LAST_FRAME || crf2 == LAST_FRAME)
            } else {
                usize::from(crf1 == LAST_FRAME || crf2 == LAST_FRAME)
            }
        }
    } else if avail_u {
        if above_intra {
            2
        } else if above_single {
            4 * usize::from(above.ref_frames[0] == LAST_FRAME)
        } else {
            1 + usize::from(above.ref_frames[0] == LAST_FRAME || above.ref_frames[1] == LAST_FRAME)
        }
    } else if avail_l {
        if left_intra {
            2
        } else if left_single {
            4 * usize::from(left.ref_frames[0] == LAST_FRAME)
        } else {
            1 + usize::from(left.ref_frames[0] == LAST_FRAME || left.ref_frames[1] == LAST_FRAME)
        }
    } else {
        2
    }
}

fn single_ref_p2_context(
    left: NeighborModeInfo,
    above: NeighborModeInfo,
    avail_l: bool,
    avail_u: bool,
) -> usize {
    let left_intra = is_intra(left);
    let above_intra = is_intra(above);
    let left_single = is_single(left);
    let above_single = is_single(above);

    if avail_u && avail_l {
        if above_intra && left_intra {
            2
        } else if left_intra {
            if above_single {
                if above.ref_frames[0] == LAST_FRAME {
                    3
                } else {
                    4 * usize::from(above.ref_frames[0] == GOLDEN_FRAME)
                }
            } else {
                1 + 2 * usize::from(
                    above.ref_frames[0] == GOLDEN_FRAME || above.ref_frames[1] == GOLDEN_FRAME,
                )
            }
        } else if above_intra {
            if left_single {
                if left.ref_frames[0] == LAST_FRAME {
                    3
                } else {
                    4 * usize::from(left.ref_frames[0] == GOLDEN_FRAME)
                }
            } else {
                1 + 2 * usize::from(
                    left.ref_frames[0] == GOLDEN_FRAME || left.ref_frames[1] == GOLDEN_FRAME,
                )
            }
        } else if above_single && left_single {
            if above.ref_frames[0] == LAST_FRAME && left.ref_frames[0] == LAST_FRAME {
                3
            } else if above.ref_frames[0] == LAST_FRAME {
                4 * usize::from(left.ref_frames[0] == GOLDEN_FRAME)
            } else if left.ref_frames[0] == LAST_FRAME {
                4 * usize::from(above.ref_frames[0] == GOLDEN_FRAME)
            } else {
                2 * usize::from(above.ref_frames[0] == GOLDEN_FRAME)
                    + 2 * usize::from(left.ref_frames[0] == GOLDEN_FRAME)
            }
        } else if !above_single && !left_single {
            if above.ref_frames == left.ref_frames {
                3 * usize::from(
                    above.ref_frames[0] == GOLDEN_FRAME || above.ref_frames[1] == GOLDEN_FRAME,
                )
            } else {
                2
            }
        } else {
            let rfs = if above_single {
                above.ref_frames[0]
            } else {
                left.ref_frames[0]
            };
            let crf1 = if above_single {
                left.ref_frames[0]
            } else {
                above.ref_frames[0]
            };
            let crf2 = if above_single {
                left.ref_frames[1]
            } else {
                above.ref_frames[1]
            };
            if rfs == GOLDEN_FRAME {
                3 + usize::from(crf1 == GOLDEN_FRAME || crf2 == GOLDEN_FRAME)
            } else if rfs == ALTREF_FRAME {
                usize::from(crf1 == GOLDEN_FRAME || crf2 == GOLDEN_FRAME)
            } else {
                1 + 2 * usize::from(crf1 == GOLDEN_FRAME || crf2 == GOLDEN_FRAME)
            }
        }
    } else if avail_u {
        if above_intra || (above.ref_frames[0] == LAST_FRAME && above_single) {
            2
        } else if above_single {
            4 * usize::from(above.ref_frames[0] == GOLDEN_FRAME)
        } else {
            3 * usize::from(
                above.ref_frames[0] == GOLDEN_FRAME || above.ref_frames[1] == GOLDEN_FRAME,
            )
        }
    } else if avail_l {
        if left_intra || (left.ref_frames[0] == LAST_FRAME && left_single) {
            2
        } else if left_single {
            4 * usize::from(left.ref_frames[0] == GOLDEN_FRAME)
        } else {
            3 * usize::from(
                left.ref_frames[0] == GOLDEN_FRAME || left.ref_frames[1] == GOLDEN_FRAME,
            )
        }
    } else {
        2
    }
}

fn comp_ref_context(
    compound: CompoundReferenceSetup,
    left: NeighborModeInfo,
    above: NeighborModeInfo,
    avail_l: bool,
    avail_u: bool,
    sign_biases: [bool; 4],
) -> Result<usize, TileSyntaxError> {
    let fixed_ref = reference_frame_raw(compound.comp_fixed_ref);
    let comp_var_ref = [
        reference_frame_raw(compound.comp_var_ref[0]),
        reference_frame_raw(compound.comp_var_ref[1]),
    ];
    let fix_ref_idx = usize::from(
        *sign_biases
            .get(usize::from(fixed_ref))
            .ok_or(TileSyntaxError::InvalidBitstream)?,
    );
    let var_ref_idx = 1 - fix_ref_idx;
    let left_intra = is_intra(left);
    let above_intra = is_intra(above);
    let left_single = is_single(left);
    let above_single = is_single(above);

    let ctx = if avail_u && avail_l {
        if above_intra && left_intra {
            2
        } else if left_intra {
            if above_single {
                1 + 2 * usize::from(above.ref_frames[0] != comp_var_ref[1])
            } else {
                1 + 2 * usize::from(above.ref_frames[var_ref_idx] != comp_var_ref[1])
            }
        } else if above_intra {
            if left_single {
                1 + 2 * usize::from(left.ref_frames[0] != comp_var_ref[1])
            } else {
                1 + 2 * usize::from(left.ref_frames[var_ref_idx] != comp_var_ref[1])
            }
        } else {
            let vrfa = if above_single {
                above.ref_frames[0]
            } else {
                above.ref_frames[var_ref_idx]
            };
            let vrfl = if left_single {
                left.ref_frames[0]
            } else {
                left.ref_frames[var_ref_idx]
            };
            if vrfa == vrfl && comp_var_ref[1] == vrfa {
                0
            } else if left_single && above_single {
                if (vrfa == fixed_ref && vrfl == comp_var_ref[0])
                    || (vrfl == fixed_ref && vrfa == comp_var_ref[0])
                {
                    4
                } else if vrfa == vrfl {
                    3
                } else {
                    1
                }
            } else if left_single || above_single {
                let vrfc = if left_single { vrfa } else { vrfl };
                let rfs = if above_single { vrfa } else { vrfl };
                if vrfc == comp_var_ref[1] && rfs != comp_var_ref[1] {
                    1
                } else if rfs == comp_var_ref[1] && vrfc != comp_var_ref[1] {
                    2
                } else {
                    4
                }
            } else if vrfa == vrfl {
                4
            } else {
                2
            }
        }
    } else if avail_u {
        if above_intra {
            2
        } else if above_single {
            3 * usize::from(above.ref_frames[0] != comp_var_ref[1])
        } else {
            4 * usize::from(above.ref_frames[var_ref_idx] != comp_var_ref[1])
        }
    } else if avail_l {
        if left_intra {
            2
        } else if left_single {
            3 * usize::from(left.ref_frames[0] != comp_var_ref[1])
        } else {
            4 * usize::from(left.ref_frames[var_ref_idx] != comp_var_ref[1])
        }
    } else {
        2
    };
    Ok(ctx)
}

fn get_sub_block_mv(
    info: CandidateModeInfo,
    ref_list: usize,
    delta_col: i8,
    block: i8,
) -> Result<MotionVector, TileSyntaxError> {
    let idx = if block >= 0 {
        let block = usize::try_from(block).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        *IDX_N_COLUMN_TO_SUBBLOCK
            .get(block)
            .and_then(|row| row.get(usize::from(delta_col == 0)))
            .ok_or(TileSyntaxError::InvalidBitstream)?
    } else {
        3
    };
    info.sub_mvs
        .get(ref_list)
        .and_then(|mvs| mvs.get(idx))
        .copied()
        .ok_or(TileSyntaxError::InvalidBitstream)
}

fn if_same_ref_frame_add_mv(state: &mut MvRefState, info: CandidateModeInfo, ref_frame: u8) {
    for ref_list in 0..REF_LISTS {
        if info.ref_frames[ref_list] == ref_frame {
            state.add_mv_ref(info.mvs[ref_list]);
            return;
        }
    }
}

fn if_same_prev_frame_add_mv(state: &mut MvRefState, info: Option<StoredModeInfo>, ref_frame: u8) {
    let Some(info) = info else {
        return;
    };
    for ref_list in 0..REF_LISTS {
        if info.ref_frames[ref_list] == ref_frame {
            state.add_mv_ref(info.mvs[ref_list]);
            return;
        }
    }
}

fn if_diff_ref_frame_add_mv(
    state: &mut MvRefState,
    info: CandidateModeInfo,
    ref_frame: u8,
    sign_biases: [bool; 4],
) -> Result<(), TileSyntaxError> {
    let same_mvs = info.mvs[0] == info.mvs[1];
    for ref_list in 0..REF_LISTS {
        let candidate_frame = info.ref_frames[ref_list];
        if candidate_frame > INTRA_FRAME
            && candidate_frame != ref_frame
            && (ref_list == 0 || !same_mvs)
        {
            let mut mv = info.mvs[ref_list];
            let candidate_bias = *sign_biases
                .get(usize::from(candidate_frame))
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let target_bias = *sign_biases
                .get(usize::from(ref_frame))
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            if candidate_bias != target_bias {
                mv.row = mv
                    .row
                    .checked_neg()
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                mv.col = mv
                    .col
                    .checked_neg()
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
            }
            state.add_mv_ref(mv);
        }
    }
    Ok(())
}

fn if_diff_prev_frame_add_mv(
    state: &mut MvRefState,
    info: Option<StoredModeInfo>,
    ref_frame: u8,
    sign_biases: [bool; 4],
) -> Result<(), TileSyntaxError> {
    let Some(info) = info else {
        return Ok(());
    };
    let same_mvs = info.mvs[0] == info.mvs[1];
    for ref_list in 0..REF_LISTS {
        let candidate_frame = info.ref_frames[ref_list];
        if candidate_frame > INTRA_FRAME
            && candidate_frame != ref_frame
            && (ref_list == 0 || !same_mvs)
        {
            let mut mv = info.mvs[ref_list];
            let candidate_bias = *sign_biases
                .get(usize::from(candidate_frame))
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            let target_bias = *sign_biases
                .get(usize::from(ref_frame))
                .ok_or(TileSyntaxError::InvalidBitstream)?;
            if candidate_bias != target_bias {
                mv.row = mv
                    .row
                    .checked_neg()
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                mv.col = mv
                    .col
                    .checked_neg()
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
            }
            state.add_mv_ref(mv);
        }
    }
    Ok(())
}

fn lower_mv_precision(component: i32) -> i32 {
    if component & 1 != 0 {
        component + if component > 0 { -1 } else { 1 }
    } else {
        component
    }
}

fn use_mv_hp(mv: MotionVector) -> bool {
    (i32::from(mv.row).abs() >> 3) < COMPANDED_MVREF_THRESH
        && (i32::from(mv.col).abs() >> 3) < COMPANDED_MVREF_THRESH
}

fn clip3(min_value: i32, max_value: i32, value: i32) -> i32 {
    core::cmp::min(core::cmp::max(value, min_value), max_value)
}

fn scan_table(tx_size: TxSize, tx_type: TxType) -> &'static [u16] {
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

fn coef_band_table(tx_size: TxSize) -> &'static [u8] {
    match tx_size {
        TxSize::Tx4x4 => &COEFBAND_4X4,
        TxSize::Tx8x8 => &COEFBAND_8X8,
        TxSize::Tx16x16 => &COEFBAND_16X16,
        TxSize::Tx32x32 => &COEFBAND_32X32,
    }
}

#[inline(always)]
fn read_more_coefs(
    decoder: &mut BoolDecoder<'_>,
    probability_row: &[u8; 3],
    counts: &mut [u32; 2],
) -> Result<bool, TileSyntaxError> {
    let more_coefs = decoder.read_bool(probability_row[0])?;
    increment_count(&mut counts[bool_index(more_coefs)]);
    Ok(more_coefs)
}

#[inline(always)]
fn read_token(
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
fn coef_token_count_index(token: CoefToken) -> usize {
    core::cmp::min(2, token.index())
}

#[inline(always)]
fn read_coef(decoder: &mut BoolDecoder<'_>, token: CoefToken) -> Result<u32, TileSyntaxError> {
    let [cat, num_extra, base] = EXTRA_BITS[token.index()];
    let mut coef = u32::from(base);

    for e in 0..num_extra {
        let probability = CAT_PROBS[usize::from(cat)][usize::from(e)];
        let bit = u32::from(decoder.read_bool(probability)?);
        coef += bit << (u32::from(num_extra) - 1 - u32::from(e));
    }

    Ok(coef)
}

fn coefficient_token_context(
    pos: usize,
    tx_size: TxSize,
    tx_type: TxType,
    token_cache: &[u8; MAX_TX_COEFFS],
) -> Result<usize, TileSyntaxError> {
    let n = 4usize << tx_size.index();
    let i = pos / n;
    let j = pos % n;
    let (nb0, nb1) = if i > 0 && j > 0 {
        let a = (i - 1)
            .checked_mul(n)
            .and_then(|value| value.checked_add(j))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let a2 = i
            .checked_mul(n)
            .and_then(|value| value.checked_add(j - 1))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        match tx_type {
            TxType::DctAdst => (a, a),
            TxType::AdstDct => (a2, a2),
            TxType::DctDct | TxType::AdstAdst => (a, a2),
        }
    } else if i > 0 {
        let a = (i - 1)
            .checked_mul(n)
            .and_then(|value| value.checked_add(j))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        (a, a)
    } else {
        let jm1 = j.checked_sub(1).ok_or(TileSyntaxError::InvalidBitstream)?;
        let a = i
            .checked_mul(n)
            .and_then(|value| value.checked_add(jm1))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
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
fn token_probability(probability_row: &[u8; 3], node: usize) -> Result<u8, TileSyntaxError> {
    if node == 0 {
        Ok(probability_row[1])
    } else if node == 1 {
        Ok(probability_row[2])
    } else {
        pareto(node - 2, probability_row[2])
    }
}

#[inline(always)]
fn pareto(table_index: usize, prob: u8) -> Result<u8, TileSyntaxError> {
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

const fn coef_band_8x8plus<const N: usize>() -> [u8; N] {
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

const COEFBAND_4X4: [u8; 16] = [0, 1, 1, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 5, 5, 5];
const COEFBAND_8X8PLUS_FIRST: [u8; 10] = [0, 1, 1, 2, 2, 2, 3, 3, 3, 3];
const COEFBAND_8X8: [u8; 64] = coef_band_8x8plus::<64>();
const COEFBAND_16X16: [u8; 256] = coef_band_8x8plus::<256>();
const COEFBAND_32X32: [u8; 1024] = coef_band_8x8plus::<1024>();

const ENERGY_CLASS: [u8; 11] = [0, 1, 2, 3, 3, 4, 4, 5, 5, 5, 5];

#[derive(Clone, Copy)]
enum TokenTreeBranch {
    Node(u8),
    Token(CoefToken),
}

const TOKEN_TREE: [[TokenTreeBranch; 2]; 10] = [
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

const MODE_TO_TXFM_MAP: [TxType; INTRA_MODES] = [
    TxType::DctDct,
    TxType::AdstDct,
    TxType::DctAdst,
    TxType::DctDct,
    TxType::AdstAdst,
    TxType::AdstDct,
    TxType::DctAdst,
    TxType::DctAdst,
    TxType::AdstDct,
    TxType::AdstAdst,
];

const SUBPEL_FILTERS: [[[i16; 8]; 16]; 4] = [
    [
        [0, 0, 0, 128, 0, 0, 0, 0],
        [0, 1, -5, 126, 8, -3, 1, 0],
        [-1, 3, -10, 122, 18, -6, 2, 0],
        [-1, 4, -13, 118, 27, -9, 3, -1],
        [-1, 4, -16, 112, 37, -11, 4, -1],
        [-1, 5, -18, 105, 48, -14, 4, -1],
        [-1, 5, -19, 97, 58, -16, 5, -1],
        [-1, 6, -19, 88, 68, -18, 5, -1],
        [-1, 6, -19, 78, 78, -19, 6, -1],
        [-1, 5, -18, 68, 88, -19, 6, -1],
        [-1, 5, -16, 58, 97, -19, 5, -1],
        [-1, 4, -14, 48, 105, -18, 5, -1],
        [-1, 4, -11, 37, 112, -16, 4, -1],
        [-1, 3, -9, 27, 118, -13, 4, -1],
        [0, 2, -6, 18, 122, -10, 3, -1],
        [0, 1, -3, 8, 126, -5, 1, 0],
    ],
    [
        [0, 0, 0, 128, 0, 0, 0, 0],
        [-3, -1, 32, 64, 38, 1, -3, 0],
        [-2, -2, 29, 63, 41, 2, -3, 0],
        [-2, -2, 26, 63, 43, 4, -4, 0],
        [-2, -3, 24, 62, 46, 5, -4, 0],
        [-2, -3, 21, 60, 49, 7, -4, 0],
        [-1, -4, 18, 59, 51, 9, -4, 0],
        [-1, -4, 16, 57, 53, 12, -4, -1],
        [-1, -4, 14, 55, 55, 14, -4, -1],
        [-1, -4, 12, 53, 57, 16, -4, -1],
        [0, -4, 9, 51, 59, 18, -4, -1],
        [0, -4, 7, 49, 60, 21, -3, -2],
        [0, -4, 5, 46, 62, 24, -3, -2],
        [0, -4, 4, 43, 63, 26, -2, -2],
        [0, -3, 2, 41, 63, 29, -2, -2],
        [0, -3, 1, 38, 64, 32, -1, -3],
    ],
    [
        [0, 0, 0, 128, 0, 0, 0, 0],
        [-1, 3, -7, 127, 8, -3, 1, 0],
        [-2, 5, -13, 125, 17, -6, 3, -1],
        [-3, 7, -17, 121, 27, -10, 5, -2],
        [-4, 9, -20, 115, 37, -13, 6, -2],
        [-4, 10, -23, 108, 48, -16, 8, -3],
        [-4, 10, -24, 100, 59, -19, 9, -3],
        [-4, 11, -24, 90, 70, -21, 10, -4],
        [-4, 11, -23, 80, 80, -23, 11, -4],
        [-4, 10, -21, 70, 90, -24, 11, -4],
        [-3, 9, -19, 59, 100, -24, 10, -4],
        [-3, 8, -16, 48, 108, -23, 10, -4],
        [-2, 6, -13, 37, 115, -20, 9, -4],
        [-2, 5, -10, 27, 121, -17, 7, -3],
        [-1, 3, -6, 17, 125, -13, 5, -2],
        [0, 1, -3, 8, 127, -7, 3, -1],
    ],
    [
        [0, 0, 0, 128, 0, 0, 0, 0],
        [0, 0, 0, 120, 8, 0, 0, 0],
        [0, 0, 0, 112, 16, 0, 0, 0],
        [0, 0, 0, 104, 24, 0, 0, 0],
        [0, 0, 0, 96, 32, 0, 0, 0],
        [0, 0, 0, 88, 40, 0, 0, 0],
        [0, 0, 0, 80, 48, 0, 0, 0],
        [0, 0, 0, 72, 56, 0, 0, 0],
        [0, 0, 0, 64, 64, 0, 0, 0],
        [0, 0, 0, 56, 72, 0, 0, 0],
        [0, 0, 0, 48, 80, 0, 0, 0],
        [0, 0, 0, 40, 88, 0, 0, 0],
        [0, 0, 0, 32, 96, 0, 0, 0],
        [0, 0, 0, 24, 104, 0, 0, 0],
        [0, 0, 0, 16, 112, 0, 0, 0],
        [0, 0, 0, 8, 120, 0, 0, 0],
    ],
];

const EXTRA_BITS: [[u8; 3]; 11] = [
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

const CAT_PROBS: [[u8; 14]; 7] = [
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

const PARTITION_TREE: [i8; 6] = [
    0,
    2,
    -(PartitionType::Horz as i8),
    4,
    -(PartitionType::Vert as i8),
    -(PartitionType::Split as i8),
];
const COLS_PARTITION_TREE: [i8; 2] = [-(PartitionType::Horz as i8), -(PartitionType::Split as i8)];
const ROWS_PARTITION_TREE: [i8; 2] = [-(PartitionType::Vert as i8), -(PartitionType::Split as i8)];

const INTRA_MODE_TREE: [i8; 18] = [
    0,
    2,
    -(IntraMode::Tm as i8),
    4,
    -(IntraMode::V as i8),
    6,
    8,
    12,
    -(IntraMode::H as i8),
    10,
    -(IntraMode::D135 as i8),
    -(IntraMode::D117 as i8),
    -(IntraMode::D45 as i8),
    14,
    -(IntraMode::D63 as i8),
    16,
    -(IntraMode::D153 as i8),
    -(IntraMode::D207 as i8),
];

const TX_SIZE_32_TREE: [i8; 6] = [
    0,
    2,
    -(TxSize::Tx8x8 as i8),
    4,
    -(TxSize::Tx16x16 as i8),
    -(TxSize::Tx32x32 as i8),
];
const TX_SIZE_16_TREE: [i8; 4] = [0, 2, -(TxSize::Tx8x8 as i8), -(TxSize::Tx16x16 as i8)];
const TX_SIZE_8_TREE: [i8; 2] = [0, -(TxSize::Tx8x8 as i8)];

const INTER_MODE_TREE: [i8; 6] = [
    -(InterMode::Zero as i8),
    2,
    -(InterMode::Nearest as i8),
    4,
    -(InterMode::Near as i8),
    -(InterMode::New as i8),
];

const INTERP_FILTER_TREE: [i8; 4] = [0, 2, -1, -2];

const SEGMENT_TREE: [i8; 14] = [2, 4, 6, 8, 10, 12, 0, -1, -2, -3, -4, -5, -6, -7];

const MV_JOINT_TREE: [i8; 6] = [0, 2, -1, 4, -2, -3];

const MV_CLASS_TREE: [i8; 20] = [
    0, 2, -1, 4, 6, 8, -2, -3, 10, 12, -4, -5, -6, 14, 16, 18, -7, -8, -9, -10,
];

const MV_FR_TREE: [i8; 6] = [0, 2, -1, 4, -2, -3];

const B_WIDTH_LOG2_LOOKUP: [u8; BLOCK_SIZES] = [0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4];
const B_HEIGHT_LOG2_LOOKUP: [u8; BLOCK_SIZES] = [0, 1, 0, 1, 2, 1, 2, 3, 2, 3, 4, 3, 4];
const NUM_4X4_BLOCKS_WIDE_LOOKUP: [u8; BLOCK_SIZES] = [1, 1, 2, 2, 2, 4, 4, 4, 8, 8, 8, 16, 16];
const NUM_4X4_BLOCKS_HIGH_LOOKUP: [u8; BLOCK_SIZES] = [1, 2, 1, 2, 4, 2, 4, 8, 4, 8, 16, 8, 16];
const MI_WIDTH_LOG2_LOOKUP: [u8; BLOCK_SIZES] = [0, 0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3];
const NUM_8X8_BLOCKS_WIDE_LOOKUP: [u8; BLOCK_SIZES] = [1, 1, 1, 1, 1, 2, 2, 2, 4, 4, 4, 8, 8];
const NUM_8X8_BLOCKS_HIGH_LOOKUP: [u8; BLOCK_SIZES] = [1, 1, 1, 1, 2, 1, 2, 4, 2, 4, 8, 4, 8];
const SIZE_GROUP_LOOKUP: [usize; BLOCK_SIZES] = [0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3];
const MAX_TXSIZE_LOOKUP: [TxSize; BLOCK_SIZES] = [
    TxSize::Tx4x4,
    TxSize::Tx4x4,
    TxSize::Tx4x4,
    TxSize::Tx8x8,
    TxSize::Tx8x8,
    TxSize::Tx8x8,
    TxSize::Tx16x16,
    TxSize::Tx16x16,
    TxSize::Tx16x16,
    TxSize::Tx32x32,
    TxSize::Tx32x32,
    TxSize::Tx32x32,
    TxSize::Tx32x32,
];

const SUBSIZE_LOOKUP: [[Option<BlockSize>; BLOCK_SIZES]; PARTITION_TYPES] = [
    [
        Some(BlockSize::Block4x4),
        Some(BlockSize::Block4x8),
        Some(BlockSize::Block8x4),
        Some(BlockSize::Block8x8),
        Some(BlockSize::Block8x16),
        Some(BlockSize::Block16x8),
        Some(BlockSize::Block16x16),
        Some(BlockSize::Block16x32),
        Some(BlockSize::Block32x16),
        Some(BlockSize::Block32x32),
        Some(BlockSize::Block32x64),
        Some(BlockSize::Block64x32),
        Some(BlockSize::Block64x64),
    ],
    [
        None,
        None,
        None,
        Some(BlockSize::Block8x4),
        None,
        None,
        Some(BlockSize::Block16x8),
        None,
        None,
        Some(BlockSize::Block32x16),
        None,
        None,
        Some(BlockSize::Block64x32),
    ],
    [
        None,
        None,
        None,
        Some(BlockSize::Block4x8),
        None,
        None,
        Some(BlockSize::Block8x16),
        None,
        None,
        Some(BlockSize::Block16x32),
        None,
        None,
        Some(BlockSize::Block32x64),
    ],
    [
        None,
        None,
        None,
        Some(BlockSize::Block4x4),
        None,
        None,
        Some(BlockSize::Block8x8),
        None,
        None,
        Some(BlockSize::Block16x16),
        None,
        None,
        Some(BlockSize::Block32x32),
    ],
];

const SS_SIZE_LOOKUP: [[[Option<BlockSize>; 2]; 2]; BLOCK_SIZES] = [
    [[Some(BlockSize::Block4x4), None], [None, None]],
    [
        [Some(BlockSize::Block4x8), Some(BlockSize::Block4x4)],
        [None, None],
    ],
    [
        [Some(BlockSize::Block8x4), None],
        [Some(BlockSize::Block4x4), None],
    ],
    [
        [Some(BlockSize::Block8x8), Some(BlockSize::Block8x4)],
        [Some(BlockSize::Block4x8), Some(BlockSize::Block4x4)],
    ],
    [
        [Some(BlockSize::Block8x16), Some(BlockSize::Block8x8)],
        [None, Some(BlockSize::Block4x8)],
    ],
    [
        [Some(BlockSize::Block16x8), None],
        [Some(BlockSize::Block8x8), Some(BlockSize::Block8x4)],
    ],
    [
        [Some(BlockSize::Block16x16), Some(BlockSize::Block16x8)],
        [Some(BlockSize::Block8x16), Some(BlockSize::Block8x8)],
    ],
    [
        [Some(BlockSize::Block16x32), Some(BlockSize::Block16x16)],
        [None, Some(BlockSize::Block8x16)],
    ],
    [
        [Some(BlockSize::Block32x16), None],
        [Some(BlockSize::Block16x16), Some(BlockSize::Block16x8)],
    ],
    [
        [Some(BlockSize::Block32x32), Some(BlockSize::Block32x16)],
        [Some(BlockSize::Block16x32), Some(BlockSize::Block16x16)],
    ],
    [
        [Some(BlockSize::Block32x64), Some(BlockSize::Block32x32)],
        [None, Some(BlockSize::Block16x32)],
    ],
    [
        [Some(BlockSize::Block64x32), None],
        [Some(BlockSize::Block32x32), Some(BlockSize::Block32x16)],
    ],
    [
        [Some(BlockSize::Block64x64), Some(BlockSize::Block64x32)],
        [Some(BlockSize::Block32x64), Some(BlockSize::Block32x32)],
    ],
];

const MODE_2_COUNTER: [u8; 14] = [9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 0, 0, 3, 1];

const COUNTER_TO_CONTEXT: [u8; 19] = [2, 3, 4, 1, 3, 9, 0, 9, 9, 5, 5, 9, 5, 9, 9, 9, 9, 9, 6];

const IDX_N_COLUMN_TO_SUBBLOCK: [[usize; 2]; SUB_BLOCKS] = [[1, 2], [1, 3], [3, 2], [3, 3]];

const MV_REF_BLOCKS: [[[i8; 2]; MVREF_NEIGHBOURS]; BLOCK_SIZES] = [
    [
        [-1, 0],
        [0, -1],
        [-1, -1],
        [-2, 0],
        [0, -2],
        [-2, -1],
        [-1, -2],
        [-2, -2],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, -1],
        [-2, 0],
        [0, -2],
        [-2, -1],
        [-1, -2],
        [-2, -2],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, -1],
        [-2, 0],
        [0, -2],
        [-2, -1],
        [-1, -2],
        [-2, -2],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, -1],
        [-2, 0],
        [0, -2],
        [-2, -1],
        [-1, -2],
        [-2, -2],
    ],
    [
        [0, -1],
        [-1, 0],
        [1, -1],
        [-1, -1],
        [0, -2],
        [-2, 0],
        [-2, -1],
        [-1, -2],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, 1],
        [-1, -1],
        [-2, 0],
        [0, -2],
        [-1, -2],
        [-2, -1],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, 1],
        [1, -1],
        [-1, -1],
        [-3, 0],
        [0, -3],
        [-3, -3],
    ],
    [
        [0, -1],
        [-1, 0],
        [2, -1],
        [-1, -1],
        [-1, 1],
        [0, -3],
        [-3, 0],
        [-3, -3],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, 2],
        [-1, -1],
        [1, -1],
        [-3, 0],
        [0, -3],
        [-3, -3],
    ],
    [
        [-1, 1],
        [1, -1],
        [-1, 2],
        [2, -1],
        [-1, -1],
        [-3, 0],
        [0, -3],
        [-3, -3],
    ],
    [
        [0, -1],
        [-1, 0],
        [4, -1],
        [-1, 2],
        [-1, -1],
        [0, -3],
        [-3, 0],
        [2, -1],
    ],
    [
        [-1, 0],
        [0, -1],
        [-1, 4],
        [2, -1],
        [-1, -1],
        [-3, 0],
        [0, -3],
        [-1, 2],
    ],
    [
        [-1, 3],
        [3, -1],
        [-1, 4],
        [4, -1],
        [-1, -1],
        [-1, 0],
        [0, -1],
        [-1, 6],
    ],
];

// The local v0.7 draft's probability-selection prose appears to reverse the
// FrameIsIntra condition for partition probabilities. Section 10.4 names these
// fixed tables as intra-frame probabilities, so key/intra syntax uses them here.
const KF_PARTITION_PROBS: [[u8; PARTITION_PROBS]; PARTITION_CONTEXTS] = [
    [158, 97, 94],
    [93, 24, 99],
    [85, 119, 44],
    [62, 59, 67],
    [149, 53, 53],
    [94, 20, 48],
    [83, 53, 24],
    [52, 18, 18],
    [150, 40, 39],
    [78, 12, 26],
    [67, 33, 11],
    [24, 7, 5],
    [174, 35, 49],
    [68, 11, 27],
    [57, 15, 9],
    [12, 3, 3],
];

const KF_Y_MODE_PROBS: [[[u8; INTRA_MODE_PROBS]; INTRA_MODES]; INTRA_MODES] = [
    [
        [137, 30, 42, 148, 151, 207, 70, 52, 91],
        [92, 45, 102, 136, 116, 180, 74, 90, 100],
        [73, 32, 19, 187, 222, 215, 46, 34, 100],
        [91, 30, 32, 116, 121, 186, 93, 86, 94],
        [72, 35, 36, 149, 68, 206, 68, 63, 105],
        [73, 31, 28, 138, 57, 124, 55, 122, 151],
        [67, 23, 21, 140, 126, 197, 40, 37, 171],
        [86, 27, 28, 128, 154, 212, 45, 43, 53],
        [74, 32, 27, 107, 86, 160, 63, 134, 102],
        [59, 67, 44, 140, 161, 202, 78, 67, 119],
    ],
    [
        [63, 36, 126, 146, 123, 158, 60, 90, 96],
        [43, 46, 168, 134, 107, 128, 69, 142, 92],
        [44, 29, 68, 159, 201, 177, 50, 57, 77],
        [58, 38, 76, 114, 97, 172, 78, 133, 92],
        [46, 41, 76, 140, 63, 184, 69, 112, 57],
        [38, 32, 85, 140, 46, 112, 54, 151, 133],
        [39, 27, 61, 131, 110, 175, 44, 75, 136],
        [52, 30, 74, 113, 130, 175, 51, 64, 58],
        [47, 35, 80, 100, 74, 143, 64, 163, 74],
        [36, 61, 116, 114, 128, 162, 80, 125, 82],
    ],
    [
        [82, 26, 26, 171, 208, 204, 44, 32, 105],
        [55, 44, 68, 166, 179, 192, 57, 57, 108],
        [42, 26, 11, 199, 241, 228, 23, 15, 85],
        [68, 42, 19, 131, 160, 199, 55, 52, 83],
        [58, 50, 25, 139, 115, 232, 39, 52, 118],
        [50, 35, 33, 153, 104, 162, 64, 59, 131],
        [44, 24, 16, 150, 177, 202, 33, 19, 156],
        [55, 27, 12, 153, 203, 218, 26, 27, 49],
        [53, 49, 21, 110, 116, 168, 59, 80, 76],
        [38, 72, 19, 168, 203, 212, 50, 50, 107],
    ],
    [
        [103, 26, 36, 129, 132, 201, 83, 80, 93],
        [59, 38, 83, 112, 103, 162, 98, 136, 90],
        [62, 30, 23, 158, 200, 207, 59, 57, 50],
        [67, 30, 29, 84, 86, 191, 102, 91, 59],
        [60, 32, 33, 112, 71, 220, 64, 89, 104],
        [53, 26, 34, 130, 56, 149, 84, 120, 103],
        [53, 21, 23, 133, 109, 210, 56, 77, 172],
        [77, 19, 29, 112, 142, 228, 55, 66, 36],
        [61, 29, 29, 93, 97, 165, 83, 175, 162],
        [47, 47, 43, 114, 137, 181, 100, 99, 95],
    ],
    [
        [69, 23, 29, 128, 83, 199, 46, 44, 101],
        [53, 40, 55, 139, 69, 183, 61, 80, 110],
        [40, 29, 19, 161, 180, 207, 43, 24, 91],
        [60, 34, 19, 105, 61, 198, 53, 64, 89],
        [52, 31, 22, 158, 40, 209, 58, 62, 89],
        [44, 31, 29, 147, 46, 158, 56, 102, 198],
        [35, 19, 12, 135, 87, 209, 41, 45, 167],
        [55, 25, 21, 118, 95, 215, 38, 39, 66],
        [51, 38, 25, 113, 58, 164, 70, 93, 97],
        [47, 54, 34, 146, 108, 203, 72, 103, 151],
    ],
    [
        [64, 19, 37, 156, 66, 138, 49, 95, 133],
        [46, 27, 80, 150, 55, 124, 55, 121, 135],
        [36, 23, 27, 165, 149, 166, 54, 64, 118],
        [53, 21, 36, 131, 63, 163, 60, 109, 81],
        [40, 26, 35, 154, 40, 185, 51, 97, 123],
        [35, 19, 34, 179, 19, 97, 48, 129, 124],
        [36, 20, 26, 136, 62, 164, 33, 77, 154],
        [45, 18, 32, 130, 90, 157, 40, 79, 91],
        [45, 26, 28, 129, 45, 129, 49, 147, 123],
        [38, 44, 51, 136, 74, 162, 57, 97, 121],
    ],
    [
        [75, 17, 22, 136, 138, 185, 32, 34, 166],
        [56, 39, 58, 133, 117, 173, 48, 53, 187],
        [35, 21, 12, 161, 212, 207, 20, 23, 145],
        [56, 29, 19, 117, 109, 181, 55, 68, 112],
        [47, 29, 17, 153, 64, 220, 59, 51, 114],
        [46, 16, 24, 136, 76, 147, 41, 64, 172],
        [34, 17, 11, 108, 152, 187, 13, 15, 209],
        [51, 24, 14, 115, 133, 209, 32, 26, 104],
        [55, 30, 18, 122, 79, 179, 44, 88, 116],
        [37, 49, 25, 129, 168, 164, 41, 54, 148],
    ],
    [
        [82, 22, 32, 127, 143, 213, 39, 41, 70],
        [62, 44, 61, 123, 105, 189, 48, 57, 64],
        [47, 25, 17, 175, 222, 220, 24, 30, 86],
        [68, 36, 17, 106, 102, 206, 59, 74, 74],
        [57, 39, 23, 151, 68, 216, 55, 63, 58],
        [49, 30, 35, 141, 70, 168, 82, 40, 115],
        [51, 25, 15, 136, 129, 202, 38, 35, 139],
        [68, 26, 16, 111, 141, 215, 29, 28, 28],
        [59, 39, 19, 114, 75, 180, 77, 104, 42],
        [40, 61, 26, 126, 152, 206, 61, 59, 93],
    ],
    [
        [78, 23, 39, 111, 117, 170, 74, 124, 94],
        [48, 34, 86, 101, 92, 146, 78, 179, 134],
        [47, 22, 24, 138, 187, 178, 68, 69, 59],
        [56, 25, 33, 105, 112, 187, 95, 177, 129],
        [48, 31, 27, 114, 63, 183, 82, 116, 56],
        [43, 28, 37, 121, 63, 123, 61, 192, 169],
        [42, 17, 24, 109, 97, 177, 56, 76, 122],
        [58, 18, 28, 105, 139, 182, 70, 92, 63],
        [46, 23, 32, 74, 86, 150, 67, 183, 88],
        [36, 38, 48, 92, 122, 165, 88, 137, 91],
    ],
    [
        [65, 70, 60, 155, 159, 199, 61, 60, 81],
        [44, 78, 115, 132, 119, 173, 71, 112, 93],
        [39, 38, 21, 184, 227, 206, 42, 32, 64],
        [58, 47, 36, 124, 137, 193, 80, 82, 78],
        [49, 50, 35, 144, 95, 205, 63, 78, 59],
        [41, 53, 52, 148, 71, 142, 65, 128, 51],
        [40, 36, 28, 143, 143, 202, 40, 55, 137],
        [52, 34, 29, 129, 183, 227, 42, 35, 43],
        [42, 44, 44, 104, 105, 164, 64, 130, 80],
        [43, 81, 53, 140, 169, 204, 68, 84, 72],
    ],
];

const KF_UV_MODE_PROBS: [[u8; INTRA_MODE_PROBS]; INTRA_MODES] = [
    [144, 11, 54, 157, 195, 130, 46, 58, 108],
    [118, 15, 123, 148, 131, 101, 44, 93, 131],
    [113, 12, 23, 188, 226, 142, 26, 32, 125],
    [120, 11, 50, 123, 163, 135, 64, 77, 103],
    [113, 9, 36, 155, 111, 157, 32, 44, 161],
    [116, 9, 55, 176, 76, 96, 37, 61, 149],
    [115, 9, 28, 141, 161, 167, 21, 25, 193],
    [120, 12, 32, 145, 195, 142, 32, 38, 86],
    [116, 12, 64, 120, 140, 125, 49, 115, 121],
    [102, 19, 66, 162, 182, 122, 35, 59, 128],
];

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::residual::{FrameDequant, TransformCoefficients};
    use super::{
        BlockSize, CoefToken, CurrentFrameMut, CurrentPlaneMut, DecodedBlockInfo, FrameModeBuffers,
        GOLDEN_FRAME, INTRA_FRAME, InterPredictionContext, IntraMode, IntraPredictionEdges,
        IntraPredictionRequest, LAST_FRAME, MAX_INTRA_ABOVE, MAX_TX_COEFFS, MAX_TX_WIDTH,
        ModeInfoView, ModeInfoViewMut, MotionVector, NEARESTMV, NONE_FRAME, NeighborModeInfo,
        REF_LISTS, ReferenceFrame, ReferenceFrames, ReferencePlane, STORED_MODE_INFO_BYTES,
        SUB_BLOCKS, SWITCHABLE_FILTER_SENTINEL, ScaledMotion, StoredModeInfo, TileModeContexts,
        TileParseBuffers, TileParser, TxSize, TxType, ZEROMV, add_residual_block,
        inter_predict_sample, intra_predict_block, parse_intra_tiles, read_coef, select_inter_mv,
    };
    use crate::boolcoder::BoolDecoder;
    use crate::compressed_header::{CompressedHeader, ReferenceMode, TxMode};
    use crate::header::{FrameType, LoopFilterParams, UncompressedFrameHeader};
    use crate::probability::{FrameContext, SyntaxCounts};
    use crate::tile::parse_tile_layout;

    #[test]
    fn dc_prediction_covers_neighbor_availability_cases() {
        let mut above = [0; MAX_INTRA_ABOVE];
        above[..4].copy_from_slice(&[10, 20, 30, 40]);
        let mut left = [0; MAX_TX_WIDTH];
        left[..4].copy_from_slice(&[50, 60, 70, 80]);
        let edges = IntraPredictionEdges {
            above_left: 0,
            above_row: above,
            left_col: left,
        };

        assert_prediction_all(
            IntraPredictionRequest {
                mode: IntraMode::Dc,
                have_left: true,
                have_above: true,
                size: 4,
            },
            &edges,
            45,
        );
        assert_prediction_all(
            IntraPredictionRequest {
                mode: IntraMode::Dc,
                have_left: true,
                have_above: false,
                size: 4,
            },
            &edges,
            65,
        );
        assert_prediction_all(
            IntraPredictionRequest {
                mode: IntraMode::Dc,
                have_left: false,
                have_above: true,
                size: 4,
            },
            &edges,
            25,
        );
        assert_prediction_all(
            IntraPredictionRequest {
                mode: IntraMode::Dc,
                have_left: false,
                have_above: false,
                size: 4,
            },
            &edges,
            128,
        );
    }

    #[test]
    fn vertical_and_horizontal_prediction_copy_edges() {
        let edges = prediction_edges(0, &[1, 2, 3, 4], &[9, 8, 7, 6]);

        assert_prediction(
            IntraPredictionRequest {
                mode: IntraMode::V,
                have_left: true,
                have_above: true,
                size: 4,
            },
            &edges,
            &[1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4, 1, 2, 3, 4],
        );
        assert_prediction(
            IntraPredictionRequest {
                mode: IntraMode::H,
                have_left: true,
                have_above: true,
                size: 4,
            },
            &edges,
            &[9, 9, 9, 9, 8, 8, 8, 8, 7, 7, 7, 7, 6, 6, 6, 6],
        );
    }

    #[test]
    fn true_motion_prediction_clips_to_sample_range() {
        let edges = prediction_edges(100, &[250, 10, 100, 200], &[250, 10, 100, 0]);
        let pred = prediction(
            IntraPredictionRequest {
                mode: IntraMode::Tm,
                have_left: true,
                have_above: true,
                size: 4,
            },
            &edges,
        );

        assert_eq!(pred[0], 255);
        assert_eq!(pred[5], 0);
        assert_eq!(pred[10], 100);
        assert_eq!(pred[15], 100);
    }

    #[test]
    fn directional_prediction_d45_uses_extended_above_edge() {
        let edges = prediction_edges(0, &[10, 20, 30, 40, 50, 60, 70, 80], &[0; 4]);

        assert_prediction(
            IntraPredictionRequest {
                mode: IntraMode::D45,
                have_left: true,
                have_above: true,
                size: 4,
            },
            &edges,
            &[
                20, 30, 40, 50, 30, 40, 50, 60, 40, 50, 60, 70, 50, 60, 70, 80,
            ],
        );
    }

    #[test]
    fn directional_prediction_d207_uses_left_edge() {
        let edges = prediction_edges(0, &[0; 8], &[10, 20, 30, 40]);

        assert_prediction(
            IntraPredictionRequest {
                mode: IntraMode::D207,
                have_left: true,
                have_above: true,
                size: 4,
            },
            &edges,
            &[
                15, 20, 25, 30, 25, 30, 35, 38, 35, 38, 40, 40, 40, 40, 40, 40,
            ],
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
    fn chroma_sub8x8_mv_selection_averages_luma_subblocks() {
        let mut block = test_block(false);
        block.is_inter = true;
        block.ref_frames = [LAST_FRAME, NONE_FRAME];
        block.block_mvs[0] = [
            MotionVector { row: 1, col: -1 },
            MotionVector { row: 2, col: -2 },
            MotionVector { row: 3, col: -3 },
            MotionVector { row: 4, col: -4 },
        ];

        assert_eq!(
            select_inter_mv(1, 0, 0, BlockSize::Block4x4, block),
            Ok(MotionVector { row: 3, col: -3 })
        );
    }

    #[test]
    fn integer_inter_prediction_samples_reference_pixels_and_clamps_edges() {
        let data = [
            0, 1, 2, 3, //
            4, 5, 6, 7, //
            8, 9, 10, 11, //
            12, 13, 14, 15,
        ];
        let reference = ReferencePlane::new(&data, crate::PlaneShape::new(4, 4, 4)).unwrap();

        assert_eq!(
            inter_predict_sample(
                reference,
                ScaledMotion {
                    start_x: 1 << 4,
                    start_y: 2 << 4,
                    step_x: 16,
                    step_y: 16,
                },
                0,
                0,
                0,
            ),
            Ok(9)
        );
        assert_eq!(
            inter_predict_sample(
                reference,
                ScaledMotion {
                    start_x: -4 << 4,
                    start_y: -3 << 4,
                    step_x: 16,
                    step_y: 16,
                },
                0,
                0,
                0,
            ),
            Ok(0)
        );
    }

    #[test]
    fn bilinear_inter_prediction_uses_separable_fractional_filtering() {
        let data = [
            10, 30, //
            50, 90,
        ];
        let reference = ReferencePlane::new(&data, crate::PlaneShape::new(2, 2, 2)).unwrap();

        assert_eq!(
            inter_predict_sample(
                reference,
                ScaledMotion {
                    start_x: 8,
                    start_y: 8,
                    step_x: 16,
                    step_y: 16,
                },
                3,
                0,
                0,
            ),
            Ok(45)
        );
    }

    #[test]
    fn compound_inter_prediction_writes_average_of_two_references() {
        let probabilities = FrameContext::DEFAULT;
        let mut contexts = TileModeContexts::new(1).unwrap();
        let mut counts = SyntaxCounts::default();
        let last_y = [10u8; 16];
        let golden_y = [20u8; 16];
        let uv = [128u8; 4];
        let last = test_reference_frame(&last_y, &uv, &uv, 4, 4);
        let golden = test_reference_frame(&golden_y, &uv, &uv, 4, 4);
        let mut references = [None; 4];
        references[usize::from(LAST_FRAME)] = Some(last);
        references[usize::from(GOLDEN_FRAME)] = Some(golden);
        let mut current_frame_storage = TestCurrentFrame::new(4, 4);

        {
            let mut current_frame = current_frame_storage.as_current_frame();
            let parser = TileParser {
                decoder: BoolDecoder::new(&[0x00, 0x00]).unwrap(),
                probabilities: &probabilities,
                counts: &mut counts,
                contexts: &mut contexts,
                tx_mode: TxMode::Only4x4,
                frame_is_intra: false,
                frame_width: 4,
                frame_height: 4,
                reference_mode: ReferenceMode::Compound,
                compound_reference: None,
                interpolation_filter: Some(crate::header::InterpolationFilter::EightTap),
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
                reference_frames: Some(ReferenceFrames::new(references)),
            };
            let mut block = test_block(false);
            block.is_inter = true;
            block.ref_frames = [LAST_FRAME, GOLDEN_FRAME];
            block.interp_filter = 0;

            parser
                .predict_inter(
                    &mut current_frame,
                    InterPredictionContext {
                        plane: 0,
                        mi_row: 0,
                        mi_col: 0,
                        start_x: 0,
                        start_y: 0,
                        width: 4,
                        height: 4,
                        block_idx: 0,
                        mi_size: BlockSize::Block8x8,
                        block,
                    },
                )
                .unwrap();
        }

        assert!(current_frame_storage.y().iter().all(|&sample| sample == 15));
    }

    #[test]
    fn minimal_intra_tile_parses_residual_syntax() {
        let frame = [0u8; 32];
        let header = test_header(false);
        let layout = parse_tile_layout(&frame, &header).unwrap();
        let compressed_header = CompressedHeader::intra(TxMode::Only4x4);
        let mut counts = SyntaxCounts::default();
        let mut current_frame_storage = TestCurrentFrame::new(16, 16);
        let mut current_frame = current_frame_storage.as_current_frame();

        assert_eq!(
            parse_intra_tiles(
                &frame,
                &header,
                &compressed_header,
                &FrameContext::DEFAULT,
                &layout,
                TileParseBuffers::new(
                    &mut counts,
                    FrameModeBuffers::current(None),
                    &mut current_frame,
                ),
            ),
            Ok(())
        );
    }

    #[test]
    fn skipped_residual_updates_nonzero_contexts_without_consuming_token_bits() {
        let probabilities = FrameContext::DEFAULT;
        let mut contexts = TileModeContexts::new(1).unwrap();
        contexts.above_nonzero[0][0] = 1;
        contexts.above_nonzero[0][1] = 1;
        contexts.left_nonzero[0][0] = 1;
        contexts.left_nonzero[0][1] = 1;

        {
            let decoder = BoolDecoder::new(&[0x00, 0x00]).unwrap();
            let bit_offset = decoder.bit_offset();
            let mut counts = SyntaxCounts::default();
            let mut current_frame_storage = TestCurrentFrame::new(8, 8);
            let mut current_frame = current_frame_storage.as_current_frame();
            let mut parser = TileParser {
                decoder,
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
            };

            assert!(
                !parser
                    .decode_residual(
                        0,
                        0,
                        BlockSize::Block8x8,
                        test_block(true),
                        &mut current_frame
                    )
                    .unwrap()
            );
            assert_eq!(parser.decoder.bit_offset(), bit_offset);
        }

        assert_eq!(contexts.above_nonzero[0][0], 0);
        assert_eq!(contexts.above_nonzero[0][1], 0);
        assert_eq!(contexts.left_nonzero[0][0], 0);
        assert_eq!(contexts.left_nonzero[0][1], 0);
    }

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
        };

        let coefficients = parser
            .tokens(
                0,
                (0, 0),
                TxSize::Tx4x4,
                0,
                BlockSize::Block8x8,
                test_block(false),
            )
            .unwrap();

        assert!(!coefficients.nonzero_context());
        assert_eq!(coefficients.eob, 0);
        assert_eq!(coefficients.coefficients[..16], [0; 16]);
        assert_eq!(parser.decoder.bit_offset(), 1);
        assert_eq!(parser.decoder.finish(), Ok(()));
    }

    #[test]
    fn token_sign_bits_store_coefficients_in_raster_position_order() {
        let probabilities = sign_bit_test_probabilities();

        for (data, expected) in [([0x01, 0x80], 1), ([0x40, 0x80], -1)] {
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
            };

            let coefficients = parser
                .tokens(
                    0,
                    (4, 8),
                    TxSize::Tx4x4,
                    0,
                    BlockSize::Block8x8,
                    test_block(false),
                )
                .unwrap();

            assert!(coefficients.nonzero_context());
            assert_eq!(coefficients.eob, 2);
            assert_eq!(coefficients.block.plane, 0);
            assert_eq!(coefficients.block.start, (4, 8));
            assert_eq!(coefficients.block.tx_size, TxSize::Tx4x4);
            assert_eq!(coefficients.block.tx_type, TxType::DctDct);
            assert_eq!(coefficients.coefficients[0], 0);
            assert_eq!(coefficients.coefficients[1], 0);
            assert_eq!(coefficients.coefficients[4], expected);
            assert_eq!(parser.decoder.finish(), Ok(()));
        }
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
    fn dequantize_uses_dc_ac_quantizers_and_tx32_dq_denom() {
        let dequant = FrameDequant::new(4, 2, 0, 0);
        let mut tx16 =
            TransformCoefficients::new(0, (8, 16), TxSize::Tx16x16, TxType::DctDct).unwrap();
        tx16.set_quantized(0, 2).unwrap();
        tx16.set_quantized(1, 4).unwrap();
        tx16.set_eob(2).unwrap();

        let output16 = dequant.dequantize(&tx16, 0);
        assert_eq!(output16.block, tx16.block);
        assert_eq!(output16.eob, 2);
        assert_eq!(output16.coefficients[0], 24);
        assert_eq!(output16.coefficients[1], 44);

        let mut tx32 =
            TransformCoefficients::new(0, (8, 16), TxSize::Tx32x32, TxType::DctDct).unwrap();
        tx32.set_quantized(0, 2).unwrap();
        tx32.set_quantized(1, 4).unwrap();
        tx32.set_eob(2).unwrap();

        let output32 = dequant.dequantize(&tx32, 0);
        assert_eq!(output32.block, tx32.block);
        assert_eq!(output32.eob, 2);
        assert_eq!(output32.coefficients[0], 12);
        assert_eq!(output32.coefficients[1], 22);
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
        };

        assert_eq!(
            read_coef(&mut parser.decoder, CoefToken::DctValCategory1),
            Ok(6)
        );
        assert_eq!(parser.decoder.finish(), Ok(()));
    }

    #[test]
    fn packed_mode_info_round_trips_motion_vectors() {
        let info = StoredModeInfo {
            valid: true,
            skip: true,
            tx_size: TxSize::Tx8x8,
            segment_id: 0,
            segment_map_id: 3,
            mi_size: BlockSize::Block16x16,
            y_mode: NEARESTMV,
            ref_frames: [LAST_FRAME, NONE_FRAME],
            mvs: [
                MotionVector { row: -123, col: 45 },
                MotionVector { row: 67, col: -89 },
            ],
            sub_mvs: [
                [
                    MotionVector { row: 1, col: 2 },
                    MotionVector { row: 3, col: 4 },
                    MotionVector { row: 5, col: 6 },
                    MotionVector { row: 7, col: 8 },
                ],
                [
                    MotionVector { row: -1, col: -2 },
                    MotionVector { row: -3, col: -4 },
                    MotionVector { row: -5, col: -6 },
                    MotionVector { row: -7, col: -8 },
                ],
            ],
        };
        let mut bytes = [0; STORED_MODE_INFO_BYTES * 2];
        let mut view = ModeInfoViewMut::new(&mut bytes).unwrap();

        view.set(1, info).unwrap();

        assert!(!ModeInfoView::new(&bytes).unwrap().get(0).unwrap().valid);
        assert_eq!(ModeInfoView::new(&bytes).unwrap().get(1), Ok(info));
    }

    #[test]
    fn persistent_segment_map_survives_disabled_segmentation_frame() {
        let probabilities = FrameContext::DEFAULT;
        let mut counts = SyntaxCounts::default();
        let mut contexts = TileModeContexts::new(2).unwrap();
        let mut previous_mode_bytes = [0; STORED_MODE_INFO_BYTES * 4];
        {
            let mut previous_modes = ModeInfoViewMut::new(&mut previous_mode_bytes).unwrap();
            for (index, segment_map_id) in [5, 4, 3, 2].into_iter().enumerate() {
                previous_modes
                    .set(
                        index,
                        StoredModeInfo {
                            valid: true,
                            skip: false,
                            tx_size: TxSize::Tx8x8,
                            segment_id: 0,
                            segment_map_id,
                            mi_size: BlockSize::Block8x8,
                            y_mode: ZEROMV,
                            ref_frames: [LAST_FRAME, NONE_FRAME],
                            mvs: [MotionVector::ZERO; REF_LISTS],
                            sub_mvs: [[MotionVector::ZERO; SUB_BLOCKS]; REF_LISTS],
                        },
                    )
                    .unwrap();
            }
        }

        let mut current_mode_bytes = [0; STORED_MODE_INFO_BYTES * 4];
        {
            let prev_frame_modes = ModeInfoView::new(&previous_mode_bytes).unwrap();
            let current_frame_modes = ModeInfoViewMut::new(&mut current_mode_bytes).unwrap();
            let mut parser = TileParser {
                decoder: BoolDecoder::new(&[0x00, 0x00]).unwrap(),
                probabilities: &probabilities,
                counts: &mut counts,
                contexts: &mut contexts,
                tx_mode: TxMode::Only4x4,
                frame_is_intra: false,
                frame_width: 16,
                frame_height: 16,
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
                mi_rows: 2,
                mi_cols: 2,
                tile_col_start: 0,
                tile_col_end: 2,
                left_row_base: 0,
                prev_frame_modes: Some(prev_frame_modes),
                current_frame_modes: Some(current_frame_modes),
                reference_frames: None,
            };
            let mut block = test_block(true);
            block.segment_id = 0;

            parser
                .update_current_frame_modes(0, 0, BlockSize::Block16x16, block)
                .unwrap();
        }

        let current_modes = ModeInfoView::new(&current_mode_bytes).unwrap();
        for (index, segment_map_id) in [5, 4, 3, 2].into_iter().enumerate() {
            let info = current_modes.get(index).unwrap();
            assert!(info.valid);
            assert_eq!(info.segment_id, 0);
            assert_eq!(info.segment_map_id, segment_map_id);
        }
    }

    #[test]
    fn mv_ref_candidate_uses_exact_current_frame_history_for_deep_offsets() {
        let probabilities = FrameContext::DEFAULT;
        let mut contexts = TileModeContexts::new(8).unwrap();
        let stale_mv = MotionVector { row: -3, col: 5 };
        contexts.above_mode[4] = NeighborModeInfo {
            y_mode: ZEROMV,
            ref_frames: [LAST_FRAME, NONE_FRAME],
            mvs: [stale_mv, MotionVector::ZERO],
            ..NeighborModeInfo::DEFAULT
        };

        let exact_mv = MotionVector { row: 11, col: -7 };
        let exact_sub_mvs = [
            MotionVector { row: 1, col: 2 },
            MotionVector { row: 3, col: 4 },
            MotionVector { row: 5, col: 6 },
            MotionVector { row: 7, col: 8 },
        ];
        let mut current_mode_bytes = [0; STORED_MODE_INFO_BYTES * 64];
        let mut current_modes = ModeInfoViewMut::new(&mut current_mode_bytes).unwrap();
        current_modes
            .set(
                8 + 4,
                StoredModeInfo {
                    valid: true,
                    skip: false,
                    tx_size: TxSize::Tx4x4,
                    segment_id: 0,
                    segment_map_id: 0,
                    mi_size: BlockSize::Block8x8,
                    y_mode: NEARESTMV,
                    ref_frames: [LAST_FRAME, NONE_FRAME],
                    mvs: [exact_mv, MotionVector::ZERO],
                    sub_mvs: [exact_sub_mvs, [MotionVector::ZERO; SUB_BLOCKS]],
                },
            )
            .unwrap();

        let mut counts = SyntaxCounts::default();
        let parser = TileParser {
            decoder: BoolDecoder::new(&[0x00, 0x00]).unwrap(),
            probabilities: &probabilities,
            counts: &mut counts,
            contexts: &mut contexts,
            tx_mode: TxMode::Only4x4,
            frame_is_intra: false,
            frame_width: 64,
            frame_height: 64,
            reference_mode: ReferenceMode::Single,
            compound_reference: None,
            interpolation_filter: Some(crate::header::InterpolationFilter::EightTap),
            allow_high_precision_mv: false,
            use_prev_frame_mvs: false,
            ref_frame_sign_bias: [false; 4],
            lossless: true,
            dequant: FrameDequant::new(0, 0, 0, 0),
            segmentation: crate::header::SegmentationParams::disabled(),
            segment_map_reset: false,
            mi_rows: 8,
            mi_cols: 8,
            tile_col_start: 0,
            tile_col_end: 8,
            left_row_base: 0,
            prev_frame_modes: None,
            current_frame_modes: Some(current_modes),
            reference_frames: None,
        };

        let candidate = parser.mv_ref_candidate(3, 4, &[-2, 0]).unwrap().unwrap();

        assert_eq!(candidate.y_mode, NEARESTMV);
        assert_eq!(candidate.mvs[0], exact_mv);
        assert_eq!(candidate.sub_mvs[0], exact_sub_mvs);
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

    struct TestCurrentFrame {
        y: [u8; 16 * 16],
        u: [u8; 8 * 8],
        v: [u8; 8 * 8],
        y_len: usize,
        uv_len: usize,
        y_shape: crate::PlaneShape,
        uv_shape: crate::PlaneShape,
    }

    impl TestCurrentFrame {
        fn new(width: u32, height: u32) -> Self {
            let y_len = usize::try_from(width * height).unwrap();
            let chroma_width = width.div_ceil(2);
            let chroma_height = height.div_ceil(2);
            let uv_len = usize::try_from(chroma_width * chroma_height).unwrap();
            assert!(y_len <= 16 * 16);
            assert!(uv_len <= 8 * 8);
            Self {
                y: [128; 16 * 16],
                u: [128; 8 * 8],
                v: [128; 8 * 8],
                y_len,
                uv_len,
                y_shape: crate::PlaneShape::new(width, height, width as usize),
                uv_shape: crate::PlaneShape::new(
                    chroma_width,
                    chroma_height,
                    chroma_width as usize,
                ),
            }
        }

        fn as_current_frame(&mut self) -> CurrentFrameMut<'_> {
            CurrentFrameMut::new(
                CurrentPlaneMut::new(&mut self.y[..self.y_len], self.y_shape).unwrap(),
                CurrentPlaneMut::new(&mut self.u[..self.uv_len], self.uv_shape).unwrap(),
                CurrentPlaneMut::new(&mut self.v[..self.uv_len], self.uv_shape).unwrap(),
            )
        }

        fn y(&self) -> &[u8] {
            &self.y[..self.y_len]
        }
    }

    fn test_reference_frame<'a>(
        y: &'a [u8],
        u: &'a [u8],
        v: &'a [u8],
        width: u32,
        height: u32,
    ) -> ReferenceFrame<'a> {
        let chroma_width = width.div_ceil(2);
        let chroma_height = height.div_ceil(2);
        ReferenceFrame::new(
            ReferencePlane::new(y, crate::PlaneShape::new(width, height, width as usize)).unwrap(),
            ReferencePlane::new(
                u,
                crate::PlaneShape::new(chroma_width, chroma_height, chroma_width as usize),
            )
            .unwrap(),
            ReferencePlane::new(
                v,
                crate::PlaneShape::new(chroma_width, chroma_height, chroma_width as usize),
            )
            .unwrap(),
        )
    }

    fn prediction_edges(above_left: u8, above: &[u8], left: &[u8]) -> IntraPredictionEdges {
        let mut above_row = [0; MAX_INTRA_ABOVE];
        above_row[..above.len()].copy_from_slice(above);
        let mut left_col = [0; MAX_TX_WIDTH];
        left_col[..left.len()].copy_from_slice(left);
        IntraPredictionEdges {
            above_left,
            above_row,
            left_col,
        }
    }

    fn prediction(
        request: IntraPredictionRequest,
        edges: &IntraPredictionEdges,
    ) -> [u8; MAX_TX_COEFFS] {
        let mut pred = [0; MAX_TX_COEFFS];
        intra_predict_block(request, edges, &mut pred).unwrap();
        pred
    }

    fn assert_prediction(
        request: IntraPredictionRequest,
        edges: &IntraPredictionEdges,
        expected: &[u8],
    ) {
        let pred = prediction(request, edges);
        assert_eq!(&pred[..expected.len()], expected);
    }

    fn assert_prediction_all(
        request: IntraPredictionRequest,
        edges: &IntraPredictionEdges,
        expected: u8,
    ) {
        let pred = prediction(request, edges);
        assert!(
            pred[..request.size * request.size]
                .iter()
                .all(|&sample| sample == expected),
            "prediction was {:?}",
            &pred[..request.size * request.size]
        );
    }

    fn test_header(segmentation_enabled: bool) -> UncompressedFrameHeader {
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
            base_q_idx: 0,
            delta_q_y_dc: 0,
            delta_q_uv_dc: 0,
            delta_q_uv_ac: 0,
            lossless: true,
            loop_filter: LoopFilterParams::disabled(),
            segmentation: crate::header::SegmentationParams::disabled(),
            segmentation_enabled,
            segmentation_update_map: false,
            tile_cols_log2: 0,
            tile_rows_log2: 0,
            header_size_in_bytes: 1,
            compressed_header_offset: 0,
            tile_data_offset: 1,
        }
    }

    fn sign_bit_test_probabilities() -> FrameContext {
        let mut probabilities = FrameContext::DEFAULT;
        let tx = TxSize::Tx4x4.index();
        let plane_type = 0;
        let ref_type = 0;

        // Scan c=0: read more_coefs=1, then a ZERO token at raster position 0.
        probabilities.coef_probs[tx][plane_type][ref_type][0][0][0] = 0;
        probabilities.coef_probs[tx][plane_type][ref_type][0][0][1] = 255;

        // Scan c=1 has raster position 4 for DCT_DCT 4x4. Since the prior
        // token was ZERO, there is no more_coefs bit before this token. Force a
        // ONE token, then force more_coefs=0 at c=2.
        probabilities.coef_probs[tx][plane_type][ref_type][1][0][0] = 255;
        probabilities.coef_probs[tx][plane_type][ref_type][1][0][1] = 0;
        probabilities.coef_probs[tx][plane_type][ref_type][1][0][2] = 255;

        probabilities
    }

    fn test_block(skip: bool) -> DecodedBlockInfo {
        DecodedBlockInfo {
            skip,
            tx_size: TxSize::Tx4x4,
            y_mode: IntraMode::Dc.raw(),
            uv_mode: IntraMode::Dc,
            sub_modes: [IntraMode::Dc; 4],
            segment_id: 0,
            is_inter: false,
            ref_frames: [INTRA_FRAME, NONE_FRAME],
            interp_filter: SWITCHABLE_FILTER_SENTINEL,
            block_mvs: [[MotionVector::ZERO; SUB_BLOCKS]; REF_LISTS],
        }
    }
}

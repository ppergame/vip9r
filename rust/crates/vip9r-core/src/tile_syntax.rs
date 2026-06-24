use crate::DecodeError;
use crate::boolcoder::BoolDecoder;
use crate::compressed_header::{CompressedHeader, TxMode};
use crate::error::ParserError;
use crate::header::UncompressedFrameHeader;
use crate::probability::{FrameContext, TX_SIZE_CONTEXTS};
use crate::tile::{TileDescriptor, TileLayout};

const MAX_MIS: usize = 8192;
const MI_SIZE_PIXELS: u32 = 8;
const MI_BLOCK_64: usize = 8;
const PARTITION_CONTEXTS: usize = 16;
const PARTITION_PROBS: usize = 3;
const INTRA_MODES: usize = 10;
const INTRA_MODE_PROBS: usize = INTRA_MODES - 1;
const BLOCK_SIZES: usize = 13;
const PARTITION_TYPES: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileSyntaxError {
    InvalidBitstream,
    Unimplemented,
}

impl TileSyntaxError {
    pub(crate) const fn into_decode_error<SinkError>(self) -> DecodeError<SinkError> {
        match self {
            Self::InvalidBitstream => DecodeError::InvalidBitstream,
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

pub(crate) fn parse_intra_tiles(
    frame: &[u8],
    header: &UncompressedFrameHeader,
    compressed_header: &CompressedHeader,
    probabilities: &FrameContext,
    layout: &TileLayout,
) -> Result<(), TileSyntaxError> {
    if !header.frame_is_intra || header.show_existing_frame {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    if header.segmentation_enabled || header.segmentation_update_map {
        return Err(TileSyntaxError::Unimplemented);
    }

    let mi_cols = mi_size(header.frame_width)?;
    let mi_rows = mi_size(header.frame_height)?;
    let mut contexts = TileModeContexts::new(mi_cols)?;

    for tile in layout.as_slice() {
        parse_intra_tile(
            frame,
            tile,
            compressed_header.tx_mode,
            probabilities,
            &mut contexts,
            mi_rows,
            mi_cols,
        )?;
    }

    Ok(())
}

fn parse_intra_tile(
    frame: &[u8],
    tile: &TileDescriptor,
    tx_mode: TxMode,
    probabilities: &FrameContext,
    contexts: &mut TileModeContexts,
    mi_rows: usize,
    mi_cols: usize,
) -> Result<(), TileSyntaxError> {
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
        contexts,
        tx_mode,
        mi_rows,
        mi_cols,
        tile_col_start,
        left_row_base: tile_row_start,
    };

    let mut row = tile_row_start;
    while row < tile_row_end {
        parser.left_row_base = row;
        parser.contexts.clear_left_context();

        let mut col = tile_col_start;
        while col < tile_col_end {
            parser.decode_partition(row, col, BlockSize::Block64x64)?;
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

struct TileParser<'a, 'b> {
    decoder: BoolDecoder<'a>,
    probabilities: &'b FrameContext,
    contexts: &'b mut TileModeContexts,
    tx_mode: TxMode,
    mi_rows: usize,
    mi_cols: usize,
    tile_col_start: usize,
    left_row_base: usize,
}

impl TileParser<'_, '_> {
    fn decode_partition(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
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
            self.decode_block(row, col, subsize)?;
        } else if partition == PartitionType::Horz {
            self.decode_block(row, col, subsize)?;
            if has_rows {
                self.decode_block(row + half_block_8x8, col, subsize)?;
            }
        } else if partition == PartitionType::Vert {
            self.decode_block(row, col, subsize)?;
            if has_cols {
                self.decode_block(row, col + half_block_8x8, subsize)?;
            }
        } else {
            self.decode_partition(row, col, subsize)?;
            self.decode_partition(row, col + half_block_8x8, subsize)?;
            self.decode_partition(row + half_block_8x8, col, subsize)?;
            self.decode_partition(row + half_block_8x8, col + half_block_8x8, subsize)?;
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
        if !has_rows && !has_cols {
            return Ok(PartitionType::Split);
        }

        let ctx = self
            .contexts
            .partition_context(self.left_row_base, row, col, block_size)?;
        let probs = &KF_PARTITION_PROBS[ctx];
        let raw = if has_rows && has_cols {
            self.decoder.read_tree(&PARTITION_TREE, probs)?
        } else if has_cols {
            self.decoder.read_tree(&COLS_PARTITION_TREE, &probs[1..2])?
        } else {
            self.decoder.read_tree(&ROWS_PARTITION_TREE, &probs[2..3])?
        };

        PartitionType::from_raw(raw).ok_or(TileSyntaxError::InvalidBitstream)
    }

    fn decode_block(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
    ) -> Result<(), TileSyntaxError> {
        let avail_u = row > 0;
        let avail_l = col > self.tile_col_start;
        let block = self.intra_frame_mode_info(row, col, block_size, avail_u, avail_l)?;

        // The current implementation intentionally stops at the residual
        // handoff. For intra blocks, residual parsing does not alter the mode,
        // skip or transform-size values needed by following mode-info contexts,
        // so keep those contexts coherent before returning the boundary error.
        self.contexts
            .update_mode_context(self.left_row_base, row, col, block_size, block)?;
        Err(TileSyntaxError::Unimplemented)
    }

    fn intra_frame_mode_info(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        avail_u: bool,
        avail_l: bool,
    ) -> Result<DecodedBlockInfo, TileSyntaxError> {
        let segment_id = 0;
        let skip = self.read_skip(row, col, avail_u, avail_l)?;
        let tx_size = self.read_tx_size(row, col, block_size, avail_u, avail_l)?;
        let (y_mode, sub_modes) = self.read_intra_modes(row, col, block_size, avail_u, avail_l)?;
        let _uv_mode = self.read_default_uv_mode(y_mode)?;

        Ok(DecodedBlockInfo {
            skip,
            tx_size,
            y_mode,
            sub_modes,
            segment_id,
        })
    }

    fn read_skip(
        &mut self,
        row: usize,
        col: usize,
        avail_u: bool,
        avail_l: bool,
    ) -> Result<bool, TileSyntaxError> {
        let ctx = self
            .contexts
            .skip_context(self.left_row_base, row, col, avail_u, avail_l)?;
        Ok(self.decoder.read_bool(self.probabilities.skip_prob[ctx])?)
    }

    fn read_tx_size(
        &mut self,
        row: usize,
        col: usize,
        block_size: BlockSize,
        avail_u: bool,
        avail_l: bool,
    ) -> Result<TxSize, TileSyntaxError> {
        let max_tx_size = block_size.max_tx_size();
        if self.tx_mode == TxMode::Select && block_size.is_at_least_8x8() {
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
            TxSize::from_raw(raw).ok_or(TileSyntaxError::InvalidBitstream)
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

    fn read_default_intra_mode(
        &mut self,
        above_mode: IntraMode,
        left_mode: IntraMode,
    ) -> Result<IntraMode, TileSyntaxError> {
        let probs = &KF_Y_MODE_PROBS[above_mode.index()][left_mode.index()];
        let raw = self.decoder.read_tree(&INTRA_MODE_TREE, probs)?;
        IntraMode::from_raw(raw).ok_or(TileSyntaxError::InvalidBitstream)
    }

    fn read_default_uv_mode(&mut self, y_mode: IntraMode) -> Result<IntraMode, TileSyntaxError> {
        let probs = &KF_UV_MODE_PROBS[y_mode.index()];
        let raw = self.decoder.read_tree(&INTRA_MODE_TREE, probs)?;
        IntraMode::from_raw(raw).ok_or(TileSyntaxError::InvalidBitstream)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DecodedBlockInfo {
    skip: bool,
    tx_size: TxSize,
    y_mode: IntraMode,
    sub_modes: [IntraMode; 4],
    segment_id: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NeighborModeInfo {
    skip: bool,
    tx_size: TxSize,
    sub_modes: [IntraMode; 4],
}

impl NeighborModeInfo {
    const DEFAULT: Self = Self {
        skip: false,
        tx_size: TxSize::Tx4x4,
        sub_modes: [IntraMode::Dc; 4],
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TileModeContexts {
    above_partition: [u8; MAX_MIS],
    above_mode: [NeighborModeInfo; MAX_MIS],
    left_partition: [u8; MI_BLOCK_64],
    left_mode: [NeighborModeInfo; MI_BLOCK_64],
    mi_cols: usize,
    partition_cols: usize,
}

impl TileModeContexts {
    fn new(mi_cols: usize) -> Result<Self, TileSyntaxError> {
        let partition_cols = mi_cols
            .checked_add(MI_BLOCK_64 - 1)
            .map(|cols| (cols / MI_BLOCK_64) * MI_BLOCK_64)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        if mi_cols == 0 || mi_cols > MAX_MIS || partition_cols > MAX_MIS {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        Ok(Self {
            above_partition: [0; MAX_MIS],
            above_mode: [NeighborModeInfo::DEFAULT; MAX_MIS],
            left_partition: [0; MI_BLOCK_64],
            left_mode: [NeighborModeInfo::DEFAULT; MI_BLOCK_64],
            mi_cols,
            partition_cols,
        })
    }

    fn clear_left_context(&mut self) {
        self.left_partition = [0; MI_BLOCK_64];
        self.left_mode = [NeighborModeInfo::DEFAULT; MI_BLOCK_64];
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
        if ctx >= TX_SIZE_CONTEXTS {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        Ok(ctx)
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
        let _ = block.segment_id;
        let mode_info = NeighborModeInfo {
            skip: block.skip,
            tx_size: block.tx_size,
            sub_modes: block.sub_modes,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

fn mi_size(pixels: u32) -> Result<usize, TileSyntaxError> {
    let mis = pixels
        .checked_add(MI_SIZE_PIXELS - 1)
        .map(|value| value / MI_SIZE_PIXELS)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    usize::try_from(mis).map_err(|_| TileSyntaxError::InvalidBitstream)
}

fn row_offset(left_row_base: usize, row: usize) -> Result<usize, TileSyntaxError> {
    row.checked_sub(left_row_base)
        .ok_or(TileSyntaxError::InvalidBitstream)
}

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

const B_WIDTH_LOG2_LOOKUP: [u8; BLOCK_SIZES] = [0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4];
const B_HEIGHT_LOG2_LOOKUP: [u8; BLOCK_SIZES] = [0, 1, 0, 1, 2, 1, 2, 3, 2, 3, 4, 3, 4];
const NUM_4X4_BLOCKS_WIDE_LOOKUP: [u8; BLOCK_SIZES] = [1, 1, 2, 2, 2, 4, 4, 4, 8, 8, 8, 16, 16];
const NUM_4X4_BLOCKS_HIGH_LOOKUP: [u8; BLOCK_SIZES] = [1, 2, 1, 2, 4, 2, 4, 8, 4, 8, 16, 8, 16];
const MI_WIDTH_LOG2_LOOKUP: [u8; BLOCK_SIZES] = [0, 0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3];
const NUM_8X8_BLOCKS_WIDE_LOOKUP: [u8; BLOCK_SIZES] = [1, 1, 1, 1, 1, 2, 2, 2, 4, 4, 4, 8, 8];
const NUM_8X8_BLOCKS_HIGH_LOOKUP: [u8; BLOCK_SIZES] = [1, 1, 1, 1, 2, 1, 2, 4, 2, 4, 8, 4, 8];
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

#[cfg(test)]
mod tests {
    use super::{TileSyntaxError, parse_intra_tiles};
    use crate::compressed_header::{CompressedHeader, TxMode};
    use crate::header::{FrameType, UncompressedFrameHeader};
    use crate::probability::FrameContext;
    use crate::tile::parse_tile_layout;

    #[test]
    fn minimal_intra_tile_reaches_residual_handoff() {
        let frame = [0u8; 32];
        let header = test_header(false);
        let layout = parse_tile_layout(&frame, &header).unwrap();
        let compressed_header = CompressedHeader {
            tx_mode: TxMode::Only4x4,
        };

        assert_eq!(
            parse_intra_tiles(
                &frame,
                &header,
                &compressed_header,
                &FrameContext::DEFAULT,
                &layout
            ),
            Err(TileSyntaxError::Unimplemented)
        );
    }

    #[test]
    fn segmentation_enabled_intra_tiles_are_explicitly_unimplemented() {
        let frame = [0u8; 32];
        let header = test_header(true);
        let layout = parse_tile_layout(&frame, &header).unwrap();
        let compressed_header = CompressedHeader {
            tx_mode: TxMode::Only4x4,
        };

        assert_eq!(
            parse_intra_tiles(
                &frame,
                &header,
                &compressed_header,
                &FrameContext::DEFAULT,
                &layout
            ),
            Err(TileSyntaxError::Unimplemented)
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
            segmentation_enabled,
            segmentation_update_map: false,
            tile_cols_log2: 0,
            tile_rows_log2: 0,
            header_size_in_bytes: 1,
            compressed_header_offset: 0,
            tile_data_offset: 1,
        }
    }
}

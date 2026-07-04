use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DecodedBlockInfo {
    pub(super) skip: bool,
    pub(super) tx_size: TxSize,
    pub(super) y_mode: u8,
    pub(super) uv_mode: IntraMode,
    pub(super) sub_modes: [IntraMode; 4],
    pub(super) segment_id: u8,
    pub(super) is_inter: bool,
    pub(super) ref_frames: [u8; REF_LISTS],
    pub(super) interp_filter: u8,
    pub(super) block_mvs: [[MotionVector; SUB_BLOCKS]; REF_LISTS],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NeighborModeInfo {
    pub(super) skip: bool,
    pub(super) tx_size: TxSize,
    pub(super) y_mode: u8,
    pub(super) sub_modes: [IntraMode; 4],
    pub(super) ref_frames: [u8; REF_LISTS],
    pub(super) interp_filter: u8,
    pub(super) mvs: [MotionVector; REF_LISTS],
}

impl NeighborModeInfo {
    pub(super) const DEFAULT: Self = Self {
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
pub(super) struct CandidateModeInfo {
    pub(super) y_mode: u8,
    pub(super) ref_frames: [u8; REF_LISTS],
    pub(super) mvs: [MotionVector; REF_LISTS],
    pub(super) sub_mvs: CandidateSubMvs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CandidateSubMvs {
    RepeatedMvs,
    StoredCurrent { index: usize },
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StoredLoopFilterModeInfo {
    pub(super) skip: bool,
    pub(super) tx_size: TxSize,
    pub(super) segment_id: u8,
    pub(super) mi_size: BlockSize,
    pub(super) y_mode: u8,
    pub(super) ref_frame: u8,
}

impl From<NeighborModeInfo> for CandidateModeInfo {
    fn from(info: NeighborModeInfo) -> Self {
        Self {
            y_mode: info.y_mode,
            ref_frames: info.ref_frames,
            mvs: info.mvs,
            sub_mvs: CandidateSubMvs::RepeatedMvs,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StoredModeInfo {
    pub(super) valid: bool,
    pub(super) skip: bool,
    pub(super) tx_size: TxSize,
    // Current frame's SegmentIds entry, used by loop filtering and other
    // current-frame syntax consumers.
    pub(super) segment_id: u8,
    // Persistent PrevSegmentIds entry after this frame. This can differ from
    // segment_id when segmentation_update_map is false: current block syntax
    // uses get_segment_id(), but the saved segmentation map is not refreshed.
    pub(super) segment_map_id: u8,
    pub(super) mi_size: BlockSize,
    pub(super) y_mode: u8,
    pub(super) ref_frames: [u8; REF_LISTS],
    pub(super) mvs: [MotionVector; REF_LISTS],
    pub(super) sub_mvs: [[MotionVector; SUB_BLOCKS]; REF_LISTS],
}

pub(super) const STORED_MODE_INFO_VALID_OFFSET: usize = 0;
pub(super) const STORED_MODE_INFO_Y_MODE_OFFSET: usize = 1;
pub(super) const STORED_MODE_INFO_REF_FRAMES_OFFSET: usize = 2;
pub(super) const STORED_MODE_INFO_MVS_OFFSET: usize = 4;
pub(super) const STORED_MODE_INFO_SUB_MVS_OFFSET: usize = 12;
pub(super) const STORED_MODE_INFO_SKIP_OFFSET: usize = 44;
pub(super) const STORED_MODE_INFO_TX_SIZE_OFFSET: usize = 45;
pub(super) const STORED_MODE_INFO_SEGMENT_ID_OFFSET: usize = 46;
pub(super) const STORED_MODE_INFO_MI_SIZE_OFFSET: usize = 47;
pub(super) const STORED_MODE_INFO_SEGMENT_MAP_ID_OFFSET: usize = 48;

pub(crate) const STORED_MODE_INFO_BYTES: usize = 49;

pub(crate) fn mode_info_byte_len(mi_count: usize) -> Option<usize> {
    mi_count.checked_mul(STORED_MODE_INFO_BYTES)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModeInfoView<'a> {
    pub(super) data: &'a [u8],
    mi_cols: usize,
    band_mi_col_start: usize,
    band_mi_col_end: usize,
}

impl<'a> ModeInfoView<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Result<Self, DecodeError> {
        if !data.len().is_multiple_of(STORED_MODE_INFO_BYTES) {
            return Err(DecodeError::InvalidConfig);
        }
        Ok(Self {
            data,
            mi_cols: 0,
            band_mi_col_start: 0,
            band_mi_col_end: 0,
        })
    }

    fn entry(self, index: usize) -> Result<&'a [u8], TileSyntaxError> {
        self.check_band(index)?;
        let start = mode_info_offset(index)?;
        let end = start
            .checked_add(STORED_MODE_INFO_BYTES)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        self.data
            .get(start..end)
            .ok_or(TileSyntaxError::InvalidBitstream)
    }

    fn check_band(self, index: usize) -> Result<(), TileSyntaxError> {
        if self.mi_cols == 0 {
            return Ok(());
        }
        let col = index % self.mi_cols;
        if col < self.band_mi_col_start || col >= self.band_mi_col_end {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        Ok(())
    }

    pub(crate) fn raw_parts(self) -> ModeInfoViewRaw {
        ModeInfoViewRaw {
            data: self.data.as_ptr() as usize,
            len: self.data.len(),
            mi_cols: self.mi_cols,
            band_mi_col_start: self.band_mi_col_start,
            band_mi_col_end: self.band_mi_col_end,
        }
    }

    pub(crate) unsafe fn from_raw_parts(raw: ModeInfoViewRaw) -> Result<Self, TileSyntaxError> {
        if !raw.len.is_multiple_of(STORED_MODE_INFO_BYTES) {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        if raw.mi_cols != 0 && raw.band_mi_col_start > raw.band_mi_col_end {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        // SAFETY: The caller guarantees that raw.data/raw.len describe a
        // live immutable mode-history region for the duration of the job.
        let data = unsafe { core::slice::from_raw_parts(raw.data as *const u8, raw.len) };
        Ok(Self {
            data,
            mi_cols: raw.mi_cols,
            band_mi_col_start: raw.band_mi_col_start,
            band_mi_col_end: raw.band_mi_col_end,
        })
    }

    #[allow(dead_code)]
    pub(super) fn get(self, index: usize) -> Result<StoredModeInfo, TileSyntaxError> {
        let entry = self.entry(index)?;
        decode_stored_mode_info(entry)
    }

    pub(super) fn mv_ref_candidate(
        self,
        index: usize,
        sub_mvs: CandidateSubMvs,
    ) -> Result<Option<CandidateModeInfo>, TileSyntaxError> {
        let entry = self.entry(index)?;
        if entry[STORED_MODE_INFO_VALID_OFFSET] == 0 {
            return Ok(None);
        }

        Ok(Some(CandidateModeInfo {
            y_mode: entry[STORED_MODE_INFO_Y_MODE_OFFSET],
            ref_frames: [
                entry[STORED_MODE_INFO_REF_FRAMES_OFFSET],
                entry[STORED_MODE_INFO_REF_FRAMES_OFFSET + 1],
            ],
            mvs: [
                decode_motion_vector(entry, STORED_MODE_INFO_MVS_OFFSET)?,
                decode_motion_vector(entry, STORED_MODE_INFO_MVS_OFFSET + 4)?,
            ],
            sub_mvs,
        }))
    }

    pub(super) fn sub_mv(
        self,
        index: usize,
        ref_list: usize,
        sub_block: usize,
    ) -> Result<MotionVector, TileSyntaxError> {
        if ref_list >= REF_LISTS || sub_block >= SUB_BLOCKS {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        let offset = ref_list
            .checked_mul(SUB_BLOCKS)
            .and_then(|base| base.checked_add(sub_block))
            .and_then(|slot| slot.checked_mul(4))
            .and_then(|byte_offset| byte_offset.checked_add(STORED_MODE_INFO_SUB_MVS_OFFSET))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        decode_motion_vector(self.entry(index)?, offset)
    }

    pub(super) fn loop_filter_info(
        self,
        index: usize,
    ) -> Result<Option<StoredLoopFilterModeInfo>, TileSyntaxError> {
        let entry = self.entry(index)?;
        if entry[STORED_MODE_INFO_VALID_OFFSET] == 0 {
            return Ok(None);
        }

        let tx_size = TxSize::from_raw(entry[STORED_MODE_INFO_TX_SIZE_OFFSET])
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let mi_size = BlockSize::from_raw(entry[STORED_MODE_INFO_MI_SIZE_OFFSET])
            .ok_or(TileSyntaxError::InvalidBitstream)?;

        Ok(Some(StoredLoopFilterModeInfo {
            skip: entry[STORED_MODE_INFO_SKIP_OFFSET] != 0,
            tx_size,
            segment_id: entry[STORED_MODE_INFO_SEGMENT_ID_OFFSET],
            mi_size,
            y_mode: entry[STORED_MODE_INFO_Y_MODE_OFFSET],
            ref_frame: entry[STORED_MODE_INFO_REF_FRAMES_OFFSET],
        }))
    }

    pub(super) fn segment_map_id(self, index: usize) -> Result<Option<u8>, TileSyntaxError> {
        let entry = self.entry(index)?;
        if entry[STORED_MODE_INFO_VALID_OFFSET] == 0 {
            return Ok(None);
        }
        Ok(Some(entry[STORED_MODE_INFO_SEGMENT_MAP_ID_OFFSET]))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ModeInfoViewMut<'a> {
    pub(super) data: &'a mut [u8],
    mi_cols: usize,
    band_mi_col_start: usize,
    band_mi_col_end: usize,
}

impl<'a> ModeInfoViewMut<'a> {
    pub(crate) fn new(data: &'a mut [u8]) -> Result<Self, DecodeError> {
        if !data.len().is_multiple_of(STORED_MODE_INFO_BYTES) {
            return Err(DecodeError::InvalidConfig);
        }
        Ok(Self {
            data,
            mi_cols: 0,
            band_mi_col_start: 0,
            band_mi_col_end: 0,
        })
    }

    pub(crate) fn clear(&mut self) {
        self.data.fill(0);
    }

    pub(super) fn as_view(&self) -> ModeInfoView<'_> {
        ModeInfoView {
            data: self.data,
            mi_cols: self.mi_cols,
            band_mi_col_start: self.band_mi_col_start,
            band_mi_col_end: self.band_mi_col_end,
        }
    }

    pub(super) fn reborrow(&mut self) -> ModeInfoViewMut<'_> {
        ModeInfoViewMut {
            data: self.data,
            mi_cols: self.mi_cols,
            band_mi_col_start: self.band_mi_col_start,
            band_mi_col_end: self.band_mi_col_end,
        }
    }

    #[allow(dead_code)]
    pub(super) fn get(&self, index: usize) -> Result<StoredModeInfo, TileSyntaxError> {
        self.as_view().get(index)
    }

    fn entry_mut(&mut self, index: usize) -> Result<&mut [u8], TileSyntaxError> {
        self.check_band(index)?;
        let start = mode_info_offset(index)?;
        let end = start
            .checked_add(STORED_MODE_INFO_BYTES)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        self.data
            .get_mut(start..end)
            .ok_or(TileSyntaxError::InvalidBitstream)
    }

    fn check_band(&self, index: usize) -> Result<(), TileSyntaxError> {
        if self.mi_cols == 0 {
            return Ok(());
        }
        let col = index % self.mi_cols;
        if col < self.band_mi_col_start || col >= self.band_mi_col_end {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        Ok(())
    }

    pub(crate) fn raw_parts(&mut self) -> ModeInfoViewMutRaw {
        ModeInfoViewMutRaw {
            data: self.data.as_mut_ptr() as usize,
            len: self.data.len(),
        }
    }

    pub(crate) unsafe fn from_raw_band(
        raw: ModeInfoViewMutRaw,
        mi_cols: usize,
        band_mi_col_start: usize,
        band_mi_col_end: usize,
    ) -> Result<Self, TileSyntaxError> {
        if mi_cols == 0
            || band_mi_col_start > band_mi_col_end
            || band_mi_col_end > mi_cols
            || !raw.len.is_multiple_of(STORED_MODE_INFO_BYTES)
        {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        // SAFETY: The band splitter creates one mutable view per
        // column-disjoint band from this raw full-grid region, and the pool's
        // Release/Acquire protocol joins all users before the coordinator
        // reuses the original grid.
        let data = unsafe { core::slice::from_raw_parts_mut(raw.data as *mut u8, raw.len) };
        Ok(Self {
            data,
            mi_cols,
            band_mi_col_start,
            band_mi_col_end,
        })
    }

    #[allow(dead_code)]
    pub(super) fn set(
        &mut self,
        index: usize,
        info: StoredModeInfo,
    ) -> Result<(), TileSyntaxError> {
        encode_stored_mode_info(info, self.entry_mut(index)?);
        Ok(())
    }

    pub(super) fn set_encoded(
        &mut self,
        index: usize,
        bytes: &[u8; STORED_MODE_INFO_BYTES],
    ) -> Result<(), TileSyntaxError> {
        self.entry_mut(index)?.copy_from_slice(bytes);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModeInfoViewRaw {
    pub(crate) data: usize,
    pub(crate) len: usize,
    pub(crate) mi_cols: usize,
    pub(crate) band_mi_col_start: usize,
    pub(crate) band_mi_col_end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ModeInfoViewMutRaw {
    pub(crate) data: usize,
    pub(crate) len: usize,
}

pub(crate) struct FrameModeBuffers<'a> {
    pub(super) use_prev_frame_mvs: bool,
    pub(super) prev_frame_modes: Option<ModeInfoView<'a>>,
    pub(super) current_frame_modes: Option<ModeInfoViewMut<'a>>,
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

    pub(super) fn for_tile(&mut self) -> FrameModeBuffers<'_> {
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

pub(super) fn mode_info_offset(index: usize) -> Result<usize, TileSyntaxError> {
    index
        .checked_mul(STORED_MODE_INFO_BYTES)
        .ok_or(TileSyntaxError::InvalidBitstream)
}

#[allow(dead_code)]
pub(super) fn decode_stored_mode_info(bytes: &[u8]) -> Result<StoredModeInfo, TileSyntaxError> {
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

pub(super) fn encode_stored_mode_info(info: StoredModeInfo, bytes: &mut [u8]) {
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

pub(super) fn decode_motion_vector(
    bytes: &[u8],
    offset: usize,
) -> Result<MotionVector, TileSyntaxError> {
    let row = read_i16_le(bytes, offset)?;
    let col_offset = offset
        .checked_add(2)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let col = read_i16_le(bytes, col_offset)?;
    Ok(MotionVector { row, col })
}

pub(super) fn encode_motion_vector(mv: MotionVector, bytes: &mut [u8], offset: usize) {
    write_i16_le(mv.row, bytes, offset);
    write_i16_le(mv.col, bytes, offset + 2);
}

pub(super) fn read_i16_le(bytes: &[u8], offset: usize) -> Result<i16, TileSyntaxError> {
    let end = offset
        .checked_add(2)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let raw = bytes
        .get(offset..end)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    Ok(i16::from_le_bytes([raw[0], raw[1]]))
}

pub(super) fn write_i16_le(value: i16, bytes: &mut [u8], offset: usize) {
    let raw = value.to_le_bytes();
    bytes[offset] = raw[0];
    bytes[offset + 1] = raw[1];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MotionVector {
    pub(super) row: i16,
    pub(super) col: i16,
}

impl MotionVector {
    pub(super) const ZERO: Self = Self { row: 0, col: 0 };

    pub(super) fn add(self, other: Self) -> Result<Self, TileSyntaxError> {
        Self::new(
            i32::from(self.row)
                .checked_add(i32::from(other.row))
                .ok_or(TileSyntaxError::InvalidBitstream)?,
            i32::from(self.col)
                .checked_add(i32::from(other.col))
                .ok_or(TileSyntaxError::InvalidBitstream)?,
        )
    }

    pub(super) fn new(row: i32, col: i32) -> Result<Self, TileSyntaxError> {
        Ok(Self {
            row: i16::try_from(row).map_err(|_| TileSyntaxError::InvalidBitstream)?,
            col: i16::try_from(col).map_err(|_| TileSyntaxError::InvalidBitstream)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MvRefState {
    pub(super) ref_list: [MotionVector; MAX_MV_REF_CANDIDATES],
    pub(super) nearest: MotionVector,
    pub(super) near: MotionVector,
    pub(super) best: MotionVector,
    pub(super) mode_context: usize,
    pub(super) count: usize,
}

impl MvRefState {
    pub(super) const DEFAULT: Self = Self {
        ref_list: [MotionVector::ZERO; MAX_MV_REF_CANDIDATES],
        nearest: MotionVector::ZERO,
        near: MotionVector::ZERO,
        best: MotionVector::ZERO,
        mode_context: 0,
        count: 0,
    };

    pub(super) fn add_mv_ref(&mut self, mv: MotionVector) {
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
pub(super) struct TileModeContexts {
    pub(super) above_partition: [u8; MAX_MI_COLS],
    pub(super) above_mode: [NeighborModeInfo; MAX_MI_COLS],
    pub(super) above_nonzero: [[u8; MAX_4X4_COLS]; PLANES],
    pub(super) above_seg_pred: [u8; MAX_MI_COLS],
    pub(super) left_partition: [u8; MI_BLOCK_64],
    pub(super) left_mode: [NeighborModeInfo; MI_BLOCK_64],
    pub(super) left_nonzero: [[u8; MI_BLOCK_64 * 2]; PLANES],
    pub(super) left_seg_pred: [u8; MI_BLOCK_64],
    pub(super) mi_cols: usize,
    pub(super) partition_cols: usize,
}

impl TileModeContexts {
    pub(super) fn new(mi_cols: usize) -> Result<Self, TileSyntaxError> {
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

    pub(super) fn clear_left_context(&mut self) {
        self.left_partition = [0; MI_BLOCK_64];
        self.left_mode = [NeighborModeInfo::DEFAULT; MI_BLOCK_64];
        self.left_nonzero = [[0; MI_BLOCK_64 * 2]; PLANES];
        self.left_seg_pred = [0; MI_BLOCK_64];
    }

    pub(super) fn partition_context(
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

    pub(super) fn update_partition_context(
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

    pub(super) fn skip_context(
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

    pub(super) fn seg_pred_context(
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

    pub(super) fn update_seg_pred_context(
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

    pub(super) fn tx_size_context(
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

    pub(super) fn coef_context(
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

    pub(super) fn update_nonzero_context(
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

    pub(super) fn update_mode_context(
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

    pub(super) fn above_mode(&self, col: usize) -> Result<NeighborModeInfo, TileSyntaxError> {
        self.above_mode
            .get(col)
            .copied()
            .ok_or(TileSyntaxError::InvalidBitstream)
    }

    pub(super) fn left_mode(
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

pub(super) fn reference_frame_raw(reference: InterReferenceFrame) -> u8 {
    match reference {
        InterReferenceFrame::Last => LAST_FRAME,
        InterReferenceFrame::Golden => GOLDEN_FRAME,
        InterReferenceFrame::Altref => ALTREF_FRAME,
    }
}

pub(super) fn is_intra(info: NeighborModeInfo) -> bool {
    info.ref_frames[0] == INTRA_FRAME
}

pub(super) fn is_single(info: NeighborModeInfo) -> bool {
    info.ref_frames[1] == NONE_FRAME
}

pub(super) fn single_ref_p1_context(
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

pub(super) fn single_ref_p2_context(
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

pub(super) fn comp_ref_context(
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

pub(super) fn sub_block_mv_index(delta_col: i8, block: i8) -> Result<usize, TileSyntaxError> {
    let block = usize::try_from(block).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    IDX_N_COLUMN_TO_SUBBLOCK
        .get(block)
        .and_then(|row| row.get(usize::from(delta_col == 0)))
        .copied()
        .ok_or(TileSyntaxError::InvalidBitstream)
}

pub(super) fn if_same_ref_frame_add_mv(
    state: &mut MvRefState,
    info: CandidateModeInfo,
    ref_frame: u8,
) {
    for ref_list in 0..REF_LISTS {
        if info.ref_frames[ref_list] == ref_frame {
            state.add_mv_ref(info.mvs[ref_list]);
            return;
        }
    }
}

pub(super) fn if_same_prev_frame_add_mv(
    state: &mut MvRefState,
    info: Option<CandidateModeInfo>,
    ref_frame: u8,
) {
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

pub(super) fn if_diff_ref_frame_add_mv(
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

pub(super) fn if_diff_prev_frame_add_mv(
    state: &mut MvRefState,
    info: Option<CandidateModeInfo>,
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

pub(super) fn lower_mv_precision(component: i32) -> i32 {
    if component & 1 != 0 {
        component + if component > 0 { -1 } else { 1 }
    } else {
        component
    }
}

pub(super) fn use_mv_hp(mv: MotionVector) -> bool {
    (i32::from(mv.row).abs() >> 3) < COMPANDED_MVREF_THRESH
        && (i32::from(mv.col).abs() >> 3) < COMPANDED_MVREF_THRESH
}

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::super::test_support::*;
    use super::*;

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
                counts: &mut counts as *mut SyntaxCounts,
                accumulate_counts: true,
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
                intra: IntraPredictionBuffers::new(),
                residual: ResidualBuffers::new(),
                interp_buffer: [0; MAX_INTERP_BUFFER],
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
            counts: &mut counts as *mut SyntaxCounts,
            accumulate_counts: true,
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
            intra: IntraPredictionBuffers::new(),
            residual: ResidualBuffers::new(),
            interp_buffer: [0; MAX_INTERP_BUFFER],
        };

        let candidate = parser.mv_ref_candidate(3, 4, [-2, 0]).unwrap().unwrap();

        assert_eq!(candidate.y_mode, NEARESTMV);
        assert_eq!(candidate.mvs[0], exact_mv);
        assert_eq!(
            parser.candidate_sub_block_mv(candidate, 0, 0, 0).unwrap(),
            exact_sub_mvs[2]
        );
        assert_eq!(
            parser.candidate_sub_block_mv(candidate, 0, 0, -1).unwrap(),
            exact_mv
        );
    }
}

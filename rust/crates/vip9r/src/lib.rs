#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

#[macro_export]
macro_rules! diag {
    ($($arg:tt)*) => {
        $crate::__vip9r_diag(::core::format_args!($($arg)*))
    };
}

mod bitstream;
mod boolcoder;
mod compressed_header;
mod error;
mod header;
mod probability;
mod superframe;
mod tile;
mod tile_syntax;
mod wasm;

use compressed_header::{
    CompressedHeader, TxMode, parse_inter_compressed_header, parse_intra_compressed_header,
};
use header::{HeaderParserState, parse_uncompressed_frame_header};
use probability::{NonCoefAdaptationConfig, ProbabilityState, SyntaxCounts};
use tile::parse_tile_layout;
use tile_syntax::{
    CurrentFrameMut, CurrentPlaneMut, FrameModeBuffers, ModeInfoView, ModeInfoViewMut,
    ReferenceFrame, ReferenceFrames, ReferencePlane, TileParseBuffers, mode_info_byte_len,
    parse_inter_tiles, parse_intra_tiles,
};

pub const MAX_CODED_FRAMES_PER_PACKET: usize = 8;

#[doc(hidden)]
pub fn __vip9r_diag(args: core::fmt::Arguments<'_>) {
    wasm::log(wasm::LogKind::Diagnostic, args);
}

#[doc(hidden)]
pub fn __vip9r_test_failure(args: core::fmt::Arguments<'_>) {
    wasm::log(wasm::LogKind::TestFailure, args);
}

const REFERENCE_FRAME_SLOTS: usize = 8;
const FRAME_POOL_SLOTS: usize = 1 + REFERENCE_FRAME_SLOTS;
const MODE_HISTORY_SLOTS: usize = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodedFrameRange {
    pub start: usize,
    pub len: usize,
}

impl CodedFrameRange {
    pub fn as_slice(self, packet: &[u8]) -> Result<&[u8], DecodeError> {
        let end = self
            .start
            .checked_add(self.len)
            .ok_or(DecodeError::InvalidBitstream)?;
        packet
            .get(self.start..end)
            .ok_or(DecodeError::InvalidBitstream)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodedFrameRanges {
    ranges: [CodedFrameRange; MAX_CODED_FRAMES_PER_PACKET],
    len: usize,
}

impl CodedFrameRanges {
    pub fn as_slice(&self) -> &[CodedFrameRange] {
        &self.ranges[..self.len]
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

pub fn split_packet(packet: &[u8]) -> Result<CodedFrameRanges, DecodeError> {
    superframe::split_packet(packet).map_err(|err| err.into_decode_error())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceLayout {
    max_width: u32,
    max_height: u32,
    frame_pool: FramePoolLayout,
    mode_history: ModeHistoryLayout,
}

impl WorkspaceLayout {
    pub fn new(max_width: u32, max_height: u32) -> Result<Self, DecodeError> {
        validate_limits(max_width, max_height)?;
        let frame_pool = FramePoolLayout::new(FrameLayout::new(max_width, max_height)?)?;
        let max_mi_count =
            frame_mi_count(max_width, max_height).map_err(|_| DecodeError::InvalidConfig)?;
        let mode_history = ModeHistoryLayout::new(frame_pool.total_bytes(), max_mi_count)?;

        Ok(Self {
            max_width,
            max_height,
            frame_pool,
            mode_history,
        })
    }

    pub const fn max_width(self) -> u32 {
        self.max_width
    }

    pub const fn max_height(self) -> u32 {
        self.max_height
    }

    pub fn total_bytes(self) -> usize {
        self.mode_history.total_bytes()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FramePoolLayout {
    frame: FrameLayout,
    current: ByteRange,
    references: [ByteRange; REFERENCE_FRAME_SLOTS],
}

impl FramePoolLayout {
    fn new(frame: FrameLayout) -> Result<Self, DecodeError> {
        let frame_bytes = frame.bytes();
        let current = ByteRange::new(0, frame_bytes)?;
        let mut references = [ByteRange::empty(); REFERENCE_FRAME_SLOTS];
        let mut next_start = current.end()?;
        for reference in &mut references {
            *reference = ByteRange::new(next_start, frame_bytes)?;
            next_start = reference.end()?;
        }
        Ok(Self {
            frame,
            current,
            references,
        })
    }

    fn total_bytes(self) -> usize {
        debug_assert_eq!(self.current.start, 0);
        debug_assert_eq!(self.current.len, self.frame.bytes());
        debug_assert_eq!(self.references[0].start, self.current.end().unwrap());
        self.references[REFERENCE_FRAME_SLOTS - 1]
            .end()
            .expect("frame-pool layout was checked at construction")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ModeHistoryLayout {
    slots: [ByteRange; MODE_HISTORY_SLOTS],
}

impl ModeHistoryLayout {
    fn new(start: usize, max_mi_count: usize) -> Result<Self, DecodeError> {
        let slot_bytes = mode_info_byte_len(max_mi_count).ok_or(DecodeError::InvalidConfig)?;
        let mut slots = [ByteRange::empty(); MODE_HISTORY_SLOTS];
        let mut next_start = start;
        for slot in &mut slots {
            *slot = ByteRange::new(next_start, slot_bytes)?;
            next_start = slot.end()?;
        }
        Ok(Self { slots })
    }

    fn total_bytes(self) -> usize {
        self.slots[MODE_HISTORY_SLOTS - 1]
            .end()
            .expect("mode-history layout was checked at construction")
    }

    fn slot_len(self) -> usize {
        self.slots[0].len
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModeHistorySlot {
    Slot0,
    Slot1,
}

impl ModeHistorySlot {
    const fn other(self) -> Self {
        match self {
            Self::Slot0 => Self::Slot1,
            Self::Slot1 => Self::Slot0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ByteRange {
    start: usize,
    len: usize,
}

impl ByteRange {
    const fn empty() -> Self {
        Self { start: 0, len: 0 }
    }

    fn new(start: usize, len: usize) -> Result<Self, DecodeError> {
        start.checked_add(len).ok_or(DecodeError::InvalidConfig)?;
        Ok(Self { start, len })
    }

    fn end(self) -> Result<usize, DecodeError> {
        self.start
            .checked_add(self.len)
            .ok_or(DecodeError::InvalidConfig)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameLayout {
    y: PlaneLayout,
    u: PlaneLayout,
    v: PlaneLayout,
}

impl FrameLayout {
    fn new(max_width: u32, max_height: u32) -> Result<Self, DecodeError> {
        // Intra prediction and loop filtering operate over the spec's MI grid
        // (`MiCols * 8` by `MiRows * 8`), even when the visible frame size is
        // not a multiple of 8. Keep frame slots large enough for those
        // reconstructed edge samples; output/reference views still expose the
        // visible dimensions.
        let padded_width = mi_aligned_pixels(max_width).ok_or(DecodeError::InvalidConfig)?;
        let padded_height = mi_aligned_pixels(max_height).ok_or(DecodeError::InvalidConfig)?;
        let y_stride = usize::try_from(padded_width).map_err(|_| DecodeError::InvalidConfig)?;
        let uv_width = padded_width / 2;
        let uv_height = padded_height / 2;
        let uv_stride = usize::try_from(uv_width).map_err(|_| DecodeError::InvalidConfig)?;

        let y = PlaneLayout::new(0, PlaneShape::new(padded_width, padded_height, y_stride))?;
        let u = PlaneLayout::new(y.end()?, PlaneShape::new(uv_width, uv_height, uv_stride))?;
        let v = PlaneLayout::new(u.end()?, u.shape)?;
        let frame = Self { y, u, v };
        frame
            .bytes()
            .checked_mul(FRAME_POOL_SLOTS)
            .ok_or(DecodeError::InvalidConfig)?;
        Ok(frame)
    }

    fn bytes(self) -> usize {
        debug_assert_eq!(self.y.end().ok(), Some(self.u.offset));
        debug_assert_eq!(self.u.end().ok(), Some(self.v.offset));
        self.v
            .end()
            .expect("frame layout was checked at construction")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PlaneLayout {
    offset: usize,
    shape: PlaneShape,
}

impl PlaneLayout {
    fn new(offset: usize, shape: PlaneShape) -> Result<Self, DecodeError> {
        let len = shape.byte_len().ok_or(DecodeError::InvalidConfig)?;
        offset.checked_add(len).ok_or(DecodeError::InvalidConfig)?;
        Ok(Self { offset, shape })
    }

    fn len(self) -> usize {
        self.shape
            .byte_len()
            .expect("plane layout was checked at construction")
    }

    fn end(self) -> Result<usize, DecodeError> {
        self.offset
            .checked_add(self.len())
            .ok_or(DecodeError::InvalidConfig)
    }
}

#[derive(Debug)]
pub struct DecodeWorkspace<'a> {
    layout: WorkspaceLayout,
    memory: &'a mut [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReconstructionBufferRequest {
    frame_width: u32,
    frame_height: u32,
    reference_slots: Option<[InterReferenceSlot; 3]>,
    use_prev_frame_mvs: bool,
    previous_slot: Option<ModeHistorySlot>,
    current_slot: ModeHistorySlot,
    mi_count: usize,
}

impl<'a> DecodeWorkspace<'a> {
    pub fn new(layout: WorkspaceLayout, memory: &'a mut [u8]) -> Result<Self, DecodeError> {
        let total_bytes = layout.total_bytes();
        if memory.len() < total_bytes {
            return Err(DecodeError::ResourceLimit);
        }
        Ok(Self {
            layout,
            memory: &mut memory[..total_bytes],
        })
    }

    fn require_layout(&self, expected: WorkspaceLayout) -> Result<(), DecodeError> {
        if self.layout != expected {
            return Err(DecodeError::InvalidConfig);
        }
        Ok(())
    }

    fn fill_current_default_i420(&mut self) -> Result<(), DecodeError> {
        let frame_layout = self.layout.frame_pool.frame;
        let frame = self.frame_slot_mut(FramePoolSlot::Current)?;
        fill_frame_plane(frame, frame_layout.y, 128)?;
        fill_frame_plane(frame, frame_layout.u, 128)?;
        fill_frame_plane(frame, frame_layout.v, 128)?;
        Ok(())
    }

    fn reconstruction_buffers(
        &mut self,
        request: ReconstructionBufferRequest,
    ) -> Result<
        (
            CurrentFrameMut<'_>,
            Option<ReferenceFrames<'_>>,
            FrameModeBuffers<'_>,
        ),
        DecodeError,
    > {
        self.fill_current_default_i420()?;

        let frame_pool_layout = self.layout.frame_pool;
        let frame_layout = frame_pool_layout.frame;
        let mode_history = self.layout.mode_history;
        let frame_pool_end = frame_pool_layout.total_bytes();
        let first_history = mode_history.slots[0];
        if first_history.start != frame_pool_end {
            return Err(DecodeError::InvalidConfig);
        }
        let history_end = mode_history.slots[MODE_HISTORY_SLOTS - 1].end()?;
        if history_end > self.memory.len() {
            return Err(DecodeError::InvalidConfig);
        }

        let (frame_pool, history_and_after) = self.memory.split_at_mut(first_history.start);
        let current = frame_pool_layout.current;
        let current_end = current.end()?;
        if current.start != 0 || current_end > frame_pool.len() {
            return Err(DecodeError::InvalidConfig);
        }
        let (current_frame_bytes, reference_bytes) = frame_pool.split_at_mut(current_end);
        let current_frame = current_frame_view(
            current_frame_bytes,
            frame_layout,
            request.frame_width,
            request.frame_height,
        )?;
        let reference_frames = match request.reference_slots {
            Some(reference_slots) => Some(reference_frame_views(
                reference_bytes,
                current_end,
                frame_pool_layout,
                frame_layout,
                reference_slots,
            )?),
            None => None,
        };

        let history = history_and_after
            .get_mut(..history_end - first_history.start)
            .ok_or(DecodeError::InvalidConfig)?;
        let mode_buffers = Self::mode_history_buffers_from_slice(
            mode_history,
            history,
            request.use_prev_frame_mvs,
            request.previous_slot,
            request.current_slot,
            request.mi_count,
        )?;

        Ok((current_frame, reference_frames, mode_buffers))
    }

    fn refresh_references_from_current(
        &mut self,
        refresh_frame_flags: u8,
    ) -> Result<(), DecodeError> {
        for index in 0..REFERENCE_FRAME_SLOTS {
            if refresh_frame_flags & (1u8 << index) != 0 {
                self.copy_current_to_reference(index)?;
            }
        }
        Ok(())
    }

    fn current_i420_frame(&self, info: FrameInfo) -> Result<I420Frame<'_>, DecodeError> {
        self.i420_frame(FramePoolSlot::Current, info)
    }

    fn reference_i420_frame(
        &self,
        index: usize,
        info: FrameInfo,
    ) -> Result<I420Frame<'_>, DecodeError> {
        self.i420_frame(FramePoolSlot::Reference(index), info)
    }

    fn i420_frame(
        &self,
        slot: FramePoolSlot,
        info: FrameInfo,
    ) -> Result<I420Frame<'_>, DecodeError> {
        let frame_layout = self.layout.frame_pool.frame;
        let frame = self.frame_slot(slot)?;
        let chroma_width = info.visible_width.div_ceil(2);
        let chroma_height = info.visible_height.div_ceil(2);

        Ok(I420Frame {
            info,
            y: frame_plane(
                frame,
                frame_layout.y,
                PlaneShape::new(
                    info.visible_width,
                    info.visible_height,
                    frame_layout.y.shape.stride,
                ),
            )?,
            u: frame_plane(
                frame,
                frame_layout.u,
                PlaneShape::new(chroma_width, chroma_height, frame_layout.u.shape.stride),
            )?,
            v: frame_plane(
                frame,
                frame_layout.v,
                PlaneShape::new(chroma_width, chroma_height, frame_layout.v.shape.stride),
            )?,
        })
    }

    fn frame_slot(&self, slot: FramePoolSlot) -> Result<&[u8], DecodeError> {
        let range = self.frame_slot_range(slot)?;
        let end = range.end()?;
        self.memory
            .get(range.start..end)
            .ok_or(DecodeError::InvalidConfig)
    }

    fn frame_slot_mut(&mut self, slot: FramePoolSlot) -> Result<&mut [u8], DecodeError> {
        let range = self.frame_slot_range(slot)?;
        let end = range.end()?;
        self.memory
            .get_mut(range.start..end)
            .ok_or(DecodeError::InvalidConfig)
    }

    fn frame_slot_range(&self, slot: FramePoolSlot) -> Result<ByteRange, DecodeError> {
        match slot {
            FramePoolSlot::Current => Ok(self.layout.frame_pool.current),
            FramePoolSlot::Reference(index) => self
                .layout
                .frame_pool
                .references
                .get(index)
                .copied()
                .ok_or(DecodeError::InvalidBitstream),
        }
    }

    fn copy_current_to_reference(&mut self, index: usize) -> Result<(), DecodeError> {
        let current = self.layout.frame_pool.current;
        let reference = self
            .layout
            .frame_pool
            .references
            .get(index)
            .copied()
            .ok_or(DecodeError::InvalidBitstream)?;
        let current_end = current.end()?;
        let reference_end = reference.end()?;
        if current_end > reference.start || reference_end > self.memory.len() {
            return Err(DecodeError::InvalidConfig);
        }

        let (before_reference, reference_and_after) = self.memory.split_at_mut(reference.start);
        let current_frame = before_reference
            .get(current.start..current_end)
            .ok_or(DecodeError::InvalidConfig)?;
        let reference_frame = reference_and_after
            .get_mut(..reference.len)
            .ok_or(DecodeError::InvalidConfig)?;
        reference_frame.copy_from_slice(current_frame);
        Ok(())
    }

    fn mode_history_buffers_from_slice<'b>(
        layout: ModeHistoryLayout,
        history: &'b mut [u8],
        use_prev_frame_mvs: bool,
        previous_slot: Option<ModeHistorySlot>,
        current_slot: ModeHistorySlot,
        mi_count: usize,
    ) -> Result<FrameModeBuffers<'b>, DecodeError> {
        let (previous, mut current) = Self::mode_history_views_from_slice(
            layout,
            history,
            previous_slot,
            current_slot,
            mi_count,
        )?;
        current.clear();
        Ok(FrameModeBuffers::new(
            use_prev_frame_mvs && previous.is_some(),
            previous,
            Some(current),
        ))
    }

    fn mode_history_views_from_slice<'b>(
        layout: ModeHistoryLayout,
        history: &'b mut [u8],
        previous_slot: Option<ModeHistorySlot>,
        current_slot: ModeHistorySlot,
        mi_count: usize,
    ) -> Result<(Option<ModeInfoView<'b>>, ModeInfoViewMut<'b>), DecodeError> {
        if previous_slot == Some(current_slot) {
            return Err(DecodeError::InvalidConfig);
        }

        let frame_bytes = mode_info_byte_len(mi_count).ok_or(DecodeError::InvalidBitstream)?;
        let slot_len = layout.slot_len();
        if frame_bytes > slot_len {
            return Err(DecodeError::ResourceLimit);
        }

        let first = layout.slots[0];
        let second = layout.slots[1];
        debug_assert_eq!(first.len, second.len);
        debug_assert_eq!(first.end().ok(), Some(second.start));
        if first.end()? != second.start {
            return Err(DecodeError::InvalidConfig);
        }

        let base = first.start;
        let history_end = second.end()?;
        let history_len = history_end
            .checked_sub(base)
            .ok_or(DecodeError::InvalidConfig)?;
        if history_len > history.len() {
            return Err(DecodeError::InvalidConfig);
        }

        let (first_full, history_after_first) = history.split_at_mut(first.len);
        let (second_full, _) = history_after_first.split_at_mut(second.len);
        let first_frame = &mut first_full[..frame_bytes];
        let second_frame = &mut second_full[..frame_bytes];

        match (previous_slot, current_slot) {
            (None, ModeHistorySlot::Slot0) => Ok((None, ModeInfoViewMut::new(first_frame)?)),
            (None, ModeHistorySlot::Slot1) => Ok((None, ModeInfoViewMut::new(second_frame)?)),
            (Some(ModeHistorySlot::Slot0), ModeHistorySlot::Slot1) => Ok((
                Some(ModeInfoView::new(first_frame)?),
                ModeInfoViewMut::new(second_frame)?,
            )),
            (Some(ModeHistorySlot::Slot1), ModeHistorySlot::Slot0) => Ok((
                Some(ModeInfoView::new(second_frame)?),
                ModeInfoViewMut::new(first_frame)?,
            )),
            (Some(_), _) => Err(DecodeError::InvalidConfig),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FramePoolSlot {
    Current,
    Reference(usize),
}

fn fill_frame_plane(frame: &mut [u8], layout: PlaneLayout, value: u8) -> Result<(), DecodeError> {
    let end = layout.end()?;
    let plane = frame
        .get_mut(layout.offset..end)
        .ok_or(DecodeError::InvalidConfig)?;
    plane.fill(value);
    Ok(())
}

fn current_frame_view(
    frame: &mut [u8],
    layout: FrameLayout,
    width: u32,
    height: u32,
) -> Result<CurrentFrameMut<'_>, DecodeError> {
    let y_len = layout.y.len();
    let u_len = layout.u.len();
    let v_len = layout.v.len();
    let y_end = layout.y.end()?;
    let u_end = layout.u.end()?;
    if layout.y.offset != 0 || layout.u.offset != y_end || layout.v.offset != u_end {
        return Err(DecodeError::InvalidConfig);
    }

    let (y_data, after_y) = frame.split_at_mut(y_len);
    let (u_data, after_u) = after_y.split_at_mut(u_len);
    let (v_data, _) = after_u.split_at_mut(v_len);
    // CurrFrame is addressed by intra prediction and loop filtering using
    // MI-rounded dimensions. This preserves reconstructed samples in partial
    // right/bottom blocks for later prediction/filtering, while the public
    // I420 output path slices back to the visible frame size.
    let padded_width = mi_aligned_pixels(width).ok_or(DecodeError::InvalidConfig)?;
    let padded_height = mi_aligned_pixels(height).ok_or(DecodeError::InvalidConfig)?;
    let chroma_width = padded_width / 2;
    let chroma_height = padded_height / 2;

    let y = CurrentPlaneMut::new(
        y_data,
        PlaneShape::new(padded_width, padded_height, layout.y.shape.stride),
    )
    .map_err(|_| DecodeError::InvalidConfig)?;
    let u = CurrentPlaneMut::new(
        u_data,
        PlaneShape::new(chroma_width, chroma_height, layout.u.shape.stride),
    )
    .map_err(|_| DecodeError::InvalidConfig)?;
    let v = CurrentPlaneMut::new(
        v_data,
        PlaneShape::new(chroma_width, chroma_height, layout.v.shape.stride),
    )
    .map_err(|_| DecodeError::InvalidConfig)?;

    Ok(CurrentFrameMut::new(y, u, v))
}

fn reference_frame_views<'a>(
    reference_bytes: &'a [u8],
    reference_base: usize,
    frame_pool_layout: FramePoolLayout,
    frame_layout: FrameLayout,
    reference_slots: [InterReferenceSlot; 3],
) -> Result<ReferenceFrames<'a>, DecodeError> {
    let mut frames = [None; 4];
    for (logical_index, reference_slot) in reference_slots.into_iter().enumerate() {
        let range = frame_pool_layout
            .references
            .get(reference_slot.slot_index)
            .copied()
            .ok_or(DecodeError::InvalidBitstream)?;
        let start = range
            .start
            .checked_sub(reference_base)
            .ok_or(DecodeError::InvalidConfig)?;
        let end = start
            .checked_add(range.len)
            .ok_or(DecodeError::InvalidConfig)?;
        let frame = reference_bytes
            .get(start..end)
            .ok_or(DecodeError::InvalidConfig)?;
        frames[logical_index + 1] = Some(reference_frame_view(
            frame,
            frame_layout,
            reference_slot.info.visible_width,
            reference_slot.info.visible_height,
        )?);
    }

    Ok(ReferenceFrames::new(frames))
}

fn reference_frame_view(
    frame: &[u8],
    layout: FrameLayout,
    width: u32,
    height: u32,
) -> Result<ReferenceFrame<'_>, DecodeError> {
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);

    let y = reference_plane(
        frame,
        layout.y,
        PlaneShape::new(width, height, layout.y.shape.stride),
    )?;
    let u = reference_plane(
        frame,
        layout.u,
        PlaneShape::new(chroma_width, chroma_height, layout.u.shape.stride),
    )?;
    let v = reference_plane(
        frame,
        layout.v,
        PlaneShape::new(chroma_width, chroma_height, layout.v.shape.stride),
    )?;

    Ok(ReferenceFrame::new(y, u, v))
}

fn reference_plane(
    frame: &[u8],
    layout: PlaneLayout,
    shape: PlaneShape,
) -> Result<ReferencePlane<'_>, DecodeError> {
    let len = shape.byte_len().ok_or(DecodeError::ResourceLimit)?;
    let end = layout
        .offset
        .checked_add(len)
        .ok_or(DecodeError::InvalidConfig)?;
    let data = frame
        .get(layout.offset..end)
        .ok_or(DecodeError::InvalidConfig)?;
    ReferencePlane::new(data, shape).map_err(|_| DecodeError::InvalidConfig)
}

fn frame_plane(
    frame: &[u8],
    layout: PlaneLayout,
    shape: PlaneShape,
) -> Result<Plane<'_>, DecodeError> {
    let len = shape.byte_len().ok_or(DecodeError::ResourceLimit)?;
    let end = layout
        .offset
        .checked_add(len)
        .ok_or(DecodeError::InvalidConfig)?;
    let data = frame
        .get(layout.offset..end)
        .ok_or(DecodeError::InvalidConfig)?;
    Ok(Plane { data, shape })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameInfo {
    pub visible_width: u32,
    pub visible_height: u32,
    pub render_width: u32,
    pub render_height: u32,
    pub frame_index: u64,
}

impl FrameInfo {
    pub fn i420(
        visible_width: u32,
        visible_height: u32,
        render_width: u32,
        render_height: u32,
        frame_index: u64,
    ) -> Option<Self> {
        required_i420_len(visible_width, visible_height)?;
        Some(Self {
            visible_width,
            visible_height,
            render_width,
            render_height,
            frame_index,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaneShape {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
}

impl PlaneShape {
    pub const fn new(width: u32, height: u32, stride: usize) -> Self {
        Self {
            width,
            height,
            stride,
        }
    }

    pub fn byte_len(self) -> Option<usize> {
        let height = usize::try_from(self.height).ok()?;
        self.stride.checked_mul(height)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Plane<'a> {
    pub data: &'a [u8],
    pub shape: PlaneShape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct I420Frame<'a> {
    pub info: FrameInfo,
    pub y: Plane<'a>,
    pub u: Plane<'a>,
    pub v: Plane<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeOutcome<'a> {
    NoOutput,
    Output(I420Frame<'a>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    InvalidConfig,
    ResourceLimit,
    UnsupportedProfile(u8),
    UnsupportedBitDepth(u8),
    InvalidBitstream,
    Unimplemented,
}

impl DecodeError {
    pub const fn code(&self) -> i32 {
        match self {
            Self::InvalidConfig => -1,
            Self::ResourceLimit => -3,
            Self::UnsupportedProfile(_) => -4,
            Self::UnsupportedBitDepth(_) => -5,
            Self::InvalidBitstream => -6,
            Self::Unimplemented => -8,
        }
    }
}

#[derive(Debug)]
pub struct Decoder {
    layout: WorkspaceLayout,
    header_state: HeaderParserState,
    probability_state: ProbabilityState,
    syntax_counts: SyntaxCounts,
    reference_frames: [Option<ReferenceSlotInfo>; REFERENCE_FRAME_SLOTS],
    next_output_frame_index: u64,
    last_frame_type: header::FrameType,
    previous_frame_for_mvs: Option<PreviousFrameForMvs>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ReferenceSlotInfo {
    visible_width: u32,
    visible_height: u32,
    render_width: u32,
    render_height: u32,
}

impl ReferenceSlotInfo {
    fn from_header(header: &header::UncompressedFrameHeader) -> Self {
        Self {
            visible_width: header.frame_width,
            visible_height: header.frame_height,
            render_width: header.render_width,
            render_height: header.render_height,
        }
    }

    fn frame_info(self, frame_index: u64) -> Result<FrameInfo, DecodeError> {
        FrameInfo::i420(
            self.visible_width,
            self.visible_height,
            self.render_width,
            self.render_height,
            frame_index,
        )
        .ok_or(DecodeError::ResourceLimit)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InterReferenceSlot {
    slot_index: usize,
    info: ReferenceSlotInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PreviousFrameForMvs {
    width: u32,
    height: u32,
    show_frame: bool,
    mode_history_slot: ModeHistorySlot,
}

impl Decoder {
    pub fn new(layout: WorkspaceLayout) -> Self {
        Self {
            layout,
            header_state: HeaderParserState::new(),
            probability_state: ProbabilityState::new(),
            syntax_counts: SyntaxCounts::default(),
            reference_frames: [None; REFERENCE_FRAME_SLOTS],
            next_output_frame_index: 0,
            last_frame_type: header::FrameType::Key,
            previous_frame_for_mvs: None,
        }
    }

    pub fn decode_coded_frame<'w>(
        &mut self,
        coded_frame: &[u8],
        workspace: &'w mut DecodeWorkspace<'_>,
    ) -> Result<DecodeOutcome<'w>, DecodeError> {
        workspace.require_layout(self.layout)?;
        let mut parsed_header_state = self.header_state;
        let header = parse_uncompressed_frame_header(coded_frame, &mut parsed_header_state)
            .map_err(|err| err.into_decode_error())?;
        self.validate_frame_limits(&header)?;
        self.setup_frame_probability_state(&header)?;

        if header.show_existing_frame {
            self.header_state = parsed_header_state;
            self.header_state.update_references(&header);
            let reference_index = usize::from(
                header
                    .frame_to_show_map_idx
                    .ok_or(DecodeError::InvalidBitstream)?,
            );
            let reference = self
                .reference_frames
                .get(reference_index)
                .and_then(|reference| *reference)
                .ok_or(DecodeError::InvalidBitstream)?;
            return self.output_reference_frame(workspace, reference_index, reference);
        }

        if header.header_size_in_bytes == 0 {
            self.header_state = parsed_header_state;
            self.header_state.update_references(&header);
            self.last_frame_type = header.frame_type;
            self.previous_frame_for_mvs = None;
            return Ok(DecodeOutcome::NoOutput);
        }

        let compressed_header_data = coded_frame
            .get(header.compressed_header_offset..header.tile_data_offset)
            .ok_or(DecodeError::InvalidBitstream)?;
        self.probability_state
            .load_probs(header.frame_context_idx)
            .map_err(|err| err.into_decode_error())?;
        self.probability_state
            .load_probs2(header.frame_context_idx)
            .map_err(|err| err.into_decode_error())?;

        let compressed_header = if header.frame_is_intra {
            parse_intra_compressed_header(
                compressed_header_data,
                &header,
                self.probability_state.current_mut(),
            )
            .map_err(|err| err.into_decode_error())?
        } else {
            parse_inter_compressed_header(
                compressed_header_data,
                &header,
                self.probability_state.current_mut(),
            )
            .map_err(|err| err.into_decode_error())?
        };
        let tile_layout =
            parse_tile_layout(coded_frame, &header).map_err(|err| err.into_decode_error())?;
        self.syntax_counts.clear();
        let mi_count = frame_mi_count(header.frame_width, header.frame_height)?;
        let previous_slot_for_mvs = self.use_prev_frame_mvs(&header);
        // Previous MVs can only be used from a shown same-sized frame, but
        // segmentation maps persist across hidden frames too.  Keep the
        // previous mode grid available for segment-id prediction independently
        // from MV reuse.
        let previous_slot = self.previous_mode_history_slot(&header);
        let current_slot = self
            .previous_frame_for_mvs
            .map(|previous| previous.mode_history_slot.other())
            .unwrap_or(ModeHistorySlot::Slot0);
        let reference_slots = self.inter_reference_slots(&header)?;
        {
            let (mut current_frame, reference_frames, mode_buffers) = workspace
                .reconstruction_buffers(ReconstructionBufferRequest {
                    frame_width: header.frame_width,
                    frame_height: header.frame_height,
                    reference_slots,
                    use_prev_frame_mvs: previous_slot_for_mvs.is_some(),
                    previous_slot,
                    current_slot,
                    mi_count,
                })?;

            let tile_parse_result = if header.frame_is_intra {
                parse_intra_tiles(
                    coded_frame,
                    &header,
                    &compressed_header,
                    self.probability_state.current(),
                    &tile_layout,
                    TileParseBuffers::new(
                        &mut self.syntax_counts,
                        mode_buffers,
                        &mut current_frame,
                    ),
                )
            } else {
                let reference_frames = reference_frames.ok_or(DecodeError::InvalidBitstream)?;
                parse_inter_tiles(
                    coded_frame,
                    &header,
                    &compressed_header,
                    self.probability_state.current(),
                    &tile_layout,
                    TileParseBuffers::with_references(
                        &mut self.syntax_counts,
                        mode_buffers,
                        &mut current_frame,
                        reference_frames,
                    ),
                )
            };

            tile_parse_result.map_err(|err| err.into_decode_error())?;
        }

        self.refresh_probability_state(&header, &compressed_header)?;

        let current_reference = ReferenceSlotInfo::from_header(&header);
        workspace.refresh_references_from_current(header.refresh_frame_flags)?;
        self.refresh_reference_info(header.refresh_frame_flags, current_reference);
        self.header_state = parsed_header_state;
        self.header_state.update_references(&header);
        self.last_frame_type = header.frame_type;
        self.previous_frame_for_mvs = Some(PreviousFrameForMvs {
            width: header.frame_width,
            height: header.frame_height,
            show_frame: header.show_frame,
            mode_history_slot: current_slot,
        });

        if header.show_frame {
            return self.output_current_frame(workspace, current_reference);
        }
        Ok(DecodeOutcome::NoOutput)
    }

    fn refresh_reference_info(&mut self, refresh_frame_flags: u8, reference: ReferenceSlotInfo) {
        for (index, slot) in self.reference_frames.iter_mut().enumerate() {
            if refresh_frame_flags & (1u8 << index) != 0 {
                *slot = Some(reference);
            }
        }
    }

    fn output_current_frame<'w>(
        &mut self,
        workspace: &'w DecodeWorkspace<'_>,
        reference: ReferenceSlotInfo,
    ) -> Result<DecodeOutcome<'w>, DecodeError> {
        let (info, next_output_frame_index) = self.next_output_frame_info(reference)?;
        let frame = workspace.current_i420_frame(info)?;
        self.next_output_frame_index = next_output_frame_index;
        Ok(DecodeOutcome::Output(frame))
    }

    fn output_reference_frame<'w>(
        &mut self,
        workspace: &'w DecodeWorkspace<'_>,
        index: usize,
        reference: ReferenceSlotInfo,
    ) -> Result<DecodeOutcome<'w>, DecodeError> {
        let (info, next_output_frame_index) = self.next_output_frame_info(reference)?;
        let frame = workspace.reference_i420_frame(index, info)?;
        self.next_output_frame_index = next_output_frame_index;
        Ok(DecodeOutcome::Output(frame))
    }

    fn next_output_frame_info(
        &self,
        reference: ReferenceSlotInfo,
    ) -> Result<(FrameInfo, u64), DecodeError> {
        let frame_index = self.next_output_frame_index;
        let next_output_frame_index = self
            .next_output_frame_index
            .checked_add(1)
            .ok_or(DecodeError::ResourceLimit)?;
        Ok((reference.frame_info(frame_index)?, next_output_frame_index))
    }

    fn validate_frame_limits(
        &self,
        header: &header::UncompressedFrameHeader,
    ) -> Result<(), DecodeError> {
        if header.frame_width > self.layout.max_width()
            || header.frame_height > self.layout.max_height()
            || header.render_width == 0
            || header.render_height == 0
            || required_i420_len(header.frame_width, header.frame_height).is_none()
        {
            return Err(DecodeError::ResourceLimit);
        }
        Ok(())
    }

    fn use_prev_frame_mvs(
        &self,
        header: &header::UncompressedFrameHeader,
    ) -> Option<ModeHistorySlot> {
        if header.error_resilient_mode || header.frame_is_intra || header.show_existing_frame {
            return None;
        }
        let previous = self.previous_frame_for_mvs?;
        (previous.width == header.frame_width
            && previous.height == header.frame_height
            && previous.show_frame)
            .then_some(previous.mode_history_slot)
    }

    fn previous_mode_history_slot(
        &self,
        header: &header::UncompressedFrameHeader,
    ) -> Option<ModeHistorySlot> {
        if header.frame_is_intra || header.show_existing_frame {
            return None;
        }
        let previous = self.previous_frame_for_mvs?;
        (previous.width == header.frame_width && previous.height == header.frame_height)
            .then_some(previous.mode_history_slot)
    }

    fn inter_reference_slots(
        &self,
        header: &header::UncompressedFrameHeader,
    ) -> Result<Option<[InterReferenceSlot; 3]>, DecodeError> {
        if header.frame_is_intra || header.show_existing_frame {
            return Ok(None);
        }

        let mut slots = [InterReferenceSlot {
            slot_index: 0,
            info: ReferenceSlotInfo {
                visible_width: 0,
                visible_height: 0,
                render_width: 0,
                render_height: 0,
            },
        }; 3];

        for (index, slot) in slots.iter_mut().enumerate() {
            let slot_index = usize::from(header.ref_frame_idx[index]);
            let info = self
                .reference_frames
                .get(slot_index)
                .and_then(|reference| *reference)
                .ok_or(DecodeError::InvalidBitstream)?;
            *slot = InterReferenceSlot { slot_index, info };
        }

        Ok(Some(slots))
    }

    fn setup_frame_probability_state(
        &mut self,
        header: &header::UncompressedFrameHeader,
    ) -> Result<(), DecodeError> {
        if !header.frame_is_intra && !header.error_resilient_mode {
            return Ok(());
        }

        self.probability_state.setup_past_independence();
        if header.frame_type == header::FrameType::Key
            || header.error_resilient_mode
            || header.reset_frame_context == 3
        {
            self.probability_state.reset_all_contexts();
        } else if header.reset_frame_context == 2 {
            self.probability_state
                .save_probs(header.raw_frame_context_idx)
                .map_err(|err| err.into_decode_error())?;
        }
        Ok(())
    }

    fn refresh_probability_state(
        &mut self,
        header: &header::UncompressedFrameHeader,
        compressed_header: &CompressedHeader,
    ) -> Result<(), DecodeError> {
        if !header.error_resilient_mode && !header.frame_parallel_decoding_mode {
            self.probability_state
                .load_probs(header.frame_context_idx)
                .map_err(|err| err.into_decode_error())?;
            let coef_update_factor = if header.frame_is_intra {
                112
            } else if self.last_frame_type == header::FrameType::Key {
                128
            } else {
                112
            };
            self.probability_state
                .adapt_coef_probs(&self.syntax_counts, coef_update_factor);
            if !header.frame_is_intra {
                self.probability_state
                    .load_probs2(header.frame_context_idx)
                    .map_err(|err| err.into_decode_error())?;
                self.probability_state.adapt_noncoef_probs(
                    &self.syntax_counts,
                    NonCoefAdaptationConfig {
                        tx_mode_select: compressed_header.tx_mode == TxMode::Select,
                        interpolation_filter_switchable: matches!(
                            header.interpolation_filter,
                            Some(header::InterpolationFilter::Switchable)
                        ),
                        allow_high_precision_mv: header.allow_high_precision_mv,
                    },
                );
            }
        }

        if header.refresh_frame_context {
            self.probability_state
                .save_probs(header.frame_context_idx)
                .map_err(|err| err.into_decode_error())?;
        }
        Ok(())
    }
}

fn required_i420_len(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 {
        return None;
    }

    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    let luma = width.checked_mul(height)?;
    let chroma_width = width / 2 + width % 2;
    let chroma_height = height / 2 + height % 2;
    let chroma_plane = chroma_width.checked_mul(chroma_height)?;
    luma.checked_add(chroma_plane.checked_mul(2)?)
}

fn frame_mi_count(width: u32, height: u32) -> Result<usize, DecodeError> {
    let mi_cols = width.checked_add(7).ok_or(DecodeError::InvalidBitstream)? >> 3;
    let mi_rows = height.checked_add(7).ok_or(DecodeError::InvalidBitstream)? >> 3;
    let mi_cols = usize::try_from(mi_cols).map_err(|_| DecodeError::InvalidBitstream)?;
    let mi_rows = usize::try_from(mi_rows).map_err(|_| DecodeError::InvalidBitstream)?;
    mi_rows
        .checked_mul(mi_cols)
        .ok_or(DecodeError::InvalidBitstream)
}

fn mi_aligned_pixels(pixels: u32) -> Option<u32> {
    pixels.checked_add(7).map(|value| value & !7)
}

fn validate_limits(max_width: u32, max_height: u32) -> Result<(), DecodeError> {
    if max_width == 0 || max_height == 0 || required_i420_len(max_width, max_height).is_none() {
        return Err(DecodeError::InvalidConfig);
    }
    Ok(())
}

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::{
        DecodeError, DecodeOutcome, DecodeWorkspace, Decoder, I420Frame, PlaneShape,
        WorkspaceLayout, required_i420_len,
    };

    #[test]
    fn i420_len_counts_luma_and_two_quarter_chroma_planes() {
        assert_eq!(required_i420_len(1280, 720), Some(1_382_400));
    }

    #[test]
    fn i420_len_rounds_chroma_planes_up_for_odd_sizes() {
        assert_eq!(required_i420_len(3, 3), Some(17));
    }

    #[test]
    fn i420_len_rejects_zero_dimensions() {
        assert_eq!(required_i420_len(0, 1), None);
        assert_eq!(required_i420_len(1, 0), None);
    }

    #[test]
    fn workspace_layout_rejects_zero_max_dimensions() {
        assert_eq!(
            WorkspaceLayout::new(0, 720).unwrap_err(),
            super::DecodeError::InvalidConfig
        );
    }

    #[test]
    fn workspace_layout_is_frame_pool_plus_two_mode_history_slots() {
        let layout = WorkspaceLayout::new(16, 16).unwrap();
        let frame_pool_bytes = 16 * 16 * 3 / 2 * 9;
        let mode_history_slot_bytes = 4 * super::tile_syntax::STORED_MODE_INFO_BYTES;

        assert_eq!(layout.max_width(), 16);
        assert_eq!(layout.max_height(), 16);
        assert_eq!(
            layout.total_bytes(),
            frame_pool_bytes + 2 * mode_history_slot_bytes
        );
        assert_eq!(layout.frame_pool.current.start, 0);
        assert_eq!(layout.frame_pool.current.len, 16 * 16 * 3 / 2);
        assert_eq!(layout.frame_pool.references[0].start, 16 * 16 * 3 / 2);
        assert_eq!(
            layout.frame_pool.references[7].end().unwrap(),
            frame_pool_bytes
        );
        assert_eq!(layout.mode_history.slots[0].start, frame_pool_bytes);
        assert_eq!(layout.mode_history.slots[0].len, mode_history_slot_bytes);
        assert_eq!(
            layout.mode_history.slots[1].start,
            frame_pool_bytes + mode_history_slot_bytes
        );
        assert_eq!(layout.mode_history.slots[1].len, mode_history_slot_bytes);
        assert_eq!(
            layout.mode_history.slots[1].end().unwrap(),
            layout.total_bytes()
        );
        assert_eq!(layout.frame_pool.frame.y.offset, 0);
        assert_eq!(layout.frame_pool.frame.y.shape.stride, 16);
        assert_eq!(layout.frame_pool.frame.y.len(), 16 * 16);
        assert_eq!(layout.frame_pool.frame.u.offset, 16 * 16);
        assert_eq!(layout.frame_pool.frame.u.shape.stride, 8);
        assert_eq!(layout.frame_pool.frame.u.len(), 8 * 8);
        assert_eq!(layout.frame_pool.frame.v.offset, 16 * 16 + 8 * 8);
        assert_eq!(layout.frame_pool.frame.v.shape.stride, 8);
        assert_eq!(layout.frame_pool.frame.v.len(), 8 * 8);
    }

    #[test]
    fn non_mi_multiple_layout_keeps_padded_reconstruction_planes() {
        let layout = WorkspaceLayout::new(17, 9).unwrap();
        let frame_bytes = 24 * 16 + 2 * 12 * 8;
        let mode_history_slot_bytes = 6 * super::tile_syntax::STORED_MODE_INFO_BYTES;

        assert_eq!(layout.max_width(), 17);
        assert_eq!(layout.max_height(), 9);
        assert_eq!(layout.frame_pool.frame.y.shape, PlaneShape::new(24, 16, 24));
        assert_eq!(layout.frame_pool.frame.u.shape, PlaneShape::new(12, 8, 12));
        assert_eq!(layout.frame_pool.frame.v.shape, PlaneShape::new(12, 8, 12));
        assert_eq!(layout.frame_pool.current.len, frame_bytes);
        assert_eq!(
            layout.total_bytes(),
            frame_bytes * 9 + 2 * mode_history_slot_bytes
        );
    }

    #[test]
    fn decode_workspace_rejects_undersized_arena() {
        let layout = WorkspaceLayout::new(16, 16).unwrap();
        let mut memory = [0; TEST_WORKSPACE_BYTES];
        let undersized_len = layout.total_bytes() - 1;

        assert_eq!(
            DecodeWorkspace::new(layout, &mut memory[..undersized_len]).unwrap_err(),
            DecodeError::ResourceLimit
        );
    }

    #[test]
    fn decode_coded_frame_rejects_workspace_for_other_layout() {
        let decoder_layout = WorkspaceLayout::new(16, 16).unwrap();
        let workspace_layout = WorkspaceLayout::new(32, 16).unwrap();
        let mut decoder = Decoder::new(decoder_layout);
        let mut test_workspace = TestWorkspace::new();
        let mut workspace = test_workspace.as_workspace(workspace_layout);

        assert_eq!(
            decoder.decode_coded_frame(&[], &mut workspace),
            Err(DecodeError::InvalidConfig)
        );
    }

    #[test]
    fn decode_coded_frame_outputs_default_i420_after_valid_shown_intra_tile_parse() {
        let frame = minimal_lossless_key_frame_with_size(13, 15);
        let layout = WorkspaceLayout::new(16, 16).unwrap();
        let mut decoder = Decoder::new(layout);
        let mut test_workspace = TestWorkspace::new();
        let mut workspace = test_workspace.as_workspace(layout);

        let outcome = decoder.decode_coded_frame(&frame, &mut workspace).unwrap();
        let DecodeOutcome::Output(frame) = outcome else {
            panic!("shown key frame should output");
        };
        assert_default_i420_frame(
            frame,
            ExpectedFrame {
                visible_width: 13,
                visible_height: 15,
                render_width: 13,
                render_height: 15,
                frame_index: 0,
                y_stride: 16,
                uv_stride: 8,
            },
        );
    }

    #[test]
    fn decode_coded_frame_outputs_visible_i420_from_padded_layout() {
        let frame = minimal_lossless_key_frame_with_size(17, 9);
        let layout = WorkspaceLayout::new(17, 9).unwrap();
        let mut decoder = Decoder::new(layout);
        let mut test_workspace = TestWorkspace::new();
        let mut workspace = test_workspace.as_workspace(layout);

        let outcome = decoder.decode_coded_frame(&frame, &mut workspace).unwrap();
        let DecodeOutcome::Output(frame) = outcome else {
            panic!("shown key frame should output");
        };
        assert_default_i420_frame(
            frame,
            ExpectedFrame {
                visible_width: 17,
                visible_height: 9,
                render_width: 17,
                render_height: 9,
                frame_index: 0,
                y_stride: 24,
                uv_stride: 12,
            },
        );
    }

    #[test]
    fn decode_coded_frame_refreshes_inter_frame_probabilities_after_tile_parse() {
        let key_frame = minimal_lossless_key_frame();
        let inter_frame = minimal_lossless_inter_frame();
        let layout = WorkspaceLayout::new(16, 16).unwrap();
        let mut decoder = Decoder::new(layout);
        let mut test_workspace = TestWorkspace::new();
        let mut workspace = test_workspace.as_workspace(layout);

        assert!(matches!(
            decoder.decode_coded_frame(&key_frame, &mut workspace),
            Ok(DecodeOutcome::Output(_))
        ));
        assert!(matches!(
            decoder.decode_coded_frame(&inter_frame, &mut workspace),
            Ok(DecodeOutcome::Output(_))
        ));
    }

    #[test]
    fn decode_coded_frame_outputs_refreshed_slot_for_show_existing_frame() {
        let key_frame = minimal_lossless_key_frame();
        let show_existing = show_existing_frame(0);
        let layout = WorkspaceLayout::new(16, 16).unwrap();
        let mut decoder = Decoder::new(layout);
        let mut test_workspace = TestWorkspace::new();
        let mut workspace = test_workspace.as_workspace(layout);

        assert!(matches!(
            decoder.decode_coded_frame(&key_frame, &mut workspace),
            Ok(DecodeOutcome::Output(_))
        ));

        let outcome = decoder
            .decode_coded_frame(&show_existing, &mut workspace)
            .unwrap();
        let DecodeOutcome::Output(frame) = outcome else {
            panic!("show_existing_frame should output");
        };
        assert_default_i420_frame(
            frame,
            ExpectedFrame {
                visible_width: 16,
                visible_height: 16,
                render_width: 16,
                render_height: 16,
                frame_index: 1,
                y_stride: 16,
                uv_stride: 8,
            },
        );
    }

    #[test]
    fn decode_coded_frame_refreshes_hidden_frame_without_output() {
        let hidden_frame = minimal_lossless_key_frame_with_size_and_show(16, 16, false);
        let show_existing = show_existing_frame(0);
        let layout = WorkspaceLayout::new(16, 16).unwrap();
        let mut decoder = Decoder::new(layout);
        let mut test_workspace = TestWorkspace::new();
        let mut workspace = test_workspace.as_workspace(layout);

        assert_eq!(
            decoder.decode_coded_frame(&hidden_frame, &mut workspace),
            Ok(DecodeOutcome::NoOutput)
        );

        let outcome = decoder
            .decode_coded_frame(&show_existing, &mut workspace)
            .unwrap();
        let DecodeOutcome::Output(frame) = outcome else {
            panic!("show_existing_frame should output refreshed hidden frame");
        };
        assert_default_i420_frame(
            frame,
            ExpectedFrame {
                visible_width: 16,
                visible_height: 16,
                render_width: 16,
                render_height: 16,
                frame_index: 0,
                y_stride: 16,
                uv_stride: 8,
            },
        );
    }

    const TEST_WORKSPACE_BYTES: usize = 8192;

    struct TestWorkspace {
        memory: [u8; TEST_WORKSPACE_BYTES],
    }

    impl TestWorkspace {
        const fn new() -> Self {
            Self {
                memory: [0; TEST_WORKSPACE_BYTES],
            }
        }

        fn as_workspace(&mut self, layout: WorkspaceLayout) -> DecodeWorkspace<'_> {
            let len = layout.total_bytes();
            DecodeWorkspace::new(layout, &mut self.memory[..len])
                .expect("test workspace should fit known small layouts")
        }
    }

    #[derive(Clone, Copy)]
    struct ExpectedFrame {
        visible_width: u32,
        visible_height: u32,
        render_width: u32,
        render_height: u32,
        frame_index: u64,
        y_stride: usize,
        uv_stride: usize,
    }

    fn assert_default_i420_frame(frame: I420Frame<'_>, expected: ExpectedFrame) {
        assert_eq!(frame.info.visible_width, expected.visible_width);
        assert_eq!(frame.info.visible_height, expected.visible_height);
        assert_eq!(frame.info.render_width, expected.render_width);
        assert_eq!(frame.info.render_height, expected.render_height);
        assert_eq!(frame.info.frame_index, expected.frame_index);

        let chroma_width = expected.visible_width.div_ceil(2);
        let chroma_height = expected.visible_height.div_ceil(2);
        assert_eq!(
            frame.y.shape,
            PlaneShape::new(
                expected.visible_width,
                expected.visible_height,
                expected.y_stride,
            )
        );
        assert_eq!(
            frame.u.shape,
            PlaneShape::new(chroma_width, chroma_height, expected.uv_stride)
        );
        assert_eq!(
            frame.v.shape,
            PlaneShape::new(chroma_width, chroma_height, expected.uv_stride)
        );
        assert_eq!(
            frame.y.data.len(),
            expected.y_stride * expected.visible_height as usize
        );
        assert_eq!(
            frame.u.data.len(),
            expected.uv_stride * chroma_height as usize
        );
        assert_eq!(
            frame.v.data.len(),
            expected.uv_stride * chroma_height as usize
        );
        assert!(frame.y.data.iter().all(|&sample| sample == 128));
        assert!(frame.u.data.iter().all(|&sample| sample == 128));
        assert!(frame.v.data.iter().all(|&sample| sample == 128));
    }

    fn minimal_lossless_key_frame() -> [u8; 128] {
        minimal_lossless_key_frame_with_size(16, 16)
    }

    fn minimal_lossless_key_frame_with_size(width: u32, height: u32) -> [u8; 128] {
        minimal_lossless_key_frame_with_size_and_show(width, height, true)
    }

    fn minimal_lossless_key_frame_with_size_and_show(
        width: u32,
        height: u32,
        show_frame: bool,
    ) -> [u8; 128] {
        assert!((1..=u32::from(u16::MAX) + 1).contains(&width));
        assert!((1..=u32::from(u16::MAX) + 1).contains(&height));
        let mut builder = HeaderBuilder::new();
        builder.f(0b10, 2); // frame marker
        builder.f(0, 1); // profile low
        builder.f(0, 1); // profile high
        builder.f(0, 1); // not show existing frame
        builder.f(0, 1); // key frame
        builder.f(if show_frame { 1 } else { 0 }, 1); // show frame
        builder.f(0, 1); // not error resilient
        builder.f(0x49, 8);
        builder.f(0x83, 8);
        builder.f(0x42, 8);
        builder.f(1, 3); // BT.601 color space
        builder.f(0, 1); // studio range
        builder.f(width - 1, 16);
        builder.f(height - 1, 16);
        builder.f(0, 1); // render size matches frame size
        builder.f(1, 1); // refresh frame context
        builder.f(0, 1); // frame parallel decoding mode
        builder.f(0, 2); // frame context idx (reset to 0 for intra frames)
        builder.f(0, 6); // loop filter level
        builder.f(0, 3); // loop filter sharpness
        builder.f(0, 1); // loop filter delta disabled
        builder.f(0, 8); // base q idx
        builder.f(0, 1); // y dc delta absent
        builder.f(0, 1); // uv dc delta absent
        builder.f(0, 1); // uv ac delta absent
        builder.f(0, 1); // segmentation disabled
        builder.f(0, 1); // tile rows log2
        builder.f(2, 16); // compressed header size
        builder.byte_align_zero();
        builder.byte(0x00); // compressed header initial BoolValue
        builder.byte(0x00); // compressed header zero padding
        builder.byte(0x00); // one tile payload byte; tile decode is still unimplemented
        builder.finish()
    }

    fn show_existing_frame(frame_to_show_map_idx: u8) -> [u8; 128] {
        assert!(frame_to_show_map_idx < 8);

        let mut builder = HeaderBuilder::new();
        builder.f(0b10, 2); // frame marker
        builder.f(0, 1); // profile low
        builder.f(0, 1); // profile high
        builder.f(1, 1); // show existing frame
        builder.f(u32::from(frame_to_show_map_idx), 3);
        builder.finish()
    }

    fn minimal_lossless_inter_frame() -> [u8; 128] {
        let mut builder = HeaderBuilder::new();
        builder.f(0b10, 2); // frame marker
        builder.f(0, 1); // profile low
        builder.f(0, 1); // profile high
        builder.f(0, 1); // not show existing frame
        builder.f(1, 1); // non-key frame
        builder.f(1, 1); // show frame
        builder.f(0, 1); // not error resilient
        builder.f(0, 2); // reset frame context
        builder.f(1, 8); // refresh reference slot 0
        for ref_idx in 0..3 {
            builder.f(ref_idx, 3); // reference frame index
            builder.f(0, 1); // sign bias
        }
        builder.f(1, 1); // use first reference size
        builder.f(0, 1); // render size matches frame size
        builder.f(0, 1); // quarter-pel motion vectors
        builder.f(0, 1); // raw interpolation filter follows
        builder.f(0, 2); // EIGHTTAP_SMOOTH
        builder.f(1, 1); // refresh frame context
        builder.f(0, 1); // frame parallel decoding mode
        builder.f(0, 2); // frame context idx
        builder.f(0, 6); // loop filter level
        builder.f(0, 3); // loop filter sharpness
        builder.f(0, 1); // loop filter delta disabled
        builder.f(0, 8); // base q idx
        builder.f(0, 1); // y dc delta absent
        builder.f(0, 1); // uv dc delta absent
        builder.f(0, 1); // uv ac delta absent
        builder.f(0, 1); // segmentation disabled
        builder.f(0, 1); // tile rows log2
        builder.f(64, 16); // compressed header size
        builder.byte_align_zero();
        for _ in 0..64 {
            builder.byte(0x00);
        }
        builder.byte(0x00); // one tile payload byte; inter tile syntax is not parsed yet
        builder.finish()
    }

    struct HeaderBuilder {
        data: [u8; 128],
        bit_len: usize,
    }

    impl HeaderBuilder {
        fn new() -> Self {
            Self {
                data: [0; 128],
                bit_len: 0,
            }
        }

        fn f(&mut self, value: u32, bits: u8) {
            for bit_index in (0..bits).rev() {
                let bit = ((value >> bit_index) & 1) as u8;
                let byte_index = self.bit_len / 8;
                let bit_in_byte = 7 - (self.bit_len & 7);
                self.data[byte_index] |= bit << bit_in_byte;
                self.bit_len += 1;
            }
        }

        fn byte_align_zero(&mut self) {
            while self.bit_len & 7 != 0 {
                self.f(0, 1);
            }
        }

        fn byte(&mut self, value: u8) {
            self.byte_align_zero();
            let byte_index = self.bit_len / 8;
            self.data[byte_index] = value;
            self.bit_len += 8;
        }

        fn finish(self) -> [u8; 128] {
            self.data
        }
    }
}

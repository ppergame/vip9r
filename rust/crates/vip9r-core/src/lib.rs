#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

mod bitstream;
mod boolcoder;
mod compressed_header;
mod error;
mod header;
mod probability;
mod superframe;
mod tile;
mod tile_syntax;

use compressed_header::{
    CompressedHeader, TxMode, parse_inter_compressed_header, parse_intra_compressed_header,
};
use header::{HeaderParserState, parse_uncompressed_frame_header};
use probability::{NonCoefAdaptationConfig, ProbabilityState, SyntaxCounts};
use tile::parse_tile_layout;
#[cfg(feature = "std")]
use tile_syntax::StoredModeInfo;
use tile_syntax::{FrameModeBuffers, parse_inter_tiles, parse_intra_tiles};

pub const MAX_CODED_FRAMES_PER_PACKET: usize = 8;

const REFERENCE_FRAME_SLOTS: usize = 8;
const FRAME_POOL_SLOTS: usize = 1 + REFERENCE_FRAME_SLOTS;

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
}

impl WorkspaceLayout {
    pub fn new(max_width: u32, max_height: u32) -> Result<Self, DecodeError> {
        validate_limits(max_width, max_height)?;
        let frame_pool = FramePoolLayout::new(FrameLayout::new(max_width, max_height)?)?;

        Ok(Self {
            max_width,
            max_height,
            frame_pool,
        })
    }

    pub const fn max_width(self) -> u32 {
        self.max_width
    }

    pub const fn max_height(self) -> u32 {
        self.max_height
    }

    pub fn total_bytes(self) -> usize {
        self.frame_pool.total_bytes()
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
        let y_stride = usize::try_from(max_width).map_err(|_| DecodeError::InvalidConfig)?;
        let uv_width = max_width.div_ceil(2);
        let uv_height = max_height.div_ceil(2);
        let uv_stride = usize::try_from(uv_width).map_err(|_| DecodeError::InvalidConfig)?;

        let y = PlaneLayout::new(0, PlaneShape::new(max_width, max_height, y_stride))?;
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
    _memory: &'a mut [u8],
}

impl<'a> DecodeWorkspace<'a> {
    pub fn new(layout: WorkspaceLayout, memory: &'a mut [u8]) -> Result<Self, DecodeError> {
        let total_bytes = layout.total_bytes();
        if memory.len() < total_bytes {
            return Err(DecodeError::ResourceLimit);
        }
        Ok(Self {
            layout,
            _memory: &mut memory[..total_bytes],
        })
    }

    fn require_layout(&self, expected: WorkspaceLayout) -> Result<(), DecodeError> {
        if self.layout != expected {
            return Err(DecodeError::InvalidConfig);
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
#[derive(Debug)]
pub struct OwnedWorkspace {
    layout: WorkspaceLayout,
    memory: Vec<u8>,
}

#[cfg(feature = "std")]
impl OwnedWorkspace {
    pub fn new(layout: WorkspaceLayout) -> Result<Self, DecodeError> {
        let mut memory = Vec::new();
        memory
            .try_reserve_exact(layout.total_bytes())
            .map_err(|_| DecodeError::ResourceLimit)?;
        memory.resize(layout.total_bytes(), 0);
        Ok(Self { memory, layout })
    }

    pub fn as_workspace(&mut self) -> DecodeWorkspace<'_> {
        DecodeWorkspace::new(self.layout, &mut self.memory)
            .expect("owned workspace was allocated from its layout")
    }
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

    pub fn i420_len(self) -> Option<usize> {
        required_i420_len(self.visible_width, self.visible_height)
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
    last_frame_type: header::FrameType,
    previous_frame_for_mvs: Option<PreviousFrameForMvs>,
    #[cfg(feature = "std")]
    prev_mode_info: Vec<StoredModeInfo>,
    #[cfg(feature = "std")]
    curr_mode_info: Vec<StoredModeInfo>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PreviousFrameForMvs {
    width: u32,
    height: u32,
    show_frame: bool,
}

impl Decoder {
    pub fn new(layout: WorkspaceLayout) -> Self {
        Self {
            layout,
            header_state: HeaderParserState::new(),
            probability_state: ProbabilityState::new(),
            syntax_counts: SyntaxCounts::default(),
            last_frame_type: header::FrameType::Key,
            previous_frame_for_mvs: None,
            #[cfg(feature = "std")]
            prev_mode_info: Vec::new(),
            #[cfg(feature = "std")]
            curr_mode_info: Vec::new(),
        }
    }

    pub fn decode_coded_frame<'w>(
        &mut self,
        coded_frame: &[u8],
        workspace: &'w mut DecodeWorkspace<'_>,
    ) -> Result<DecodeOutcome<'w>, DecodeError> {
        workspace.require_layout(self.layout)?;
        let header = parse_uncompressed_frame_header(coded_frame, &self.header_state)
            .map_err(|err| err.into_decode_error())?;
        self.validate_frame_limits(&header)?;
        self.setup_frame_probability_state(&header)?;

        if !header.show_existing_frame && header.header_size_in_bytes != 0 {
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
            #[cfg(feature = "std")]
            let use_prev_frame_mvs = self.use_prev_frame_mvs(&header);

            #[cfg(feature = "std")]
            let tile_parse_result = {
                let mi_count = frame_mi_count(header.frame_width, header.frame_height)?;
                let mut prev_mode_info = core::mem::take(&mut self.prev_mode_info);
                let mut curr_mode_info = core::mem::take(&mut self.curr_mode_info);
                curr_mode_info.resize(mi_count, StoredModeInfo::DEFAULT);
                curr_mode_info.fill(StoredModeInfo::DEFAULT);
                let prev_frame_modes = if use_prev_frame_mvs && prev_mode_info.len() == mi_count {
                    Some(prev_mode_info.as_slice())
                } else {
                    None
                };
                let result = if header.frame_is_intra {
                    parse_intra_tiles(
                        coded_frame,
                        &header,
                        &compressed_header,
                        self.probability_state.current(),
                        &mut self.syntax_counts,
                        &tile_layout,
                        Some(curr_mode_info.as_mut_slice()),
                    )
                } else {
                    let use_prev_frame_mvs = use_prev_frame_mvs && prev_frame_modes.is_some();
                    parse_inter_tiles(
                        coded_frame,
                        &header,
                        &compressed_header,
                        self.probability_state.current(),
                        &mut self.syntax_counts,
                        &tile_layout,
                        FrameModeBuffers::new(
                            use_prev_frame_mvs,
                            prev_frame_modes,
                            Some(curr_mode_info.as_mut_slice()),
                        ),
                    )
                };
                if result.is_ok() {
                    core::mem::swap(&mut prev_mode_info, &mut curr_mode_info);
                }
                self.prev_mode_info = prev_mode_info;
                self.curr_mode_info = curr_mode_info;
                result
            };

            #[cfg(not(feature = "std"))]
            let tile_parse_result = if header.frame_is_intra {
                parse_intra_tiles(
                    coded_frame,
                    &header,
                    &compressed_header,
                    self.probability_state.current(),
                    &mut self.syntax_counts,
                    &tile_layout,
                    None,
                )
            } else {
                parse_inter_tiles(
                    coded_frame,
                    &header,
                    &compressed_header,
                    self.probability_state.current(),
                    &mut self.syntax_counts,
                    &tile_layout,
                    FrameModeBuffers::none(),
                )
            };

            tile_parse_result.map_err(|err| err.into_decode_error())?;

            self.refresh_probability_state(&header, &compressed_header)?;
        }

        self.header_state.update_references(&header);
        if !header.show_existing_frame {
            self.last_frame_type = header.frame_type;
            self.previous_frame_for_mvs = Some(PreviousFrameForMvs {
                width: header.frame_width,
                height: header.frame_height,
                show_frame: header.show_frame,
            });
        }
        Ok(DecodeOutcome::NoOutput)
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

    #[cfg(feature = "std")]
    fn use_prev_frame_mvs(&self, header: &header::UncompressedFrameHeader) -> bool {
        if header.error_resilient_mode || header.frame_is_intra || header.show_existing_frame {
            return false;
        }
        let Some(previous) = self.previous_frame_for_mvs else {
            return false;
        };
        previous.width == header.frame_width
            && previous.height == header.frame_height
            && previous.show_frame
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

pub fn required_i420_len(width: u32, height: u32) -> Option<usize> {
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

#[cfg(feature = "std")]
fn frame_mi_count(width: u32, height: u32) -> Result<usize, DecodeError> {
    let mi_cols = width.checked_add(7).ok_or(DecodeError::InvalidBitstream)? >> 3;
    let mi_rows = height.checked_add(7).ok_or(DecodeError::InvalidBitstream)? >> 3;
    let mi_cols = usize::try_from(mi_cols).map_err(|_| DecodeError::InvalidBitstream)?;
    let mi_rows = usize::try_from(mi_rows).map_err(|_| DecodeError::InvalidBitstream)?;
    mi_rows
        .checked_mul(mi_cols)
        .ok_or(DecodeError::InvalidBitstream)
}

fn validate_limits(max_width: u32, max_height: u32) -> Result<(), DecodeError> {
    if max_width == 0 || max_height == 0 || required_i420_len(max_width, max_height).is_none() {
        return Err(DecodeError::InvalidConfig);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        DecodeError, DecodeWorkspace, Decoder, OwnedWorkspace, WorkspaceLayout, required_i420_len,
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
    fn workspace_layout_is_current_plus_eight_i420_frame_slots() {
        let layout = WorkspaceLayout::new(16, 16).unwrap();

        assert_eq!(layout.max_width(), 16);
        assert_eq!(layout.max_height(), 16);
        assert_eq!(layout.total_bytes(), 16 * 16 * 3 / 2 * 9);
        assert_eq!(layout.frame_pool.current.start, 0);
        assert_eq!(layout.frame_pool.current.len, 16 * 16 * 3 / 2);
        assert_eq!(layout.frame_pool.references[0].start, 16 * 16 * 3 / 2);
        assert_eq!(
            layout.frame_pool.references[7].end().unwrap(),
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
    fn decode_workspace_rejects_undersized_arena() {
        let layout = WorkspaceLayout::new(16, 16).unwrap();
        let mut memory = vec![0; layout.total_bytes() - 1];

        assert_eq!(
            DecodeWorkspace::new(layout, &mut memory).unwrap_err(),
            DecodeError::ResourceLimit
        );
    }

    #[test]
    fn decode_coded_frame_rejects_workspace_for_other_layout() {
        let decoder_layout = WorkspaceLayout::new(16, 16).unwrap();
        let workspace_layout = WorkspaceLayout::new(32, 16).unwrap();
        let mut decoder = Decoder::new(decoder_layout);
        let mut owned_workspace = OwnedWorkspace::new(workspace_layout).unwrap();
        let mut workspace = owned_workspace.as_workspace();

        assert_eq!(
            decoder.decode_coded_frame(&[], &mut workspace),
            Err(DecodeError::InvalidConfig)
        );
    }

    #[test]
    fn decode_coded_frame_returns_no_output_after_valid_intra_tile_parse() {
        let frame = minimal_lossless_key_frame();
        let layout = WorkspaceLayout::new(16, 16).unwrap();
        let mut decoder = Decoder::new(layout);
        let mut owned_workspace = OwnedWorkspace::new(layout).unwrap();
        let mut workspace = owned_workspace.as_workspace();

        assert_eq!(
            decoder.decode_coded_frame(&frame, &mut workspace),
            Ok(super::DecodeOutcome::NoOutput)
        );
    }

    #[test]
    fn decode_coded_frame_refreshes_inter_frame_probabilities_after_tile_parse() {
        let key_frame = minimal_lossless_key_frame();
        let inter_frame = minimal_lossless_inter_frame();
        let layout = WorkspaceLayout::new(16, 16).unwrap();
        let mut decoder = Decoder::new(layout);
        let mut owned_workspace = OwnedWorkspace::new(layout).unwrap();
        let mut workspace = owned_workspace.as_workspace();

        assert_eq!(
            decoder.decode_coded_frame(&key_frame, &mut workspace),
            Ok(super::DecodeOutcome::NoOutput)
        );
        assert_eq!(
            decoder.decode_coded_frame(&inter_frame, &mut workspace),
            Ok(super::DecodeOutcome::NoOutput)
        );
    }

    fn minimal_lossless_key_frame() -> [u8; 128] {
        let mut builder = HeaderBuilder::new();
        builder.f(0b10, 2); // frame marker
        builder.f(0, 1); // profile low
        builder.f(0, 1); // profile high
        builder.f(0, 1); // not show existing frame
        builder.f(0, 1); // key frame
        builder.f(1, 1); // show frame
        builder.f(0, 1); // not error resilient
        builder.f(0x49, 8);
        builder.f(0x83, 8);
        builder.f(0x42, 8);
        builder.f(1, 3); // BT.601 color space
        builder.f(0, 1); // studio range
        builder.f(15, 16); // width - 1
        builder.f(15, 16); // height - 1
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

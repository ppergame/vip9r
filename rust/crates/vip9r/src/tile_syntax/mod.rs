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
use crate::pool::{self, WORKER_COUNT};
use crate::probability::{
    CLASS0_SIZE, FrameContext, MV_OFFSET_BITS, SWITCHABLE_FILTERS, SyntaxCounts,
};
use crate::tile::{MAX_TILE_COLS_LOG2, TileDescriptor, TileLayout};
use core::sync::atomic::AtomicU32;

mod coef;
mod inter_predict;
mod intra_predict;
mod loop_filter;
mod mode_info;
mod residual;
mod tables;

use coef::*;
use inter_predict::*;
use intra_predict::*;
use loop_filter::*;
pub(crate) use loop_filter::{LoopFilterJob, run_loop_filter_job};
use mode_info::ModeInfoViewMutRaw;
#[cfg(any(test, feature = "wasm-tests"))]
pub(crate) use mode_info::STORED_MODE_INFO_BYTES;
use mode_info::*;
pub(crate) use mode_info::{FrameModeBuffers, ModeInfoView, ModeInfoViewMut, mode_info_byte_len};
use residual::*;
use tables::*;

// Above contexts are column-indexed, not frame-MI indexed. This fixed storage
// covers the local VP9 large-scaling frontier: the largest expected frame is
// 20400px wide (2550 MI columns), rounded up to 2560 entries for 64x64
// partition contexts. Wider frames need workspace-backed context storage and
// are reported as a resource limit instead of an invalid bitstream.
const MAX_MI_COLS: usize = 2560;
const MAX_TILE_COLS: usize = 1usize << (MAX_TILE_COLS_LOG2 as usize);
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
type InterpBuffer = [u8; MAX_INTERP_BUFFER];
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

impl From<TileSyntaxError> for DecodeError {
    fn from(error: TileSyntaxError) -> Self {
        match error {
            TileSyntaxError::InvalidBitstream => Self::InvalidBitstream,
            TileSyntaxError::ResourceLimit => Self::ResourceLimit,
            TileSyntaxError::Unimplemented => Self::Unimplemented,
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

    fn raw_parts(&mut self) -> CurrentFrameRaw {
        CurrentFrameRaw {
            y: self.y.raw_parts(),
            u: self.u.raw_parts(),
            v: self.v.raw_parts(),
        }
    }

    /// Rebuild a full-frame view from raw parts for a loop-filter wavefront
    /// participant. The view starts with an empty window; the participant
    /// must move it onto a superblock via `set_superblock_window` before
    /// filtering.
    ///
    /// # Safety
    /// Same liveness contract as `from_band_raw`. Aliased full-plane views
    /// may coexist across threads only under the wavefront protocol: every
    /// access stays inside the current superblock window (checked in
    /// `loop_filter_segment`), and the watermark lag keeps simultaneously
    /// active superblock windows pairwise disjoint.
    unsafe fn from_raw_windowed(raw: CurrentFrameRaw) -> Result<Self, TileSyntaxError> {
        // SAFETY: Forwarded caller contract, see above.
        let mut frame = unsafe { Self::from_band_raw(raw, 0, 0) }?;
        frame.y.set_window(0, 0, 0, 0);
        frame.u.set_window(0, 0, 0, 0);
        frame.v.set_window(0, 0, 0, 0);
        Ok(frame)
    }

    /// Move the access window onto the superblock at MI position
    /// (`row`, `col`), with an 8-pixel margin on every side: edge-0 filters
    /// reach up to 8 pixels into the left/above neighbors, and the clamped
    /// fallback path reads up to 8 pixels past the last in-superblock edge.
    fn set_superblock_window(&mut self, row: usize, col: usize) -> Result<(), TileSyntaxError> {
        for plane in 0..PLANES {
            let sub_x = subsampling_x(plane);
            let sub_y = subsampling_y(plane);
            let sb_w = 64usize >> sub_x;
            let sb_h = 64usize >> sub_y;
            let x0 = col
                .checked_mul(8)
                .ok_or(TileSyntaxError::InvalidBitstream)?
                >> sub_x;
            let y0 = row
                .checked_mul(8)
                .ok_or(TileSyntaxError::InvalidBitstream)?
                >> sub_y;
            let plane_view = self.plane_mut(plane)?;
            let x_end = core::cmp::min(
                x0.checked_add(sb_w)
                    .and_then(|end| end.checked_add(8))
                    .ok_or(TileSyntaxError::InvalidBitstream)?,
                plane_view.width,
            );
            let y_end = core::cmp::min(
                y0.checked_add(sb_h)
                    .and_then(|end| end.checked_add(8))
                    .ok_or(TileSyntaxError::InvalidBitstream)?,
                plane_view.height,
            );
            plane_view.set_window(x0.saturating_sub(8), x_end, y0.saturating_sub(8), y_end);
        }
        Ok(())
    }

    unsafe fn from_band_raw(
        raw: CurrentFrameRaw,
        mi_col_start: usize,
        mi_col_end: usize,
    ) -> Result<Self, TileSyntaxError> {
        Ok(Self {
            // SAFETY: The caller's band-splitting contract guarantees that
            // each raw plane points at the live current-frame storage and that
            // simultaneously constructed bands have disjoint column ranges.
            y: unsafe {
                CurrentPlaneMut::from_raw_band(raw.y, luma_band(mi_col_start, mi_col_end)?)?
            },
            // SAFETY: Same as for luma; chroma bands are the corresponding
            // half-resolution column ranges, with the last band extending to
            // the plane edge by the plane-band constructor.
            u: unsafe {
                CurrentPlaneMut::from_raw_band(raw.u, chroma_band(mi_col_start, mi_col_end)?)?
            },
            // SAFETY: Same as for U.
            v: unsafe {
                CurrentPlaneMut::from_raw_band(raw.v, chroma_band(mi_col_start, mi_col_end)?)?
            },
        })
    }
}

#[derive(Debug)]
pub(crate) struct CurrentPlaneMut<'a> {
    data: &'a mut [u8],
    width: usize,
    height: usize,
    stride: usize,
    band_x_start: usize,
    band_x_end: usize,
    band_y_start: usize,
    band_y_end: usize,
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
            band_x_start: 0,
            band_x_end: width,
            band_y_start: 0,
            band_y_end: height,
        })
    }

    fn sample_clamped(&self, x: usize, y: usize) -> Result<u8, TileSyntaxError> {
        if self.width == 0 || self.height == 0 {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let x = core::cmp::min(x, self.width - 1);
        let y = core::cmp::min(y, self.height - 1);
        self.check_band_x(x)?;
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
        self.check_band_x(x)?;

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

    fn raw_parts(&mut self) -> CurrentPlaneRaw {
        CurrentPlaneRaw {
            data: self.data.as_mut_ptr() as usize,
            len: self.data.len(),
            width: self.width,
            height: self.height,
            stride: self.stride,
        }
    }

    unsafe fn from_raw_band(
        raw: CurrentPlaneRaw,
        band: PlaneBand,
    ) -> Result<Self, TileSyntaxError> {
        if raw.stride < raw.width || band.x_start > band.x_end || band.x_start > raw.width {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        let len = raw
            .stride
            .checked_mul(raw.height)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        if raw.len < len {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        let band_x_end = core::cmp::min(band.x_end, raw.width);
        // SAFETY: The raw parts came from a live CurrentPlaneMut over the
        // current frame.  The band splitter creates aliased full-plane slices
        // only with disjoint accepted column ranges; every direct access path
        // checks those ranges and reports InvalidBitstream before touching a
        // cross-band column.  Fused loop-filter aliases are further contained
        // to checked superblock windows and gated by decode-row watermarks.
        let data = unsafe { core::slice::from_raw_parts_mut(raw.data as *mut u8, raw.len) };
        Ok(Self {
            data,
            width: raw.width,
            height: raw.height,
            stride: raw.stride,
            band_x_start: band.x_start,
            band_x_end,
            band_y_start: 0,
            band_y_end: raw.height,
        })
    }

    fn set_window(&mut self, x_start: usize, x_end: usize, y_start: usize, y_end: usize) {
        self.band_x_start = x_start;
        self.band_x_end = x_end;
        self.band_y_start = y_start;
        self.band_y_end = y_end;
    }

    /// Check that the half-open rectangle fits the view's window. The loop
    /// filter calls this once per segment with the segment's maximal touch
    /// rectangle, which makes each raw kernel access below it in-window by
    /// construction.
    pub(super) fn check_window_rect(
        &self,
        x_start: usize,
        x_end: usize,
        y_start: usize,
        y_end: usize,
    ) -> Result<(), TileSyntaxError> {
        if x_start < self.band_x_start
            || x_end > self.band_x_end
            || y_start < self.band_y_start
            || y_end > self.band_y_end
        {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        Ok(())
    }

    fn check_band_x(&self, x: usize) -> Result<(), TileSyntaxError> {
        if x < self.band_x_start || x >= self.band_x_end {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        Ok(())
    }

    pub(super) fn check_band_span(&self, x: usize, width: usize) -> Result<(), TileSyntaxError> {
        if width == 0 {
            return Ok(());
        }
        let end = x
            .checked_add(width)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        if x < self.band_x_start || end > self.band_x_end {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        Ok(())
    }

    pub(super) fn span_inside_band(&self, x: usize, width: usize) -> bool {
        width == 0
            || x.checked_add(width)
                .is_some_and(|end| x >= self.band_x_start && end <= self.band_x_end)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CurrentFrameRaw {
    y: CurrentPlaneRaw,
    u: CurrentPlaneRaw,
    v: CurrentPlaneRaw,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CurrentPlaneRaw {
    data: usize,
    len: usize,
    width: usize,
    height: usize,
    stride: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PlaneBand {
    x_start: usize,
    x_end: usize,
}

fn luma_band(mi_col_start: usize, mi_col_end: usize) -> Result<PlaneBand, TileSyntaxError> {
    Ok(PlaneBand {
        x_start: mi_col_start
            .checked_mul(8)
            .ok_or(TileSyntaxError::InvalidBitstream)?,
        x_end: mi_col_end
            .checked_mul(8)
            .ok_or(TileSyntaxError::InvalidBitstream)?,
    })
}

fn chroma_band(mi_col_start: usize, mi_col_end: usize) -> Result<PlaneBand, TileSyntaxError> {
    Ok(PlaneBand {
        x_start: mi_col_start
            .checked_mul(4)
            .ok_or(TileSyntaxError::InvalidBitstream)?,
        x_end: mi_col_end
            .checked_mul(4)
            .ok_or(TileSyntaxError::InvalidBitstream)?,
    })
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
        reference_frames: Option<ReferenceFrames<'r>>,
    ) -> Self {
        Self {
            counts,
            mode_buffers,
            current_frame,
            reference_frames,
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
        accumulate_counts: !header.error_resilient_mode && !header.frame_parallel_decoding_mode,
    };

    let loop_filter_done =
        parse_tile_frame(frame, header, probabilities, layout, config, &mut buffers)?;
    if !loop_filter_done {
        loop_filter_frame(header, buffers.current_frame, &buffers.mode_buffers)?;
    }

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
    if buffers.reference_frames.is_none() {
        return Err(TileSyntaxError::InvalidBitstream);
    }
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
        accumulate_counts: !header.error_resilient_mode && !header.frame_parallel_decoding_mode,
    };

    let loop_filter_done =
        parse_tile_frame(frame, header, probabilities, layout, config, &mut buffers)?;
    if !loop_filter_done {
        loop_filter_frame(header, buffers.current_frame, &buffers.mode_buffers)?;
    }

    Ok(())
}

/// Everything a column-band job reads that is identical across the wave.
/// Lives on the coordinator's stack from before dispatch until after join;
/// jobs carry a pointer to it. Read-only state crosses as plain references —
/// their true lifetime ("until join") is enforced by the pool protocol, not
/// the type system. Only the two shared-mutable surfaces stay as raw parts,
/// because every job must rebuild its own band-restricted view over them.
#[derive(Clone, Copy)]
struct TileWave<'a> {
    frame: &'a [u8],
    tiles: &'a [TileDescriptor],
    probabilities: &'a FrameContext,
    config: TileParserConfig,
    tile_cols: usize,
    tile_rows: usize,
    prev_frame_modes: Option<ModeInfoView<'a>>,
    current_frame: CurrentFrameRaw,
    current_frame_modes: Option<ModeInfoViewMutRaw>,
    reference_frames: Option<ReferenceFrames<'a>>,
    decode_watermarks: Option<&'a [AtomicU32]>,
    loop_filter: Option<LoopFilterWave<'a>>,
}

/// One column band of the wave. All pointers are usize-erased because job
/// slots are statics: `wave` targets the coordinator's stack-resident
/// TileWave, `counts` the worker's WORKER_SYNTAX_COUNTS entry (0 when counts
/// are off), and `result` the coordinator's stack-resident result cell (0 for
/// the coordinator's own jobs, which take the result by return value).
#[derive(Clone, Copy)]
pub(crate) struct TileJob {
    wave: usize,
    tile_col: usize,
    mi_col_start: usize,
    mi_col_end: usize,
    counts: usize,
    result: usize,
}

/// One participant in a fused tile-decode plus loop-filter wave. A worker may
/// have no decode band when there are fewer tile columns than participants; it
/// then goes straight to the shared filter row-claim pool.
#[derive(Clone, Copy)]
pub(crate) struct FusedTileFilterJob {
    wave: usize,
    participant: usize,
    has_decode: bool,
    tile_col: usize,
    mi_col_start: usize,
    mi_col_end: usize,
    counts: usize,
    result: usize,
}

/// A tile decode failure tagged with the row-major index of the tile it is
/// attributed to; error selection across bands takes the lowest index to
/// match serial decode's first-failure reporting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TileBandError {
    tile_index: usize,
    error: TileSyntaxError,
}

type TileBandResult = Result<(), TileBandError>;

#[derive(Clone, Copy)]
struct FusedTileFilterResult {
    decode: TileBandResult,
    filter: LoopFilterBandResult,
}

impl FusedTileFilterResult {
    const OK: Self = Self {
        decode: Ok(()),
        filter: Ok(()),
    };
}

pub(crate) fn run_tile_job(job: TileJob) {
    let result = decode_tile_job(job);
    // SAFETY: The coordinator stores a pointer to this wave's per-slot
    // result cell in the job before the Release epoch bump.  The cell remains
    // live and slot-disjoint until after join's Acquire observes this worker's
    // Release acknowledgement.
    unsafe {
        *(job.result as *mut TileBandResult) = result;
    }
}

pub(crate) fn run_fused_tile_filter_job(job: FusedTileFilterJob) {
    let decode = if job.has_decode {
        decode_tile_job(TileJob {
            wave: job.wave,
            tile_col: job.tile_col,
            mi_col_start: job.mi_col_start,
            mi_col_end: job.mi_col_end,
            counts: job.counts,
            result: 0,
        })
    } else {
        Ok(())
    };
    // SAFETY: The wave lives in parse_tile_frame_parallel's stack frame and
    // this job runs strictly between dispatch and join.
    let wave = unsafe { &*(job.wave as *const TileWave) };
    let filter = match wave.loop_filter {
        Some(filter_wave) => loop_filter_band(&filter_wave, job.participant),
        None => Ok(()),
    };
    // SAFETY: Same result-cell handoff as tile and loop-filter jobs.
    unsafe {
        *(job.result as *mut FusedTileFilterResult) = FusedTileFilterResult { decode, filter };
    }
}

fn parse_tile_frame(
    frame: &[u8],
    header: &UncompressedFrameHeader,
    probabilities: &FrameContext,
    layout: &TileLayout,
    config: TileParserConfig,
    buffers: &mut TileParseBuffers<'_, '_, '_, '_>,
) -> Result<bool, TileSyntaxError> {
    let (tile_cols, tile_rows) = tile_grid(layout)?;
    if !pool::is_active() || tile_cols == 1 {
        parse_tile_frame_serial(frame, probabilities, layout, config, buffers)?;
        return Ok(false);
    }
    parse_tile_frame_parallel(
        frame,
        header,
        probabilities,
        layout.as_slice(),
        config,
        buffers,
        (tile_cols, tile_rows),
    )
}

fn parse_tile_frame_serial(
    frame: &[u8],
    probabilities: &FrameContext,
    layout: &TileLayout,
    config: TileParserConfig,
    buffers: &mut TileParseBuffers<'_, '_, '_, '_>,
) -> Result<(), TileSyntaxError> {
    let (_, mi_cols) = config.frame_mis;
    let mut contexts = TileModeContexts::new(mi_cols)?;

    for tile in layout.as_slice() {
        parse_tile(
            frame,
            tile,
            config,
            buffers.mode_buffers.for_tile(),
            TileParseShared {
                probabilities,
                counts: buffers.counts as *mut SyntaxCounts,
                contexts: &mut contexts,
                current_frame: &mut *buffers.current_frame,
                reference_frames: buffers.reference_frames,
            },
            None,
        )?;
    }
    Ok(())
}

/// Per-worker syntax-count accumulators. Coordinator-owned, like SESSION:
/// the coordinator clears them before dispatch and merges after join, and
/// workers only ever write through the pointer handed to them in their job
/// slot. Not in the workspace arena because nothing about it is per-frame or
/// per-geometry — it is fixed decoder state.
static mut WORKER_SYNTAX_COUNTS: [SyntaxCounts; WORKER_COUNT] = [SyntaxCounts::ZERO; WORKER_COUNT];

fn parse_tile_frame_parallel(
    frame: &[u8],
    header: &UncompressedFrameHeader,
    probabilities: &FrameContext,
    tiles: &[TileDescriptor],
    config: TileParserConfig,
    buffers: &mut TileParseBuffers<'_, '_, '_, '_>,
    tile_grid: (usize, usize),
) -> Result<bool, TileSyntaxError> {
    let (tile_cols, tile_rows) = tile_grid;
    if tile_cols > MAX_TILE_COLS {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    let mut band_mi_starts = [0usize; MAX_TILE_COLS];
    let mut band_mi_ends = [0usize; MAX_TILE_COLS];
    for tile_col in 0..tile_cols {
        let (mi_col_start, mi_col_end) = column_mi_range(tiles, tile_cols, tile_rows, tile_col)?;
        band_mi_starts[tile_col] = mi_col_start;
        band_mi_ends[tile_col] = mi_col_end;
    }

    let current_frame = buffers.current_frame.raw_parts();
    let current_frame_modes = buffers
        .mode_buffers
        .current_frame_modes
        .as_mut()
        .map(ModeInfoViewMut::raw_parts);

    let sb_rows = config.frame_mis.0.div_ceil(MI_BLOCK_64);
    let sb_cols = config.frame_mis.1.div_ceil(MI_BLOCK_64);
    let can_fuse_filter = header.loop_filter.level != 0
        && (2..=MAX_WAVEFRONT_SB_ROWS).contains(&sb_rows)
        && sb_cols <= MAX_WAVEFRONT_SB_COLS;
    let loop_filter_done = header.loop_filter.level == 0 || can_fuse_filter;

    let decode_watermarks = [const { AtomicU32::new(0) }; MAX_TILE_COLS];
    let filter_watermarks = [const { pool::Watermark::new(0) }; MAX_WAVEFRONT_SB_ROWS];
    let filter_row_claim = AtomicU32::new(0);
    let mut sb_col_band_start = [0u8; MAX_WAVEFRONT_SB_COLS];
    let mut sb_col_band_end = [0u8; MAX_WAVEFRONT_SB_COLS];

    let loop_filter_config = if can_fuse_filter {
        if header.profile != 0 || header.bit_depth != 8 {
            return Err(TileSyntaxError::Unimplemented);
        }
        let raw_modes = current_frame_modes.ok_or(TileSyntaxError::InvalidBitstream)?;
        // SAFETY: The read view aliases decoder band-mutable views only inside
        // the fused wave. Loop-filter mode reads are gated by per-band decode
        // row watermarks before use.
        let modes = unsafe { ModeInfoView::from_raw_full(raw_modes, config.frame_mis.1)? };
        precompute_loop_filter_band_gates(
            config.frame_mis.1,
            sb_cols,
            tile_cols,
            &band_mi_starts[..tile_cols],
            &band_mi_ends[..tile_cols],
            &mut sb_col_band_start[..sb_cols],
            &mut sb_col_band_end[..sb_cols],
        )?;
        Some(loop_filter_config_from_modes(header, modes)?)
    } else {
        None
    };
    let loop_filter_strengths = match loop_filter_config {
        Some(filter_config) => Some(loop_filter_strength_lut(
            filter_config.params,
            filter_config.segmentation,
        )?),
        None => None,
    };
    let decode_watermark_slice = &decode_watermarks[..tile_cols];
    let decode_gates = loop_filter_config.map(|_| LoopFilterDecodeGates {
        watermarks: decode_watermark_slice,
        sb_col_band_start: &sb_col_band_start[..sb_cols],
        sb_col_band_end: &sb_col_band_end[..sb_cols],
    });
    let loop_filter = match (loop_filter_config, loop_filter_strengths.as_ref()) {
        (Some(filter_config), Some(strengths)) => Some(LoopFilterWave {
            config: filter_config,
            strengths,
            current_frame,
            sb_rows,
            sb_cols,
            watermarks: &filter_watermarks[..sb_rows],
            row_claim: &filter_row_claim,
            decode_gates,
        }),
        _ => None,
    };

    let wave = TileWave {
        frame,
        tiles,
        probabilities,
        config,
        tile_cols,
        tile_rows,
        prev_frame_modes: buffers.mode_buffers.prev_frame_modes,
        current_frame,
        current_frame_modes,
        reference_frames: buffers.reference_frames,
        decode_watermarks: loop_filter.map(|_| decode_watermark_slice),
        loop_filter,
    };
    // SAFETY: The coordinator is the only caller and no wave is in flight —
    // the previous wave was joined before parse_tile_frame_parallel returned,
    // and workers touch the array only through per-wave slot pointers.
    let worker_counts = unsafe { &mut *core::ptr::addr_of_mut!(WORKER_SYNTAX_COUNTS) };

    let worker_decode_jobs = core::cmp::min(WORKER_COUNT, tile_cols - 1);
    if wave.loop_filter.is_some() {
        let mut worker_results: [FusedTileFilterResult; WORKER_COUNT] =
            [FusedTileFilterResult::OK; WORKER_COUNT];
        let mut jobs = [None; WORKER_COUNT];
        for worker_index in 0..WORKER_COUNT {
            let has_decode = worker_index < worker_decode_jobs;
            let counts = if has_decode && config.accumulate_counts {
                worker_counts[worker_index].clear();
                &mut worker_counts[worker_index] as *mut SyntaxCounts as usize
            } else {
                0
            };
            jobs[worker_index] = Some(pool::Job::FusedTileFilter(FusedTileFilterJob {
                wave: &wave as *const TileWave as usize,
                participant: worker_index + 1,
                has_decode,
                tile_col: worker_index,
                mi_col_start: if has_decode {
                    band_mi_starts[worker_index]
                } else {
                    0
                },
                mi_col_end: if has_decode {
                    band_mi_ends[worker_index]
                } else {
                    0
                },
                counts,
                result: &mut worker_results[worker_index] as *mut FusedTileFilterResult as usize,
            }));
        }

        pool::dispatch(&jobs);

        // No early return between dispatch and join: workers hold pointers
        // into this frame's stack (the wave and their result cells), so every
        // path must join before unwinding. Errors funnel through aggregation;
        // decode errors win over any speculative filter errors because serial
        // semantics decode the full tile frame before filtering.
        let mut coordinator_result = FusedTileFilterResult::OK;
        for tile_col in worker_decode_jobs..tile_cols {
            let result = decode_tile_job(TileJob {
                wave: &wave as *const TileWave as usize,
                tile_col,
                mi_col_start: band_mi_starts[tile_col],
                mi_col_end: band_mi_ends[tile_col],
                counts: if config.accumulate_counts {
                    buffers.counts as *mut SyntaxCounts as usize
                } else {
                    0
                },
                result: 0,
            });
            choose_earliest_error(&mut coordinator_result.decode, result);
        }
        if let Some(filter_wave) = wave.loop_filter {
            coordinator_result.filter = loop_filter_band(&filter_wave, 0);
        }

        if !pool::join(-1) {
            return Err(TileSyntaxError::ResourceLimit);
        }

        let mut best_decode = coordinator_result.decode;
        let mut best_filter = coordinator_result.filter;
        for result in &worker_results {
            choose_earliest_error(&mut best_decode, result.decode);
            choose_earliest_filter_error(&mut best_filter, result.filter);
        }
        if let Err(failure) = best_decode {
            return Err(failure.error);
        }

        if config.accumulate_counts {
            for counts in worker_counts.iter().take(worker_decode_jobs) {
                buffers.counts.merge_from(counts);
            }
        }
        return best_filter.map(|()| true).map_err(|failure| failure.error);
    }

    let mut worker_results: [TileBandResult; WORKER_COUNT] = [Ok(()); WORKER_COUNT];
    let mut jobs = [None; WORKER_COUNT];
    for worker_index in 0..worker_decode_jobs {
        let counts = if config.accumulate_counts {
            worker_counts[worker_index].clear();
            &mut worker_counts[worker_index] as *mut SyntaxCounts as usize
        } else {
            0
        };
        jobs[worker_index] = Some(pool::Job::Tile(TileJob {
            wave: &wave as *const TileWave as usize,
            tile_col: worker_index,
            mi_col_start: band_mi_starts[worker_index],
            mi_col_end: band_mi_ends[worker_index],
            counts,
            result: &mut worker_results[worker_index] as *mut TileBandResult as usize,
        }));
    }

    pool::dispatch(&jobs);

    // No early return between dispatch and join: workers hold pointers into
    // this frame's stack (the wave and their result cells), so every path
    // must join before unwinding. Errors funnel through the result
    // aggregation instead.
    let mut best: TileBandResult = Ok(());
    for tile_col in worker_decode_jobs..tile_cols {
        let result = decode_tile_job(TileJob {
            wave: &wave as *const TileWave as usize,
            tile_col,
            mi_col_start: band_mi_starts[tile_col],
            mi_col_end: band_mi_ends[tile_col],
            counts: if config.accumulate_counts {
                buffers.counts as *mut SyntaxCounts as usize
            } else {
                0
            },
            result: 0,
        });
        choose_earliest_error(&mut best, result);
    }

    if !pool::join(-1) {
        return Err(TileSyntaxError::ResourceLimit);
    }

    for &result in worker_results.iter().take(worker_decode_jobs) {
        choose_earliest_error(&mut best, result);
    }
    if let Err(failure) = best {
        return Err(failure.error);
    }

    if config.accumulate_counts {
        for counts in worker_counts.iter().take(worker_decode_jobs) {
            buffers.counts.merge_from(counts);
        }
    }
    Ok(loop_filter_done)
}

fn choose_earliest_error(best: &mut TileBandResult, candidate: TileBandResult) {
    if let Err(new) = candidate {
        let earlier = match *best {
            Ok(()) => true,
            Err(old) => new.tile_index < old.tile_index,
        };
        if earlier {
            *best = Err(new);
        }
    }
}

fn choose_earliest_filter_error(best: &mut LoopFilterBandResult, candidate: LoopFilterBandResult) {
    if let Err(new) = candidate {
        let earlier = match *best {
            Ok(()) => true,
            Err(old) => new.sb_index < old.sb_index,
        };
        if earlier {
            *best = Err(new);
        }
    }
}

fn decode_tile_job(job: TileJob) -> TileBandResult {
    // SAFETY: The wave lives in parse_tile_frame_parallel's stack frame,
    // which does not return between dispatch and join; its references are
    // valid for at least that long, and this job runs strictly inside it.
    let wave = unsafe { &*(job.wave as *const TileWave) };
    let result = decode_tile_job_inner(wave, job);
    if result.is_err() {
        publish_decode_abandoned(wave, job.tile_col);
    }
    result
}

fn decode_tile_job_inner(wave: &TileWave<'_>, job: TileJob) -> TileBandResult {
    // Failures before any tile decodes are attributed to this column's row-0
    // tile, whose row-major index is the column index itself.
    let band_error = |error| TileBandError {
        tile_index: job.tile_col,
        error,
    };
    // SAFETY: Band views are column-disjoint, live only for this wave, and all
    // cross-thread visibility is ordered by pool dispatch/join plus, in the
    // fused filter case, by the decode-row watermarks.
    let mut current_frame = unsafe {
        CurrentFrameMut::from_band_raw(wave.current_frame, job.mi_col_start, job.mi_col_end)
    }
    .map_err(band_error)?;
    let current_frame_modes = match wave.current_frame_modes {
        // SAFETY: Same band contract as the planes, over the current mode
        // grid: one view per column-disjoint band, every write band-checked.
        Some(raw) => Some(
            unsafe {
                ModeInfoViewMut::from_raw_band(
                    raw,
                    wave.config.frame_mis.1,
                    job.mi_col_start,
                    job.mi_col_end,
                )
            }
            .map_err(band_error)?,
        ),
        None => None,
    };
    let mode_buffers = FrameModeBuffers::new(
        wave.config.use_prev_frame_mvs,
        wave.prev_frame_modes,
        current_frame_modes,
    );
    parse_tile_band(
        wave,
        job.tile_col,
        mode_buffers,
        &mut current_frame,
        job.counts as *mut SyntaxCounts,
    )
}

fn publish_decode_abandoned(wave: &TileWave<'_>, tile_col: usize) {
    if let Some(watermarks) = wave.decode_watermarks
        && let Some(watermark) = watermarks.get(tile_col)
    {
        DecodeBandProgress {
            watermark,
            sb_rows: wave.config.frame_mis.0.div_ceil(MI_BLOCK_64),
        }
        .publish_abandoned();
    }
}

fn parse_tile_band(
    wave: &TileWave<'_>,
    tile_col: usize,
    mut mode_buffers: FrameModeBuffers<'_>,
    current_frame: &mut CurrentFrameMut<'_>,
    counts: *mut SyntaxCounts,
) -> TileBandResult {
    let TileWave {
        frame,
        tiles,
        tile_cols,
        tile_rows,
        config,
        ..
    } = *wave;
    let (_, mi_cols) = config.frame_mis;
    let mut contexts = TileModeContexts::new(mi_cols).map_err(|error| TileBandError {
        tile_index: tile_col,
        error,
    })?;
    let decode_progress = wave.decode_watermarks.map(|watermarks| DecodeBandProgress {
        watermark: &watermarks[tile_col],
        sb_rows: config.frame_mis.0.div_ceil(MI_BLOCK_64),
    });

    for tile_row in 0..tile_rows {
        let tile_index = tile_row
            .checked_mul(tile_cols)
            .and_then(|base| base.checked_add(tile_col))
            .ok_or(TileBandError {
                tile_index: tile_col,
                error: TileSyntaxError::InvalidBitstream,
            })?;
        let tile = tiles.get(tile_index).ok_or(TileBandError {
            tile_index,
            error: TileSyntaxError::InvalidBitstream,
        })?;
        if usize::from(tile.tile_col) != tile_col || usize::from(tile.tile_row) != tile_row {
            return Err(TileBandError {
                tile_index,
                error: TileSyntaxError::InvalidBitstream,
            });
        }
        parse_tile(
            frame,
            tile,
            config,
            mode_buffers.for_tile(),
            TileParseShared {
                probabilities: wave.probabilities,
                counts,
                contexts: &mut contexts,
                current_frame: &mut *current_frame,
                reference_frames: wave.reference_frames,
            },
            decode_progress,
        )
        .map_err(|error| TileBandError { tile_index, error })?;
    }
    if let Some(progress) = decode_progress {
        progress.publish_abandoned();
    }
    Ok(())
}

fn tile_grid(layout: &TileLayout) -> Result<(usize, usize), TileSyntaxError> {
    let tiles = layout.as_slice();
    let last = tiles.last().ok_or(TileSyntaxError::InvalidBitstream)?;
    let tile_cols = usize::from(last.tile_col)
        .checked_add(1)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let tile_rows = usize::from(last.tile_row)
        .checked_add(1)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    if tile_cols == 0
        || tile_rows == 0
        || tile_cols
            .checked_mul(tile_rows)
            .ok_or(TileSyntaxError::InvalidBitstream)?
            != tiles.len()
    {
        return Err(TileSyntaxError::InvalidBitstream);
    }
    Ok((tile_cols, tile_rows))
}

fn column_mi_range(
    tiles: &[TileDescriptor],
    tile_cols: usize,
    tile_rows: usize,
    tile_col: usize,
) -> Result<(usize, usize), TileSyntaxError> {
    if tile_col >= tile_cols {
        return Err(TileSyntaxError::InvalidBitstream);
    }
    let first = tiles
        .get(tile_col)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let start =
        usize::try_from(first.mi_col_start).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let end = usize::try_from(first.mi_col_end).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    for tile_row in 0..tile_rows {
        let tile_index = tile_row
            .checked_mul(tile_cols)
            .and_then(|base| base.checked_add(tile_col))
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let tile = tiles
            .get(tile_index)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        if usize::from(tile.tile_col) != tile_col
            || usize::from(tile.tile_row) != tile_row
            || usize::try_from(tile.mi_col_start).map_err(|_| TileSyntaxError::InvalidBitstream)?
                != start
            || usize::try_from(tile.mi_col_end).map_err(|_| TileSyntaxError::InvalidBitstream)?
                != end
        {
            return Err(TileSyntaxError::InvalidBitstream);
        }
    }
    Ok((start, end))
}

fn precompute_loop_filter_band_gates(
    mi_cols: usize,
    sb_cols: usize,
    tile_cols: usize,
    band_mi_starts: &[usize],
    band_mi_ends: &[usize],
    sb_col_band_start: &mut [u8],
    sb_col_band_end: &mut [u8],
) -> Result<(), TileSyntaxError> {
    if tile_cols == 0
        || band_mi_starts.len() < tile_cols
        || band_mi_ends.len() < tile_cols
        || sb_col_band_start.len() < sb_cols
        || sb_col_band_end.len() < sb_cols
    {
        return Err(TileSyntaxError::InvalidBitstream);
    }

    for sb_col in 0..sb_cols {
        let left_sb = sb_col.saturating_sub(1);
        let right_sb = core::cmp::min(
            sb_col
                .checked_add(1)
                .ok_or(TileSyntaxError::InvalidBitstream)?,
            sb_cols - 1,
        );
        let mi_start = left_sb
            .checked_mul(MI_BLOCK_64)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let mi_end = core::cmp::min(
            right_sb
                .checked_add(1)
                .and_then(|value| value.checked_mul(MI_BLOCK_64))
                .ok_or(TileSyntaxError::InvalidBitstream)?,
            mi_cols,
        );

        let mut first = 0usize;
        while first < tile_cols && band_mi_ends[first] <= mi_start {
            first += 1;
        }
        if first == tile_cols || band_mi_starts[first] >= mi_end {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let mut end = first;
        while end < tile_cols && band_mi_starts[end] < mi_end {
            end += 1;
        }
        sb_col_band_start[sb_col] =
            u8::try_from(first).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        sb_col_band_end[sb_col] =
            u8::try_from(end).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    }

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
    accumulate_counts: bool,
}

struct TileParseShared<'a, 'f, 'r> {
    probabilities: &'a FrameContext,
    counts: *mut SyntaxCounts,
    contexts: &'a mut TileModeContexts,
    current_frame: &'a mut CurrentFrameMut<'f>,
    reference_frames: Option<ReferenceFrames<'r>>,
}

#[derive(Clone, Copy)]
struct DecodeBandProgress<'a> {
    watermark: &'a AtomicU32,
    sb_rows: usize,
}

impl DecodeBandProgress<'_> {
    fn publish_completed_mi_row(self, next_mi_row: usize) {
        let completed = core::cmp::min(next_mi_row / MI_BLOCK_64, self.sb_rows);
        pool::watermark_store(self.watermark, completed as u32);
    }

    fn publish_abandoned(self) {
        pool::watermark_store(self.watermark, self.sb_rows as u32);
    }
}

fn parse_tile(
    frame: &[u8],
    tile: &TileDescriptor,
    config: TileParserConfig,
    mode_buffers: FrameModeBuffers<'_>,
    shared: TileParseShared<'_, '_, '_>,
    decode_progress: Option<DecodeBandProgress<'_>>,
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
        accumulate_counts: config.accumulate_counts,
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
        intra: IntraPredictionBuffers::new(),
        residual: ResidualBuffers::new(),
        interp_buffer: [0; MAX_INTERP_BUFFER],
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
        if let Some(progress) = decode_progress {
            progress.publish_completed_mi_row(row);
        }
    }

    parser.decoder.finish()?;
    Ok(())
}

struct TileParser<'a, 'b, 'r> {
    decoder: BoolDecoder<'a>,
    probabilities: &'b FrameContext,
    counts: *mut SyntaxCounts,
    accumulate_counts: bool,
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
    intra: IntraPredictionBuffers,
    residual: ResidualBuffers,
    interp_buffer: InterpBuffer,
}

macro_rules! increment_syntax_count {
    ($parser:expr, $($field:tt)+) => {{
        if let Some(counts) = $parser.counts_mut()? {
            increment_count(&mut counts.$($field)+);
        }
    }};
}

struct IntraPredictionBuffers {
    edges: IntraPredictionEdges,
    pred: [u8; MAX_TX_COEFFS],
}

impl IntraPredictionBuffers {
    const fn new() -> Self {
        Self {
            edges: IntraPredictionEdges::new(),
            pred: [0; MAX_TX_COEFFS],
        }
    }
}

struct ResidualBuffers {
    dequantized: DequantizedCoefficients,
    token_cache: [u8; MAX_TX_COEFFS],
    dequantized_dirty: bool,
}

impl ResidualBuffers {
    fn new() -> Self {
        Self {
            dequantized: DequantizedCoefficients::empty(),
            token_cache: [0; MAX_TX_COEFFS],
            dequantized_dirty: false,
        }
    }

    fn clear_dequantized_dirty(&mut self) {
        if self.dequantized_dirty {
            self.dequantized.clear_transform_extent();
            self.dequantized_dirty = false;
        }
    }

    fn clear_dequantized_block(&mut self) {
        self.dequantized.clear_transform_extent();
        self.dequantized_dirty = false;
    }

    fn clear_token_cache_prefix(&mut self, scan: &[u16], eob: usize) {
        for &pos in scan.iter().take(eob) {
            self.token_cache[usize::from(pos)] = 0;
        }
    }
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

impl IntraPredictionEdges {
    const fn new() -> Self {
        Self {
            above_left: 127,
            above_row: [127; MAX_INTRA_ABOVE],
            left_col: [129; MAX_TX_WIDTH],
        }
    }
}

impl TileParser<'_, '_, '_> {
    fn counts_mut(&mut self) -> Result<Option<&mut SyntaxCounts>, TileSyntaxError> {
        if !self.accumulate_counts {
            return Ok(None);
        }
        if self.counts.is_null() {
            return Err(TileSyntaxError::InvalidBitstream);
        }
        // SAFETY: When accumulation is enabled, serial decode passes the
        // decoder's single counts object and parallel decode passes either the
        // coordinator's object (for coordinator-owned bands) or this worker's
        // disjoint workspace slot.  &mut self guarantees one transient mutable
        // access at a time within the parser.
        Ok(Some(unsafe { &mut *self.counts }))
    }

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
        increment_syntax_count!(self, counts_partition[ctx][partition.index()]);
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
                let segment_map_id = prev_frame_modes
                    .segment_map_id(index)?
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                segment_id = core::cmp::min(segment_id, segment_map_id);
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
        increment_syntax_count!(self, counts_is_inter[ctx][bool_index(is_inter)]);
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
        let stored = StoredModeInfo {
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
        let mut encoded = [0; STORED_MODE_INFO_BYTES];
        encode_stored_mode_info(stored, &mut encoded);
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
                encoded[STORED_MODE_INFO_SEGMENT_MAP_ID_OFFSET] = if preserve_segment_map {
                    match prev_frame_modes {
                        Some(prev_frame_modes) => prev_frame_modes
                            .segment_map_id(index)?
                            .ok_or(TileSyntaxError::InvalidBitstream)?,
                        None => 0,
                    }
                } else if self.segment_map_reset && !segment_map_updates {
                    0
                } else {
                    block.segment_id
                };
                current_frame_modes.set_encoded(index, &encoded)?;
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
        increment_syntax_count!(self, counts_skip[ctx][bool_index(skip)]);
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
            increment_syntax_count!(
                self,
                counts_tx_size[max_tx_size.index()][ctx][tx_size.index()]
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
        increment_syntax_count!(self, counts_intra_mode[ctx][mode.index()]);
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
        increment_syntax_count!(self, counts_uv_mode[y_mode.index()][mode.index()]);
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
            increment_syntax_count!(self, counts_comp_mode[ctx][bool_index(compound)]);
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
        increment_syntax_count!(self, counts_single_ref[ctx][0][bool_index(single_ref_p1)]);
        let ref_frame = if single_ref_p1 {
            let ctx = single_ref_p2_context(left, above, avail_l, avail_u);
            let single_ref_p2 = self
                .decoder
                .read_bool(self.probabilities.single_ref_prob[ctx][1])?;
            increment_syntax_count!(self, counts_single_ref[ctx][1][bool_index(single_ref_p2)]);
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
        increment_syntax_count!(self, counts_comp_ref[ctx][comp_ref]);
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
        increment_syntax_count!(self, counts_inter_mode[ctx][mode.index()]);
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
                increment_syntax_count!(self, counts_interp_filter[usize::from(ctx)][filter_index]);
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
        increment_syntax_count!(self, counts_mv_joint[usize::from(joint)]);
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
        increment_syntax_count!(self, counts_mv_sign[comp][bool_index(sign)]);
        let mv_class = usize::from(self.decoder.read_tree(&MV_CLASS_TREE, &probs.class[comp])?);
        increment_syntax_count!(self, counts_mv_class[comp][mv_class]);
        let mag = if mv_class == 0 {
            let class0_bit = usize::from(self.decoder.read_bool(probs.class0_bit[comp])?);
            increment_syntax_count!(self, counts_mv_class0_bit[comp][class0_bit]);
            let class0_fr = usize::from(
                self.decoder
                    .read_tree(&MV_FR_TREE, &probs.class0_fr[comp][class0_bit])?,
            );
            increment_syntax_count!(self, counts_mv_class0_fr[comp][class0_bit][class0_fr]);
            let class0_hp = if use_hp {
                usize::from(self.decoder.read_bool(probs.class0_hp[comp])?)
            } else {
                1
            };
            increment_syntax_count!(self, counts_mv_class0_hp[comp][class0_hp]);
            ((class0_bit << 3) | (class0_fr << 1) | class0_hp) + 1
        } else {
            let mut d = 0usize;
            for i in 0..mv_class {
                if i >= MV_OFFSET_BITS {
                    return Err(TileSyntaxError::InvalidBitstream);
                }
                let mv_bit = self.decoder.read_bool(probs.bits[comp][i])?;
                increment_syntax_count!(self, counts_mv_bits[comp][i][bool_index(mv_bit)]);
                if mv_bit {
                    d |= 1usize << i;
                }
            }
            let mv_fr = usize::from(self.decoder.read_tree(&MV_FR_TREE, &probs.fr[comp])?);
            increment_syntax_count!(self, counts_mv_fr[comp][mv_fr]);
            let mv_hp = if use_hp {
                usize::from(self.decoder.read_bool(probs.hp[comp])?)
            } else {
                1
            };
            increment_syntax_count!(self, counts_mv_hp[comp][mv_hp]);
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

        for &candidate in search.iter().take(2) {
            if let Some(info) = self.mv_ref_candidate(row, col, candidate)? {
                different_ref_found = true;
                context_counter = context_counter
                    .checked_add(usize::from(MODE_2_COUNTER[usize::from(info.y_mode)]))
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                for ref_list in 0..REF_LISTS {
                    if info.ref_frames[ref_list] == ref_frame {
                        let mv =
                            self.candidate_sub_block_mv(info, ref_list, candidate[1], block)?;
                        state.add_mv_ref(mv);
                        break;
                    }
                }
            }
        }

        for &candidate in search.iter().skip(2) {
            if let Some(info) = self.mv_ref_candidate(row, col, candidate)? {
                different_ref_found = true;
                if_same_ref_frame_add_mv(&mut state, info, ref_frame);
            }
        }

        let prev_candidate = if self.use_prev_frame_mvs {
            self.prev_mv_ref_candidate(row, col)?
        } else {
            None
        };
        if self.use_prev_frame_mvs {
            if_same_prev_frame_add_mv(&mut state, prev_candidate, ref_frame);
        }
        if different_ref_found {
            for &candidate in search {
                if let Some(info) = self.mv_ref_candidate(row, col, candidate)? {
                    if_diff_ref_frame_add_mv(&mut state, info, ref_frame, self.ref_sign_biases())?;
                }
            }
        }
        if self.use_prev_frame_mvs {
            if_diff_prev_frame_add_mv(
                &mut state,
                prev_candidate,
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
        candidate: [i8; 2],
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
            return current_frame_modes
                .as_view()
                .mv_ref_candidate(index, CandidateSubMvs::StoredCurrent { index });
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
    ) -> Result<Option<CandidateModeInfo>, TileSyntaxError> {
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
        prev_frame_modes.mv_ref_candidate(index, CandidateSubMvs::Unavailable)
    }

    fn candidate_sub_block_mv(
        &self,
        info: CandidateModeInfo,
        ref_list: usize,
        delta_col: i8,
        block: i8,
    ) -> Result<MotionVector, TileSyntaxError> {
        if block < 0 {
            return info
                .mvs
                .get(ref_list)
                .copied()
                .ok_or(TileSyntaxError::InvalidBitstream);
        }

        match info.sub_mvs {
            CandidateSubMvs::RepeatedMvs => info
                .mvs
                .get(ref_list)
                .copied()
                .ok_or(TileSyntaxError::InvalidBitstream),
            CandidateSubMvs::StoredCurrent { index } => {
                let sub_block = sub_block_mv_index(delta_col, block)?;
                let current_frame_modes = self
                    .current_frame_modes
                    .as_ref()
                    .ok_or(TileSyntaxError::InvalidBitstream)?;
                current_frame_modes
                    .as_view()
                    .sub_mv(index, ref_list, sub_block)
            }
            CandidateSubMvs::Unavailable => Err(TileSyntaxError::InvalidBitstream),
        }
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
                            self.tokens(
                                plane,
                                (start_x, start_y),
                                tx_size,
                                block_idx,
                                mi_size,
                                block,
                            )?;
                            nonzero = self.residual.dequantized.nonzero_context();
                            if nonzero {
                                self.residual.dequantized.inverse_transform(self.lossless)?;
                                reconstruct(current_frame, &self.residual.dequantized)?;
                                self.residual.clear_dequantized_block();
                            }
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
        &mut self,
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
        intra_prediction_edges(plane, context, &mut self.intra.edges)?;
        let request = IntraPredictionRequest {
            mode,
            have_left: context.have_left,
            have_above: context.have_above,
            size,
        };
        let edges = &self.intra.edges;
        if write_common_intra_prediction_direct(
            plane,
            context.start_x,
            context.start_y,
            request,
            edges,
        )? {
            return Ok(());
        }

        let pred = &mut self.intra.pred;
        intra_predict_block(request, edges, pred)?;
        write_prediction_block(plane, context.start_x, context.start_y, size, pred)
    }

    fn predict_inter(
        &mut self,
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
            let interp_buffer = &mut self.interp_buffer;
            inter_predict_unscaled_block(
                refs[0].ok_or(TileSyntaxError::InvalidBitstream)?,
                scaled[0],
                interp_filter,
                plane,
                context,
                InterPredictionWrite::Store,
                interp_buffer,
            )?;
            if is_compound {
                inter_predict_unscaled_block(
                    refs[1].ok_or(TileSyntaxError::InvalidBitstream)?,
                    scaled[1],
                    interp_filter,
                    plane,
                    context,
                    InterPredictionWrite::Average,
                    interp_buffer,
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
    ) -> Result<(), TileSyntaxError> {
        if plane >= PLANES {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let seg_eob = 16usize << (tx_size.index() << 1);
        let tx_type = self.get_tx_type(plane, tx_size, block_idx, mi_size, block)?;
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
        let dq_shift = dq_shift(tx_size);
        let dc_quant = self
            .dequant
            .get_dc_quant_for_segment(plane, block.segment_id);
        let ac_quant = self
            .dequant
            .get_ac_quant_for_segment(plane, block.segment_id);
        let counts = self.counts;
        let accumulate_counts = self.accumulate_counts;
        let decoder = &mut self.decoder;
        let residual = &mut self.residual;
        residual.clear_dequantized_dirty();
        residual
            .dequantized
            .reset(TransformBlock::new(plane, start, tx_size, tx_type), 0);

        {
            let ResidualBuffers {
                dequantized,
                token_cache,
                dequantized_dirty,
            } = residual;

            while c < seg_eob {
                let pos = usize::from(*scan.get(c).ok_or(TileSyntaxError::InvalidBitstream)?);
                let band = usize::from(coef_bands[c]);
                let ctx = if c == 0 {
                    dc_ctx
                } else {
                    coefficient_token_context(pos, tx_size, tx_type, token_cache)?
                };
                let probability_row = &coef_probs[band][ctx];

                if check_eob
                    && !read_more_coefs(
                        decoder,
                        probability_row,
                        syntax_counts_from_raw(accumulate_counts, counts)?.map(|counts| {
                            &mut counts.counts_more_coefs[tx_index][plane_type][ref_type][band][ctx]
                        }),
                    )?
                {
                    break;
                }

                let token = read_token(
                    decoder,
                    probability_row,
                    syntax_counts_from_raw(accumulate_counts, counts)?.map(|counts| {
                        &mut counts.counts_token[tx_index][plane_type][ref_type][band][ctx]
                    }),
                )?;
                token_cache[pos] = ENERGY_CLASS[token.index()];
                if token == CoefToken::Zero {
                    check_eob = false;
                } else {
                    let coef = read_coef(decoder, token)?;
                    let sign_bit = decoder.read_literal(1)?;
                    dequantized.set_signed_dequantized(
                        pos, coef, sign_bit, dc_quant, ac_quant, dq_shift,
                    )?;
                    *dequantized_dirty = true;
                    check_eob = true;
                }

                c = c.checked_add(1).ok_or(TileSyntaxError::InvalidBitstream)?;
            }

            dequantized.set_eob(c)?;
        }
        residual.clear_token_cache_prefix(scan, c);
        Ok(())
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

fn avg2(a: u8, b: u8) -> u8 {
    ((u16::from(a) + u16::from(b) + 1) >> 1) as u8
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

fn syntax_counts_from_raw<'a>(
    accumulate_counts: bool,
    counts: *mut SyntaxCounts,
) -> Result<Option<&'a mut SyntaxCounts>, TileSyntaxError> {
    if !accumulate_counts {
        return Ok(None);
    }
    if counts.is_null() {
        return Err(TileSyntaxError::InvalidBitstream);
    }
    // SAFETY: The caller passes the parser/job-local counts pointer.  Parallel
    // jobs use disjoint worker slots and the coordinator uses its own counts;
    // callers immediately project the returned reference to one counter row
    // and do not keep overlapping mutable references alive.
    Ok(Some(unsafe { &mut *counts }))
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

fn clip3(min_value: i32, max_value: i32, value: i32) -> i32 {
    core::cmp::min(core::cmp::max(value, min_value), max_value)
}

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

#[cfg(feature = "wasm-tests")]
mod test_support {
    use super::*;
    use crate::header::{FrameType, LoopFilterParams, UncompressedFrameHeader};
    use crate::probability::FrameContext;

    pub(super) const LOOP_FILTER_TEST_STRIDE: usize = 32;
    pub(super) const LOOP_FILTER_TEST_WIDTH: usize = 24;
    pub(super) const LOOP_FILTER_TEST_HEIGHT: usize = 32;
    pub(super) const LOOP_FILTER_TEST_X: usize = 8;
    pub(super) const LOOP_FILTER_TEST_Y: usize = 16;
    pub(super) struct TestCurrentFrame {
        y: [u8; 16 * 16],
        u: [u8; 8 * 8],
        v: [u8; 8 * 8],
        y_len: usize,
        uv_len: usize,
        y_shape: crate::PlaneShape,
        uv_shape: crate::PlaneShape,
    }

    impl TestCurrentFrame {
        pub(super) fn new(width: u32, height: u32) -> Self {
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

        pub(super) fn as_current_frame(&mut self) -> CurrentFrameMut<'_> {
            CurrentFrameMut::new(
                CurrentPlaneMut::new(&mut self.y[..self.y_len], self.y_shape).unwrap(),
                CurrentPlaneMut::new(&mut self.u[..self.uv_len], self.uv_shape).unwrap(),
                CurrentPlaneMut::new(&mut self.v[..self.uv_len], self.uv_shape).unwrap(),
            )
        }

        pub(super) fn y(&self) -> &[u8] {
            &self.y[..self.y_len]
        }
    }

    pub(super) fn test_reference_frame<'a>(
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

    pub(super) fn prediction_edges(
        above_left: u8,
        above: &[u8],
        left: &[u8],
    ) -> IntraPredictionEdges {
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

    pub(super) fn random_prediction_edges(seed: &mut u32) -> IntraPredictionEdges {
        let mut edges = IntraPredictionEdges {
            above_left: 0,
            above_row: [0; MAX_INTRA_ABOVE],
            left_col: [0; MAX_TX_WIDTH],
        };
        fill_pseudorandom(core::slice::from_mut(&mut edges.above_left), seed);
        fill_pseudorandom(&mut edges.above_row, seed);
        fill_pseudorandom(&mut edges.left_col, seed);
        edges
    }

    pub(super) fn prediction(
        request: IntraPredictionRequest,
        edges: &IntraPredictionEdges,
    ) -> [u8; MAX_TX_COEFFS] {
        let mut pred = [0; MAX_TX_COEFFS];
        intra_predict_block(request, edges, &mut pred).unwrap();
        pred
    }

    pub(super) fn assert_prediction(
        request: IntraPredictionRequest,
        edges: &IntraPredictionEdges,
        expected: &[u8],
    ) {
        let pred = prediction(request, edges);
        assert_eq!(&pred[..expected.len()], expected);
    }

    pub(super) fn assert_prediction_all(
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

    pub(super) fn check_horizontal_loop_filter_segment(
        initial: [u8; LOOP_FILTER_TEST_STRIDE * LOOP_FILTER_TEST_HEIGHT],
        len: usize,
        filter_size: TxSize,
        strength: LoopFilterStrength,
        case_index: usize,
    ) -> Result<(), TileSyntaxError> {
        let mut scalar_data = initial;
        let mut simd_data = initial;
        let base = LOOP_FILTER_TEST_Y * LOOP_FILTER_TEST_STRIDE + LOOP_FILTER_TEST_X;

        for offset in 0..len {
            sample_filter_direct(
                &mut scalar_data,
                base + offset,
                LOOP_FILTER_TEST_STRIDE,
                filter_size,
                strength,
            );
        }

        let mut plane = CurrentPlaneMut {
            data: &mut simd_data,
            width: LOOP_FILTER_TEST_WIDTH,
            height: LOOP_FILTER_TEST_HEIGHT,
            stride: LOOP_FILTER_TEST_STRIDE,
            band_x_start: 0,
            band_x_end: LOOP_FILTER_TEST_WIDTH,
            band_y_start: 0,
            band_y_end: LOOP_FILTER_TEST_HEIGHT,
        };
        loop_filter_segment(
            &mut plane,
            1,
            LOOP_FILTER_TEST_X,
            LOOP_FILTER_TEST_Y,
            len,
            filter_size,
            strength,
        )?;

        let mut mismatch = None;
        for index in 0..scalar_data.len() {
            if scalar_data[index] != simd_data[index] {
                mismatch = Some(index);
                break;
            }
        }
        if let Some(mismatch) = mismatch {
            panic!(
                "horizontal loop filter segment mismatch: case={case_index} len={len} \
                 filter_size={filter_size:?} strength={strength:?} index={mismatch} \
                 scalar={} simd={}",
                u32::from(scalar_data[mismatch]),
                u32::from(simd_data[mismatch])
            );
        }

        Ok(())
    }

    pub(super) fn paint_loop_filter_pattern(
        data: &mut [u8; LOOP_FILTER_TEST_STRIDE * LOOP_FILTER_TEST_HEIGHT],
        len: usize,
        strength: LoopFilterStrength,
        pattern: usize,
    ) {
        for col in LOOP_FILTER_TEST_X..LOOP_FILTER_TEST_X + len {
            let (p, q) = match pattern {
                0 => {
                    let base = match col % 3 {
                        0 => 128,
                        1 => 129,
                        _ => 127,
                    };
                    ([base; 8], [base; 8])
                }
                1 => {
                    let delta = core::cmp::max(1, strength.thresh.saturating_add(1));
                    let delta = core::cmp::min(delta, strength.limit);
                    (
                        [
                            128,
                            128u8.saturating_sub(delta),
                            128u8.saturating_sub(delta),
                            128u8.saturating_sub(delta),
                            128u8.saturating_sub(delta),
                            128u8.saturating_sub(delta),
                            128u8.saturating_sub(delta),
                            128u8.saturating_sub(delta),
                        ],
                        [
                            128,
                            128u8.saturating_add(delta),
                            128u8.saturating_add(delta),
                            128u8.saturating_add(delta),
                            128u8.saturating_add(delta),
                            128u8.saturating_add(delta),
                            128u8.saturating_add(delta),
                            128u8.saturating_add(delta),
                        ],
                    )
                }
                2 => {
                    let high = 64u8.saturating_add(strength.limit.saturating_add(1));
                    ([64, 64, 64, high, 64, 64, 64, 64], [64; 8])
                }
                4 => match (col - LOOP_FILTER_TEST_X) & 7 {
                    // Fully flat through p7/q7: Tx8 wide3 and Tx16 wide4.
                    0 => ([128; 8], [128; 8]),
                    // Flat across p3/q3 but not p7/q7: Tx16 falls back to wide3.
                    1 => (
                        [128, 128, 128, 128, 131, 131, 131, 131],
                        [128, 128, 128, 128, 125, 125, 125, 125],
                    ),
                    // Non-flat with low p1/q1 deltas: narrow, usually !hev.
                    2 => {
                        if strength.limit >= 2 {
                            (
                                [128, 128, 130, 130, 130, 130, 130, 130],
                                [128, 128, 126, 126, 126, 126, 126, 126],
                            )
                        } else {
                            ([128; 8], [128; 8])
                        }
                    }
                    // Non-flat and above the hev threshold, while staying inside limit.
                    3 => {
                        let delta = core::cmp::max(1, strength.thresh.saturating_add(1));
                        let delta = core::cmp::min(delta, strength.limit);
                        let p1 = 128u8.saturating_add(delta);
                        let p2 = if delta == 1 { p1.saturating_add(1) } else { p1 };
                        (
                            [128, p1, p2, p2, p2, p2, p2, p2],
                            [128, p1, p2, p2, p2, p2, p2, p2],
                        )
                    }
                    // Mask-fail lane interleaved with filtered lanes.
                    4 => {
                        let high = 64u8.saturating_add(strength.limit.saturating_add(1));
                        ([64, 64, 64, high, 64, 64, 64, 64], [64; 8])
                    }
                    // More wide3 lanes with q-side flat2 failure.
                    5 => (
                        [127, 128, 128, 128, 128, 128, 128, 128],
                        [128, 128, 129, 128, 130, 130, 130, 130],
                    ),
                    // Additional wide4 lanes with a different base to catch lane blends.
                    _ => ([90; 8], [91; 8]),
                },
                _ => ([0; 8], [255; 8]),
            };
            paint_loop_filter_column(data, col, p, q);
        }
    }

    pub(super) fn paint_loop_filter_column(
        data: &mut [u8; LOOP_FILTER_TEST_STRIDE * LOOP_FILTER_TEST_HEIGHT],
        col: usize,
        p: [u8; 8],
        q: [u8; 8],
    ) {
        for (offset, sample) in p.into_iter().enumerate() {
            data[(LOOP_FILTER_TEST_Y - 1 - offset) * LOOP_FILTER_TEST_STRIDE + col] = sample;
        }
        for (offset, sample) in q.into_iter().enumerate() {
            data[(LOOP_FILTER_TEST_Y + offset) * LOOP_FILTER_TEST_STRIDE + col] = sample;
        }
    }

    pub(super) fn fill_pseudorandom(data: &mut [u8], seed: &mut u32) {
        for byte in data {
            *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *byte = (*seed >> 24) as u8;
        }
    }

    pub(super) fn test_header(segmentation_enabled: bool) -> UncompressedFrameHeader {
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

    pub(super) fn sign_bit_test_probabilities() -> FrameContext {
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

    pub(super) fn test_block(skip: bool) -> DecodedBlockInfo {
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

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::test_support::*;
    use super::*;
    use crate::tile::parse_tile_layout;

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
                    None,
                ),
            ),
            Ok(())
        );
    }

    #[test]
    fn current_frame_band_views_reject_cross_band_luma_and_chroma_access() {
        let mut current_frame_storage = TestCurrentFrame::new(16, 16);
        let mut current_frame = current_frame_storage.as_current_frame();
        let raw = current_frame.raw_parts();

        // SAFETY: The test constructs a single band view and does not use the
        // original full view while the band view is live.
        let mut band = unsafe { CurrentFrameMut::from_band_raw(raw, 0, 1) }.unwrap();

        assert_eq!(band.y.sample_clamped(0, 0), Ok(128));
        assert_eq!(band.y.set_visible(7, 0, 17), Ok(()));
        assert_eq!(
            band.y.sample_clamped(8, 0),
            Err(TileSyntaxError::InvalidBitstream)
        );
        assert_eq!(
            band.y.set_visible(8, 0, 19),
            Err(TileSyntaxError::InvalidBitstream)
        );

        assert_eq!(band.u.sample_clamped(0, 0), Ok(128));
        assert_eq!(band.u.set_visible(3, 0, 23), Ok(()));
        assert_eq!(
            band.u.sample_clamped(4, 0),
            Err(TileSyntaxError::InvalidBitstream)
        );
        assert_eq!(
            band.u.set_visible(4, 0, 29),
            Err(TileSyntaxError::InvalidBitstream)
        );
    }

    #[test]
    fn current_frame_last_band_extends_to_plane_edge() {
        let mut y = [128u8; 18 * 8];
        let mut u = [128u8; 9 * 4];
        let mut v = [128u8; 9 * 4];
        let y_shape = crate::PlaneShape::new(18, 8, 18);
        let uv_shape = crate::PlaneShape::new(9, 4, 9);
        let mut current_frame = CurrentFrameMut::new(
            CurrentPlaneMut::new(&mut y, y_shape).unwrap(),
            CurrentPlaneMut::new(&mut u, uv_shape).unwrap(),
            CurrentPlaneMut::new(&mut v, uv_shape).unwrap(),
        );
        let raw = current_frame.raw_parts();

        // SAFETY: The test constructs only the last band view from the raw
        // full-frame parts and does not use the original view concurrently.
        let band = unsafe { CurrentFrameMut::from_band_raw(raw, 2, 3) }.unwrap();

        assert_eq!(band.y.sample_clamped(100, 0), Ok(128));
        assert_eq!(band.u.sample_clamped(100, 0), Ok(128));
        assert_eq!(
            band.y.sample_clamped(15, 0),
            Err(TileSyntaxError::InvalidBitstream)
        );
        assert_eq!(
            band.u.sample_clamped(7, 0),
            Err(TileSyntaxError::InvalidBitstream)
        );
    }

    #[test]
    fn mode_grid_band_view_rejects_out_of_band_indices() {
        let mut bytes = [0u8; 8 * STORED_MODE_INFO_BYTES];
        let mut modes = ModeInfoViewMut::new(&mut bytes).unwrap();
        let raw = modes.raw_parts();

        // SAFETY: This test creates one mutable band view and does not use the
        // original full-grid view while it is live.
        let mut band = unsafe { ModeInfoViewMut::from_raw_band(raw, 4, 1, 3) }.unwrap();
        let encoded = [0u8; STORED_MODE_INFO_BYTES];

        assert_eq!(band.set_encoded(1, &encoded), Ok(()));
        assert_eq!(band.set_encoded(6, &encoded), Ok(()));
        assert_eq!(
            band.set_encoded(0, &encoded),
            Err(TileSyntaxError::InvalidBitstream)
        );
        assert_eq!(
            band.as_view().segment_map_id(3),
            Err(TileSyntaxError::InvalidBitstream)
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
                counts: &mut counts as *mut SyntaxCounts,
                accumulate_counts: true,
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
}

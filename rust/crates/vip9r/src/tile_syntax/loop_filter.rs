use super::*;

use core::arch::wasm32::*;
use core::sync::atomic::AtomicU32;

#[derive(Clone, Copy, Debug)]
pub(super) struct LoopFilterConfig<'a> {
    pub(super) params: LoopFilterParams,
    pub(super) segmentation: SegmentationParams,
    pub(super) modes: ModeInfoView<'a>,
    pub(super) mi_rows: usize,
    pub(super) mi_cols: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LoopFilterStrength {
    pub(super) lvl: u8,
    pub(super) limit: u8,
    pub(super) blimit: u8,
    pub(super) thresh: u8,
}

impl LoopFilterStrength {
    const ZERO: Self = Self {
        lvl: 0,
        limit: 0,
        blimit: 0,
        thresh: 0,
    };
}

pub(super) const LOOP_FILTER_REF_FRAMES: usize = 4;
pub(super) const LOOP_FILTER_MODE_TYPES: usize = 2;
pub(super) const LOOP_FILTER_SB_MIS: usize = MI_BLOCK_64 * MI_BLOCK_64;

#[derive(Clone, Copy, Debug)]
pub(super) struct LoopFilterStrengthLut {
    pub(super) by_level: [LoopFilterStrength; 64],
    pub(super) by_segment_ref_mode:
        [[[LoopFilterStrength; LOOP_FILTER_MODE_TYPES]; LOOP_FILTER_REF_FRAMES]; MAX_SEGMENTS],
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LoopFilterMiInfo {
    pub(super) valid: bool,
    pub(super) skip: bool,
    pub(super) tx_size: TxSize,
    pub(super) uv_tx_size: TxSize,
    pub(super) mi_size: BlockSize,
    pub(super) ref_frame: u8,
    pub(super) strength: LoopFilterStrength,
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
pub(super) struct LoopFilterSuperblockInfo {
    pub(super) mi: [LoopFilterMiInfo; LOOP_FILTER_SB_MIS],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LoopFilterMasks {
    pub(super) hev: bool,
    pub(super) filter: bool,
    pub(super) flat: bool,
    pub(super) flat2: bool,
}

pub(super) fn loop_filter_frame(
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

    let sb_rows = config.mi_rows.div_ceil(MI_BLOCK_64);
    let sb_cols = config.mi_cols.div_ceil(MI_BLOCK_64);
    if pool::is_active() && (2..=MAX_WAVEFRONT_SB_ROWS).contains(&sb_rows) {
        return loop_filter_frame_wavefront(config, &strengths, current_frame, sb_rows, sb_cols);
    }

    for sb_row in 0..sb_rows {
        for sb_col in 0..sb_cols {
            loop_filter_superblock_all(
                current_frame,
                config,
                &strengths,
                sb_row * MI_BLOCK_64,
                sb_col * MI_BLOCK_64,
            )?;
        }
    }

    Ok(())
}

fn loop_filter_superblock_all(
    current_frame: &mut CurrentFrameMut<'_>,
    config: LoopFilterConfig<'_>,
    strengths: &LoopFilterStrengthLut,
    row: usize,
    col: usize,
) -> Result<(), TileSyntaxError> {
    let sb_info = loop_filter_superblock_info(config, strengths, row, col)?;
    for plane in 0..PLANES {
        for pass in 0..2 {
            loop_filter_superblock(current_frame, config, &sb_info, plane, pass, row, col)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SB-row wavefront
//
// Filtering superblock (r, c) touches only its own 64x64 extent plus an
// 8-pixel apron into the left and above neighbors (edge-0 filters write up
// to 7 and read up to 8 samples across the boundary; nothing reaches right
// of or below the superblock). In-row order makes the left apron safe. The
// above apron overlaps the *left apron of the above-right neighbor*: the
// vertical edge-0 filter of (r-1, c+1) writes into the bottom-right corner
// pixels of (r-1, c) that the horizontal edge-0 filter of (r, c) reads and
// writes. Hence the wavefront lag: row r may filter column c once row r-1
// has completed column c+1. Under that constraint every reorderable
// superblock pair has disjoint touch windows, so the schedule is bit-exact
// against serial raster order, and simultaneously active windows never
// alias (the containment `check_window_rect` in `loop_filter_segment`
// verifies each segment stays inside its window).

/// VP9 frame dimensions cap at 2^16, so 65536 / 64 superblock rows bound the
/// watermark table. Larger (malformed) frames fall back to serial filtering.
const MAX_WAVEFRONT_SB_ROWS: usize = 1024;

/// Wavefront participants: the coordinator plus every pool worker.
/// Participant p owns superblock rows p, p + 4, p + 8, ...
const LOOP_FILTER_PARTICIPANTS: usize = WORKER_COUNT + 1;

/// Everything a wavefront participant needs. Lives on the coordinator's
/// stack from before dispatch until after join; jobs carry a pointer to it.
/// `watermarks[r]` counts fully filtered superblocks of row r; participants
/// publish with Release stores and wait with Acquire loads, which also
/// carries the filtered pixels of the row above.
pub(super) struct LoopFilterWave<'a> {
    config: LoopFilterConfig<'a>,
    strengths: &'a LoopFilterStrengthLut,
    current_frame: CurrentFrameRaw,
    sb_rows: usize,
    sb_cols: usize,
    watermarks: &'a [AtomicU32],
}

/// One wavefront participant's job. Pointers are usize-erased because job
/// slots are statics: `wave` targets the coordinator's stack-resident
/// LoopFilterWave, `result` the coordinator's stack-resident result cell.
#[derive(Clone, Copy)]
pub(crate) struct LoopFilterJob {
    wave: usize,
    participant: usize,
    result: usize,
}

/// A filtering failure tagged with the raster index of the superblock it is
/// attributed to; error selection across participants takes the lowest index
/// to match serial filtering's first-failure reporting.
#[derive(Clone, Copy)]
struct LoopFilterBandError {
    sb_index: usize,
    error: TileSyntaxError,
}

type LoopFilterBandResult = Result<(), LoopFilterBandError>;

pub(crate) fn run_loop_filter_job(job: LoopFilterJob) {
    // SAFETY: The wave lives in loop_filter_frame_wavefront's stack frame,
    // which does not return between dispatch and join.
    let wave = unsafe { &*(job.wave as *const LoopFilterWave) };
    let result = loop_filter_band(wave, job.participant);
    // SAFETY: The result cell is live and slot-disjoint until after join,
    // exactly like tile job result cells.
    unsafe {
        *(job.result as *mut LoopFilterBandResult) = result;
    }
}

fn loop_filter_frame_wavefront(
    config: LoopFilterConfig<'_>,
    strengths: &LoopFilterStrengthLut,
    current_frame: &mut CurrentFrameMut<'_>,
    sb_rows: usize,
    sb_cols: usize,
) -> Result<(), TileSyntaxError> {
    let watermarks = [const { AtomicU32::new(0) }; MAX_WAVEFRONT_SB_ROWS];
    let wave = LoopFilterWave {
        config,
        strengths,
        current_frame: current_frame.raw_parts(),
        sb_rows,
        sb_cols,
        watermarks: &watermarks[..sb_rows],
    };

    let mut worker_results: [LoopFilterBandResult; WORKER_COUNT] = [Ok(()); WORKER_COUNT];
    let mut jobs = [None; WORKER_COUNT];
    for (worker_index, job) in jobs.iter_mut().enumerate() {
        *job = Some(pool::Job::LoopFilter(LoopFilterJob {
            wave: &wave as *const LoopFilterWave as usize,
            participant: worker_index + 1,
            result: &mut worker_results[worker_index] as *mut LoopFilterBandResult as usize,
        }));
    }

    pool::dispatch(&jobs);

    // No early return between dispatch and join: workers hold pointers into
    // this frame's stack. The coordinator owns row 0, so filtering starts
    // before the workers finish waking.
    let mut best = loop_filter_band(&wave, 0);

    if !pool::join(-1) {
        return Err(TileSyntaxError::ResourceLimit);
    }

    for &result in &worker_results {
        if let Err(new) = result {
            let earlier = match best {
                Ok(()) => true,
                Err(old) => new.sb_index < old.sb_index,
            };
            if earlier {
                best = Err(new);
            }
        }
    }
    best.map_err(|failure| failure.error)
}

fn loop_filter_band(wave: &LoopFilterWave<'_>, participant: usize) -> LoopFilterBandResult {
    // SAFETY: Every participant holds an aliased full-plane view, but all
    // accesses stay inside the current superblock window (checked per
    // segment) and simultaneously active windows are disjoint under the
    // watermark lag; cross-thread visibility rides the watermark
    // Release/Acquire edges and the pool dispatch/join edges.
    let mut current_frame = unsafe { CurrentFrameMut::from_raw_windowed(wave.current_frame) };

    let mut failure: LoopFilterBandResult = Ok(());
    let mut sb_row = participant;
    while sb_row < wave.sb_rows {
        for sb_col in 0..wave.sb_cols {
            if failure.is_err() {
                break;
            }
            let result = match current_frame.as_mut() {
                Ok(frame) => loop_filter_wavefront_superblock(wave, frame, sb_row, sb_col),
                Err(error) => Err(*error),
            };
            if let Err(error) = result {
                failure = Err(LoopFilterBandError {
                    sb_index: sb_row * wave.sb_cols + sb_col,
                    error,
                });
            }
        }
        // The row must end fully marked even when filtering failed, or
        // participants waiting on it would never wake. Dependents of an
        // abandoned row filter against unfiltered pixels; the error still
        // fails the frame after join, so the output is never used.
        pool::watermark_store(&wave.watermarks[sb_row], wave.sb_cols as u32);
        sb_row += LOOP_FILTER_PARTICIPANTS;
    }
    failure
}

fn loop_filter_wavefront_superblock(
    wave: &LoopFilterWave<'_>,
    current_frame: &mut CurrentFrameMut<'_>,
    sb_row: usize,
    sb_col: usize,
) -> Result<(), TileSyntaxError> {
    if sb_row > 0 {
        let target = core::cmp::min(
            sb_col
                .checked_add(2)
                .ok_or(TileSyntaxError::InvalidBitstream)?,
            wave.sb_cols,
        );
        pool::watermark_wait_at_least(
            &wave.watermarks[sb_row - 1],
            u32::try_from(target).map_err(|_| TileSyntaxError::InvalidBitstream)?,
        );
    }
    let row = sb_row * MI_BLOCK_64;
    let col = sb_col * MI_BLOCK_64;
    current_frame.set_superblock_window(row, col)?;
    loop_filter_superblock_all(current_frame, wave.config, wave.strengths, row, col)?;
    pool::watermark_store(&wave.watermarks[sb_row], sb_col as u32 + 1);
    Ok(())
}

pub(super) fn loop_filter_strength_lut(
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

pub(super) fn loop_filter_superblock_info(
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
            let ref_frame = info.ref_frame;
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

pub(super) fn loop_filter_superblock_mi(
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

pub(super) fn loop_filter_superblock(
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

pub(super) fn loop_filter_on_screen(
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

pub(super) fn loop_filter_mode_info(
    config: LoopFilterConfig<'_>,
    row: usize,
    col: usize,
) -> Result<StoredLoopFilterModeInfo, TileSyntaxError> {
    if row >= config.mi_rows || col >= config.mi_cols {
        return Err(TileSyntaxError::InvalidBitstream);
    }
    let index = row
        .checked_mul(config.mi_cols)
        .and_then(|value| value.checked_add(col))
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    config
        .modes
        .loop_filter_info(index)?
        .ok_or(TileSyntaxError::InvalidBitstream)
}

pub(super) fn loop_filter_is_block_edge(
    pass: usize,
    x: usize,
    y: usize,
    sb_size: BlockSize,
) -> bool {
    // num_8x8_wide/high are powers of two, so the multiple-of test is a mask
    // test; is_multiple_of would lower to a serializing udiv per edge.
    if pass == 0 {
        (x & (8 * usize::from(sb_size.num_8x8_wide()) - 1)) == 0
    } else {
        (y & (8 * usize::from(sb_size.num_8x8_high()) - 1)) == 0
    }
}

pub(super) fn loop_filter_is_tx_edge(
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
pub(super) struct LoopFilterSizeInput {
    pub(super) tx_size: TxSize,
    pub(super) is_32_edge: bool,
    pub(super) pass: usize,
    pub(super) x: usize,
    pub(super) y: usize,
    pub(super) sub_x: usize,
    pub(super) sub_y: usize,
    pub(super) mi_rows: usize,
    pub(super) mi_cols: usize,
}

pub(super) fn loop_filter_size(input: LoopFilterSizeInput) -> TxSize {
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

pub(super) fn loop_filter_level(
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

pub(super) fn loop_filter_strength_from_level(
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

pub(super) fn loop_filter_mode_type(y_mode: u8) -> bool {
    matches!(y_mode, NEARESTMV | NEARMV | NEWMV)
}

pub(super) fn loop_filter_segment(
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

    // Maximal touch rectangle of this segment, clipped to the plane: the
    // widest filter reads 8 samples on each side of the edge, and the
    // clamped fallback path reads 8 regardless of filter size. Checking it
    // here makes every raw access below in-window; during wavefront
    // filtering, simultaneously active windows are disjoint by the
    // watermark lag, so an in-window access can never race.
    let along_end = core::cmp::min(
        if pass == 0 { y } else { x }
            .checked_add(len)
            .ok_or(TileSyntaxError::InvalidBitstream)?,
        if pass == 0 { plane.height } else { plane.width },
    );
    if pass == 0 {
        plane.check_window_rect(
            x.saturating_sub(8),
            core::cmp::min(x.saturating_add(8), plane.width),
            y,
            along_end,
        )?;
    } else {
        plane.check_window_rect(
            x,
            along_end,
            y.saturating_sub(8),
            core::cmp::min(y.saturating_add(8), plane.height),
        )?;
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
            if len == 8 {
                let filtered = match filter_size {
                    TxSize::Tx4x4 => {
                        loop_filter_tx4x4_horizontal_8(plane.data, base, plane.stride, strength)
                    }
                    TxSize::Tx8x8 => {
                        loop_filter_tx8x8_horizontal_8(plane.data, base, plane.stride, strength)
                    }
                    TxSize::Tx16x16 | TxSize::Tx32x32 => {
                        loop_filter_tx16x16_horizontal_8(plane.data, base, plane.stride, strength)
                    }
                };
                if filtered {
                    return Ok(());
                }
            }
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
pub(super) fn loop_filter_tx4x4_horizontal_8(
    data: &mut [u8],
    base: usize,
    stride: usize,
    strength: LoopFilterStrength,
) -> bool {
    let q0 = load_loop_filter_row_8(data, base);
    let q1 = load_loop_filter_row_8(data, base + stride);
    let q2 = load_loop_filter_row_8(data, base + 2 * stride);
    let q3 = load_loop_filter_row_8(data, base + 3 * stride);
    let p0 = load_loop_filter_row_8(data, base - stride);
    let p1 = load_loop_filter_row_8(data, base - 2 * stride);
    let p2 = load_loop_filter_row_8(data, base - 3 * stride);
    let p3 = load_loop_filter_row_8(data, base - 4 * stride);

    let q0s = signed_sample_i16x8(q0);
    let q1s = signed_sample_i16x8(q1);
    let q2s = signed_sample_i16x8(q2);
    let q3s = signed_sample_i16x8(q3);
    let p0s = signed_sample_i16x8(p0);
    let p1s = signed_sample_i16x8(p1);
    let p2s = signed_sample_i16x8(p2);
    let p3s = signed_sample_i16x8(p3);

    let limit = i16x8_splat(i16::from(strength.limit));
    let p3p2 = abs_diff_i16x8(p3s, p2s);
    let p2p1 = abs_diff_i16x8(p2s, p1s);
    let p1p0 = abs_diff_i16x8(p1s, p0s);
    let q1q0 = abs_diff_i16x8(q1s, q0s);
    let q2q1 = abs_diff_i16x8(q2s, q1s);
    let q3q2 = abs_diff_i16x8(q3s, q2s);
    let p0q0 = abs_diff_i16x8(p0s, q0s);
    let p1q1 = abs_diff_i16x8(p1s, q1s);

    let mut masked = i16x8_gt(p3p2, limit);
    masked = v128_or(masked, i16x8_gt(p2p1, limit));
    masked = v128_or(masked, i16x8_gt(p1p0, limit));
    masked = v128_or(masked, i16x8_gt(q1q0, limit));
    masked = v128_or(masked, i16x8_gt(q2q1, limit));
    masked = v128_or(masked, i16x8_gt(q3q2, limit));

    let edge = i16x8_add(i16x8_shl(p0q0, 1), u16x8_shr(p1q1, 1));
    masked = v128_or(
        masked,
        i16x8_gt(edge, i16x8_splat(i16::from(strength.blimit))),
    );

    let filter = v128_not(masked);
    // v128_any_true, not i16x8_bitmask: V8's arm32 bitmask lowering
    // materializes a powers-of-two lane constant that can clobber the aliased
    // high D-half of a live Q register under pressure, miscompiling this
    // kernel (lanes 4..7 corrupted). See docs/log.md 2026-07-03.
    if !v128_any_true(filter) {
        return true;
    }

    let thresh = i16x8_splat(i16::from(strength.thresh));
    let hev = v128_or(i16x8_gt(p1p0, thresh), i16x8_gt(q1q0, thresh));
    let signed_offset = i16x8_splat(128);
    let ps1 = p1s;
    let ps0 = p0s;
    let qs0 = q0s;
    let qs1 = q1s;

    let hev_filter = v128_bitselect(
        filter4_clamp_i16x8(i16x8_sub(ps1, qs1)),
        i16x8_splat(0),
        hev,
    );
    let qs0_ps0 = i16x8_sub(qs0, ps0);
    let filter_value = filter4_clamp_i16x8(i16x8_add(
        hev_filter,
        i16x8_add(i16x8_add(qs0_ps0, qs0_ps0), qs0_ps0),
    ));
    let filter1 = i16x8_shr(
        filter4_clamp_i16x8(i16x8_add(filter_value, i16x8_splat(4))),
        3,
    );
    let filter2 = i16x8_shr(
        filter4_clamp_i16x8(i16x8_add(filter_value, i16x8_splat(3))),
        3,
    );

    let oq0 = i16x8_add(filter4_clamp_i16x8(i16x8_sub(qs0, filter1)), signed_offset);
    let op0 = i16x8_add(filter4_clamp_i16x8(i16x8_add(ps0, filter2)), signed_offset);
    let zero = i16x8_splat(0);
    let filter_bytes = i16x8_mask_to_u8x8(filter);
    store_loop_filter_row_8(
        data,
        base,
        v128_bitselect(u8x16_narrow_i16x8(oq0, zero), q0, filter_bytes),
    );
    store_loop_filter_row_8(
        data,
        base - stride,
        v128_bitselect(u8x16_narrow_i16x8(op0, zero), p0, filter_bytes),
    );

    let filter1_round = i16x8_shr(i16x8_add(filter1, i16x8_splat(1)), 1);
    let oq1 = i16x8_add(
        filter4_clamp_i16x8(i16x8_sub(qs1, filter1_round)),
        signed_offset,
    );
    let op1 = i16x8_add(
        filter4_clamp_i16x8(i16x8_add(ps1, filter1_round)),
        signed_offset,
    );
    let p1q1_filter_bytes = i16x8_mask_to_u8x8(v128_and(filter, v128_not(hev)));
    store_loop_filter_row_8(
        data,
        base + stride,
        v128_bitselect(u8x16_narrow_i16x8(oq1, zero), q1, p1q1_filter_bytes),
    );
    store_loop_filter_row_8(
        data,
        base - 2 * stride,
        v128_bitselect(u8x16_narrow_i16x8(op1, zero), p1, p1q1_filter_bytes),
    );

    true
}

pub(super) struct LoopFilterMasks8 {
    pub(super) filter: v128,
    pub(super) hev: v128,
    pub(super) flat: v128,
}

pub(super) struct LoopFilterNarrowBytes {
    pub(super) p1: v128,
    pub(super) p0: v128,
    pub(super) q0: v128,
    pub(super) q1: v128,
}

pub(super) struct LoopFilterWide3Bytes {
    pub(super) p2: v128,
    pub(super) p1: v128,
    pub(super) p0: v128,
    pub(super) q0: v128,
    pub(super) q1: v128,
    pub(super) q2: v128,
}

pub(super) struct LoopFilterWide4Bytes {
    pub(super) p6: v128,
    pub(super) p5: v128,
    pub(super) p4: v128,
    pub(super) p3: v128,
    pub(super) p2: v128,
    pub(super) p1: v128,
    pub(super) p0: v128,
    pub(super) q0: v128,
    pub(super) q1: v128,
    pub(super) q2: v128,
    pub(super) q3: v128,
    pub(super) q4: v128,
    pub(super) q5: v128,
    pub(super) q6: v128,
}

#[inline(always)]
pub(super) fn loop_filter_tx8x8_horizontal_8(
    data: &mut [u8],
    base: usize,
    stride: usize,
    strength: LoopFilterStrength,
) -> bool {
    let q0 = load_loop_filter_row_8(data, base);
    let q1 = load_loop_filter_row_8(data, base + stride);
    let q2 = load_loop_filter_row_8(data, base + 2 * stride);
    let q3 = load_loop_filter_row_8(data, base + 3 * stride);
    let p0 = load_loop_filter_row_8(data, base - stride);
    let p1 = load_loop_filter_row_8(data, base - 2 * stride);
    let p2 = load_loop_filter_row_8(data, base - 3 * stride);
    let p3 = load_loop_filter_row_8(data, base - 4 * stride);

    let q0s = signed_sample_i16x8(q0);
    let q1s = signed_sample_i16x8(q1);
    let q2s = signed_sample_i16x8(q2);
    let q3s = signed_sample_i16x8(q3);
    let p0s = signed_sample_i16x8(p0);
    let p1s = signed_sample_i16x8(p1);
    let p2s = signed_sample_i16x8(p2);
    let p3s = signed_sample_i16x8(p3);

    let masks = loop_filter_masks_horizontal_8(p3s, p2s, p1s, p0s, q0s, q1s, q2s, q3s, strength);
    // v128_any_true, not i16x8_bitmask: see the Tx4x4 kernel comment.
    if !v128_any_true(masks.filter) {
        return true;
    }

    let wide3_filter = v128_and(masks.filter, masks.flat);
    let narrow_filter = v128_and(masks.filter, v128_not(masks.flat));
    if !v128_any_true(wide3_filter) {
        store_loop_filter_narrow_horizontal_8(
            data,
            base,
            stride,
            p1,
            p0,
            q0,
            q1,
            p1s,
            p0s,
            q0s,
            q1s,
            narrow_filter,
            masks.hev,
        );
        return true;
    }

    let wide3 = loop_filter_wide3_outputs_horizontal_8(
        unsigned_sample_i16x8(p3),
        unsigned_sample_i16x8(p2),
        unsigned_sample_i16x8(p1),
        unsigned_sample_i16x8(p0),
        unsigned_sample_i16x8(q0),
        unsigned_sample_i16x8(q1),
        unsigned_sample_i16x8(q2),
        unsigned_sample_i16x8(q3),
    );

    if !v128_any_true(narrow_filter) {
        store_loop_filter_wide3_horizontal_8(
            data,
            base,
            stride,
            p2,
            p1,
            p0,
            q0,
            q1,
            q2,
            wide3,
            wide3_filter,
        );
        return true;
    }

    let narrow = loop_filter_narrow_outputs_horizontal_8(p1s, p0s, q0s, q1s, masks.hev);
    let wide3_bytes = i16x8_mask_to_u8x8(wide3_filter);
    let narrow_p0q0_bytes = i16x8_mask_to_u8x8(narrow_filter);
    let narrow_p1q1_bytes = i16x8_mask_to_u8x8(v128_and(narrow_filter, v128_not(masks.hev)));

    store_loop_filter_row_8(
        data,
        base - 3 * stride,
        v128_bitselect(wide3.p2, p2, wide3_bytes),
    );
    let p1_mixed = v128_bitselect(narrow.p1, p1, narrow_p1q1_bytes);
    store_loop_filter_row_8(
        data,
        base - 2 * stride,
        v128_bitselect(wide3.p1, p1_mixed, wide3_bytes),
    );
    let p0_mixed = v128_bitselect(narrow.p0, p0, narrow_p0q0_bytes);
    store_loop_filter_row_8(
        data,
        base - stride,
        v128_bitselect(wide3.p0, p0_mixed, wide3_bytes),
    );
    let q0_mixed = v128_bitselect(narrow.q0, q0, narrow_p0q0_bytes);
    store_loop_filter_row_8(data, base, v128_bitselect(wide3.q0, q0_mixed, wide3_bytes));
    let q1_mixed = v128_bitselect(narrow.q1, q1, narrow_p1q1_bytes);
    store_loop_filter_row_8(
        data,
        base + stride,
        v128_bitselect(wide3.q1, q1_mixed, wide3_bytes),
    );
    store_loop_filter_row_8(
        data,
        base + 2 * stride,
        v128_bitselect(wide3.q2, q2, wide3_bytes),
    );

    true
}

#[inline(always)]
pub(super) fn loop_filter_tx16x16_horizontal_8(
    data: &mut [u8],
    base: usize,
    stride: usize,
    strength: LoopFilterStrength,
) -> bool {
    let q0 = load_loop_filter_row_8(data, base);
    let q1 = load_loop_filter_row_8(data, base + stride);
    let q2 = load_loop_filter_row_8(data, base + 2 * stride);
    let q3 = load_loop_filter_row_8(data, base + 3 * stride);
    let p0 = load_loop_filter_row_8(data, base - stride);
    let p1 = load_loop_filter_row_8(data, base - 2 * stride);
    let p2 = load_loop_filter_row_8(data, base - 3 * stride);
    let p3 = load_loop_filter_row_8(data, base - 4 * stride);

    let q0s = signed_sample_i16x8(q0);
    let q1s = signed_sample_i16x8(q1);
    let q2s = signed_sample_i16x8(q2);
    let q3s = signed_sample_i16x8(q3);
    let p0s = signed_sample_i16x8(p0);
    let p1s = signed_sample_i16x8(p1);
    let p2s = signed_sample_i16x8(p2);
    let p3s = signed_sample_i16x8(p3);

    let masks = loop_filter_masks_horizontal_8(p3s, p2s, p1s, p0s, q0s, q1s, q2s, q3s, strength);
    // v128_any_true, not i16x8_bitmask: see the Tx4x4 kernel comment.
    if !v128_any_true(masks.filter) {
        return true;
    }

    let flat_filter = v128_and(masks.filter, masks.flat);
    let narrow_filter = v128_and(masks.filter, v128_not(masks.flat));
    if !v128_any_true(flat_filter) {
        store_loop_filter_narrow_horizontal_8(
            data,
            base,
            stride,
            p1,
            p0,
            q0,
            q1,
            p1s,
            p0s,
            q0s,
            q1s,
            narrow_filter,
            masks.hev,
        );
        return true;
    }

    let q4 = load_loop_filter_row_8(data, base + 4 * stride);
    let q5 = load_loop_filter_row_8(data, base + 5 * stride);
    let q6 = load_loop_filter_row_8(data, base + 6 * stride);
    let q7 = load_loop_filter_row_8(data, base + 7 * stride);
    let p4 = load_loop_filter_row_8(data, base - 5 * stride);
    let p5 = load_loop_filter_row_8(data, base - 6 * stride);
    let p6 = load_loop_filter_row_8(data, base - 7 * stride);
    let p7 = load_loop_filter_row_8(data, base - 8 * stride);

    let q4s = signed_sample_i16x8(q4);
    let q5s = signed_sample_i16x8(q5);
    let q6s = signed_sample_i16x8(q6);
    let q7s = signed_sample_i16x8(q7);
    let p4s = signed_sample_i16x8(p4);
    let p5s = signed_sample_i16x8(p5);
    let p6s = signed_sample_i16x8(p6);
    let p7s = signed_sample_i16x8(p7);

    let flat2 = loop_filter_flat2_horizontal_8(p7s, p6s, p5s, p4s, p0s, q0s, q4s, q5s, q6s, q7s);
    let wide4_filter = v128_and(flat_filter, flat2);
    let wide3_filter = v128_and(flat_filter, v128_not(flat2));
    let has_narrow = v128_any_true(narrow_filter);
    let has_wide3 = v128_any_true(wide3_filter);
    let has_wide4 = v128_any_true(wide4_filter);

    let narrow = if has_narrow {
        loop_filter_narrow_outputs_horizontal_8(p1s, p0s, q0s, q1s, masks.hev)
    } else {
        LoopFilterNarrowBytes { p1, p0, q0, q1 }
    };

    let p3u = unsigned_sample_i16x8(p3);
    let p2u = unsigned_sample_i16x8(p2);
    let p1u = unsigned_sample_i16x8(p1);
    let p0u = unsigned_sample_i16x8(p0);
    let q0u = unsigned_sample_i16x8(q0);
    let q1u = unsigned_sample_i16x8(q1);
    let q2u = unsigned_sample_i16x8(q2);
    let q3u = unsigned_sample_i16x8(q3);

    let wide3 = if has_wide3 {
        loop_filter_wide3_outputs_horizontal_8(p3u, p2u, p1u, p0u, q0u, q1u, q2u, q3u)
    } else {
        LoopFilterWide3Bytes {
            p2,
            p1,
            p0,
            q0,
            q1,
            q2,
        }
    };

    let wide4 = if has_wide4 {
        loop_filter_wide4_outputs_horizontal_8(
            unsigned_sample_i16x8(p7),
            unsigned_sample_i16x8(p6),
            unsigned_sample_i16x8(p5),
            unsigned_sample_i16x8(p4),
            p3u,
            p2u,
            p1u,
            p0u,
            q0u,
            q1u,
            q2u,
            q3u,
            unsigned_sample_i16x8(q4),
            unsigned_sample_i16x8(q5),
            unsigned_sample_i16x8(q6),
            unsigned_sample_i16x8(q7),
        )
    } else {
        LoopFilterWide4Bytes {
            p6,
            p5,
            p4,
            p3,
            p2,
            p1,
            p0,
            q0,
            q1,
            q2,
            q3,
            q4,
            q5,
            q6,
        }
    };

    let narrow_p0q0_bytes = i16x8_mask_to_u8x8(narrow_filter);
    let narrow_p1q1_bytes = i16x8_mask_to_u8x8(v128_and(narrow_filter, v128_not(masks.hev)));
    let wide3_bytes = i16x8_mask_to_u8x8(wide3_filter);
    let wide4_bytes = i16x8_mask_to_u8x8(wide4_filter);

    if has_wide4 {
        store_loop_filter_row_8(
            data,
            base - 7 * stride,
            v128_bitselect(wide4.p6, p6, wide4_bytes),
        );
        store_loop_filter_row_8(
            data,
            base - 6 * stride,
            v128_bitselect(wide4.p5, p5, wide4_bytes),
        );
        store_loop_filter_row_8(
            data,
            base - 5 * stride,
            v128_bitselect(wide4.p4, p4, wide4_bytes),
        );
        store_loop_filter_row_8(
            data,
            base - 4 * stride,
            v128_bitselect(wide4.p3, p3, wide4_bytes),
        );
    }

    let mut p2_mixed = p2;
    if has_wide3 {
        p2_mixed = v128_bitselect(wide3.p2, p2_mixed, wide3_bytes);
    }
    if has_wide4 {
        p2_mixed = v128_bitselect(wide4.p2, p2_mixed, wide4_bytes);
    }
    store_loop_filter_row_8(data, base - 3 * stride, p2_mixed);

    let mut p1_mixed = p1;
    if has_narrow {
        p1_mixed = v128_bitselect(narrow.p1, p1_mixed, narrow_p1q1_bytes);
    }
    if has_wide3 {
        p1_mixed = v128_bitselect(wide3.p1, p1_mixed, wide3_bytes);
    }
    if has_wide4 {
        p1_mixed = v128_bitselect(wide4.p1, p1_mixed, wide4_bytes);
    }
    store_loop_filter_row_8(data, base - 2 * stride, p1_mixed);

    let mut p0_mixed = p0;
    if has_narrow {
        p0_mixed = v128_bitselect(narrow.p0, p0_mixed, narrow_p0q0_bytes);
    }
    if has_wide3 {
        p0_mixed = v128_bitselect(wide3.p0, p0_mixed, wide3_bytes);
    }
    if has_wide4 {
        p0_mixed = v128_bitselect(wide4.p0, p0_mixed, wide4_bytes);
    }
    store_loop_filter_row_8(data, base - stride, p0_mixed);

    let mut q0_mixed = q0;
    if has_narrow {
        q0_mixed = v128_bitselect(narrow.q0, q0_mixed, narrow_p0q0_bytes);
    }
    if has_wide3 {
        q0_mixed = v128_bitselect(wide3.q0, q0_mixed, wide3_bytes);
    }
    if has_wide4 {
        q0_mixed = v128_bitselect(wide4.q0, q0_mixed, wide4_bytes);
    }
    store_loop_filter_row_8(data, base, q0_mixed);

    let mut q1_mixed = q1;
    if has_narrow {
        q1_mixed = v128_bitselect(narrow.q1, q1_mixed, narrow_p1q1_bytes);
    }
    if has_wide3 {
        q1_mixed = v128_bitselect(wide3.q1, q1_mixed, wide3_bytes);
    }
    if has_wide4 {
        q1_mixed = v128_bitselect(wide4.q1, q1_mixed, wide4_bytes);
    }
    store_loop_filter_row_8(data, base + stride, q1_mixed);

    let mut q2_mixed = q2;
    if has_wide3 {
        q2_mixed = v128_bitselect(wide3.q2, q2_mixed, wide3_bytes);
    }
    if has_wide4 {
        q2_mixed = v128_bitselect(wide4.q2, q2_mixed, wide4_bytes);
    }
    store_loop_filter_row_8(data, base + 2 * stride, q2_mixed);

    if has_wide4 {
        store_loop_filter_row_8(
            data,
            base + 3 * stride,
            v128_bitselect(wide4.q3, q3, wide4_bytes),
        );
        store_loop_filter_row_8(
            data,
            base + 4 * stride,
            v128_bitselect(wide4.q4, q4, wide4_bytes),
        );
        store_loop_filter_row_8(
            data,
            base + 5 * stride,
            v128_bitselect(wide4.q5, q5, wide4_bytes),
        );
        store_loop_filter_row_8(
            data,
            base + 6 * stride,
            v128_bitselect(wide4.q6, q6, wide4_bytes),
        );
    }

    true
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) fn loop_filter_masks_horizontal_8(
    p3s: v128,
    p2s: v128,
    p1s: v128,
    p0s: v128,
    q0s: v128,
    q1s: v128,
    q2s: v128,
    q3s: v128,
    strength: LoopFilterStrength,
) -> LoopFilterMasks8 {
    let limit = i16x8_splat(i16::from(strength.limit));
    let p3p2 = abs_diff_i16x8(p3s, p2s);
    let p2p1 = abs_diff_i16x8(p2s, p1s);
    let p1p0 = abs_diff_i16x8(p1s, p0s);
    let q1q0 = abs_diff_i16x8(q1s, q0s);
    let q2q1 = abs_diff_i16x8(q2s, q1s);
    let q3q2 = abs_diff_i16x8(q3s, q2s);
    let p0q0 = abs_diff_i16x8(p0s, q0s);
    let p1q1 = abs_diff_i16x8(p1s, q1s);

    let mut masked = i16x8_gt(p3p2, limit);
    masked = v128_or(masked, i16x8_gt(p2p1, limit));
    masked = v128_or(masked, i16x8_gt(p1p0, limit));
    masked = v128_or(masked, i16x8_gt(q1q0, limit));
    masked = v128_or(masked, i16x8_gt(q2q1, limit));
    masked = v128_or(masked, i16x8_gt(q3q2, limit));

    let edge = i16x8_add(i16x8_shl(p0q0, 1), u16x8_shr(p1q1, 1));
    masked = v128_or(
        masked,
        i16x8_gt(edge, i16x8_splat(i16::from(strength.blimit))),
    );
    let filter = v128_not(masked);

    let thresh = i16x8_splat(i16::from(strength.thresh));
    let hev = v128_or(i16x8_gt(p1p0, thresh), i16x8_gt(q1q0, thresh));

    let one = i16x8_splat(1);
    let mut not_flat = i16x8_gt(p1p0, one);
    not_flat = v128_or(not_flat, i16x8_gt(q1q0, one));
    not_flat = v128_or(not_flat, i16x8_gt(abs_diff_i16x8(p2s, p0s), one));
    not_flat = v128_or(not_flat, i16x8_gt(abs_diff_i16x8(q2s, q0s), one));
    not_flat = v128_or(not_flat, i16x8_gt(abs_diff_i16x8(p3s, p0s), one));
    not_flat = v128_or(not_flat, i16x8_gt(abs_diff_i16x8(q3s, q0s), one));
    let flat = v128_not(not_flat);

    LoopFilterMasks8 { filter, hev, flat }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) fn loop_filter_flat2_horizontal_8(
    p7s: v128,
    p6s: v128,
    p5s: v128,
    p4s: v128,
    p0s: v128,
    q0s: v128,
    q4s: v128,
    q5s: v128,
    q6s: v128,
    q7s: v128,
) -> v128 {
    let one = i16x8_splat(1);
    let mut not_flat2 = i16x8_gt(abs_diff_i16x8(p7s, p0s), one);
    not_flat2 = v128_or(not_flat2, i16x8_gt(abs_diff_i16x8(q7s, q0s), one));
    not_flat2 = v128_or(not_flat2, i16x8_gt(abs_diff_i16x8(p6s, p0s), one));
    not_flat2 = v128_or(not_flat2, i16x8_gt(abs_diff_i16x8(q6s, q0s), one));
    not_flat2 = v128_or(not_flat2, i16x8_gt(abs_diff_i16x8(p5s, p0s), one));
    not_flat2 = v128_or(not_flat2, i16x8_gt(abs_diff_i16x8(q5s, q0s), one));
    not_flat2 = v128_or(not_flat2, i16x8_gt(abs_diff_i16x8(p4s, p0s), one));
    not_flat2 = v128_or(not_flat2, i16x8_gt(abs_diff_i16x8(q4s, q0s), one));
    v128_not(not_flat2)
}

#[inline(always)]
pub(super) fn loop_filter_narrow_outputs_horizontal_8(
    p1s: v128,
    p0s: v128,
    q0s: v128,
    q1s: v128,
    hev: v128,
) -> LoopFilterNarrowBytes {
    let signed_offset = i16x8_splat(128);
    let hev_filter = v128_bitselect(
        filter4_clamp_i16x8(i16x8_sub(p1s, q1s)),
        i16x8_splat(0),
        hev,
    );
    let q0s_p0s = i16x8_sub(q0s, p0s);
    let filter_value = filter4_clamp_i16x8(i16x8_add(
        hev_filter,
        i16x8_add(i16x8_add(q0s_p0s, q0s_p0s), q0s_p0s),
    ));
    let filter1 = i16x8_shr(
        filter4_clamp_i16x8(i16x8_add(filter_value, i16x8_splat(4))),
        3,
    );
    let filter2 = i16x8_shr(
        filter4_clamp_i16x8(i16x8_add(filter_value, i16x8_splat(3))),
        3,
    );

    let oq0 = i16x8_add(filter4_clamp_i16x8(i16x8_sub(q0s, filter1)), signed_offset);
    let op0 = i16x8_add(filter4_clamp_i16x8(i16x8_add(p0s, filter2)), signed_offset);

    let filter1_round = i16x8_shr(i16x8_add(filter1, i16x8_splat(1)), 1);
    let oq1 = i16x8_add(
        filter4_clamp_i16x8(i16x8_sub(q1s, filter1_round)),
        signed_offset,
    );
    let op1 = i16x8_add(
        filter4_clamp_i16x8(i16x8_add(p1s, filter1_round)),
        signed_offset,
    );
    let zero = i16x8_splat(0);
    LoopFilterNarrowBytes {
        p1: u8x16_narrow_i16x8(op1, zero),
        p0: u8x16_narrow_i16x8(op0, zero),
        q0: u8x16_narrow_i16x8(oq0, zero),
        q1: u8x16_narrow_i16x8(oq1, zero),
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) fn store_loop_filter_narrow_horizontal_8(
    data: &mut [u8],
    base: usize,
    stride: usize,
    p1: v128,
    p0: v128,
    q0: v128,
    q1: v128,
    p1s: v128,
    p0s: v128,
    q0s: v128,
    q1s: v128,
    filter: v128,
    hev: v128,
) {
    if !v128_any_true(filter) {
        return;
    }
    let narrow = loop_filter_narrow_outputs_horizontal_8(p1s, p0s, q0s, q1s, hev);
    let filter_bytes = i16x8_mask_to_u8x8(filter);
    store_loop_filter_row_8(data, base, v128_bitselect(narrow.q0, q0, filter_bytes));
    store_loop_filter_row_8(
        data,
        base - stride,
        v128_bitselect(narrow.p0, p0, filter_bytes),
    );

    let p1q1_filter_bytes = i16x8_mask_to_u8x8(v128_and(filter, v128_not(hev)));
    store_loop_filter_row_8(
        data,
        base + stride,
        v128_bitselect(narrow.q1, q1, p1q1_filter_bytes),
    );
    store_loop_filter_row_8(
        data,
        base - 2 * stride,
        v128_bitselect(narrow.p1, p1, p1q1_filter_bytes),
    );
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) fn store_loop_filter_wide3_horizontal_8(
    data: &mut [u8],
    base: usize,
    stride: usize,
    p2: v128,
    p1: v128,
    p0: v128,
    q0: v128,
    q1: v128,
    q2: v128,
    wide3: LoopFilterWide3Bytes,
    filter: v128,
) {
    let filter_bytes = i16x8_mask_to_u8x8(filter);
    store_loop_filter_row_8(
        data,
        base - 3 * stride,
        v128_bitselect(wide3.p2, p2, filter_bytes),
    );
    store_loop_filter_row_8(
        data,
        base - 2 * stride,
        v128_bitselect(wide3.p1, p1, filter_bytes),
    );
    store_loop_filter_row_8(
        data,
        base - stride,
        v128_bitselect(wide3.p0, p0, filter_bytes),
    );
    store_loop_filter_row_8(data, base, v128_bitselect(wide3.q0, q0, filter_bytes));
    store_loop_filter_row_8(
        data,
        base + stride,
        v128_bitselect(wide3.q1, q1, filter_bytes),
    );
    store_loop_filter_row_8(
        data,
        base + 2 * stride,
        v128_bitselect(wide3.q2, q2, filter_bytes),
    );
}

#[inline(always)]
pub(super) fn unsigned_sample_i16x8(samples: v128) -> v128 {
    u16x8_extend_low_u8x16(samples)
}

#[inline(always)]
pub(super) fn add3_i16x8(a: v128, b: v128, c: v128) -> v128 {
    i16x8_add(i16x8_add(a, b), c)
}

#[inline(always)]
pub(super) fn slide_loop_filter_sum_i16x8(
    sum: v128,
    add_a: v128,
    add_b: v128,
    sub_a: v128,
    sub_b: v128,
) -> v128 {
    i16x8_add(
        i16x8_sub(sum, i16x8_add(sub_a, sub_b)),
        i16x8_add(add_a, add_b),
    )
}

#[inline(always)]
pub(super) fn round_loop_filter_sum_u8x8(sum: v128, bits: u32) -> v128 {
    let rounded = i16x8_shr(
        i16x8_add(sum, i16x8_splat((1i32 << (bits - 1)) as i16)),
        bits,
    );
    u8x16_narrow_i16x8(rounded, i16x8_splat(0))
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) fn loop_filter_wide3_outputs_horizontal_8(
    p3: v128,
    p2: v128,
    p1: v128,
    p0: v128,
    q0: v128,
    q1: v128,
    q2: v128,
    q3: v128,
) -> LoopFilterWide3Bytes {
    let p3x3 = i16x8_add(i16x8_add(p3, p3), p3);
    let mut sum = i16x8_add(i16x8_add(p3x3, i16x8_add(p2, p2)), add3_i16x8(p1, p0, q0));
    let out_p2 = round_loop_filter_sum_u8x8(sum, 3);
    sum = slide_loop_filter_sum_i16x8(sum, q1, p1, p3, p2);
    let out_p1 = round_loop_filter_sum_u8x8(sum, 3);
    sum = slide_loop_filter_sum_i16x8(sum, q2, p0, p3, p1);
    let out_p0 = round_loop_filter_sum_u8x8(sum, 3);
    sum = slide_loop_filter_sum_i16x8(sum, q3, q0, p3, p0);
    let out_q0 = round_loop_filter_sum_u8x8(sum, 3);
    sum = slide_loop_filter_sum_i16x8(sum, q3, q1, p2, q0);
    let out_q1 = round_loop_filter_sum_u8x8(sum, 3);
    sum = slide_loop_filter_sum_i16x8(sum, q3, q2, p1, q1);
    let out_q2 = round_loop_filter_sum_u8x8(sum, 3);

    LoopFilterWide3Bytes {
        p2: out_p2,
        p1: out_p1,
        p0: out_p0,
        q0: out_q0,
        q1: out_q1,
        q2: out_q2,
    }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(super) fn loop_filter_wide4_outputs_horizontal_8(
    p7: v128,
    p6: v128,
    p5: v128,
    p4: v128,
    p3: v128,
    p2: v128,
    p1: v128,
    p0: v128,
    q0: v128,
    q1: v128,
    q2: v128,
    q3: v128,
    q4: v128,
    q5: v128,
    q6: v128,
    q7: v128,
) -> LoopFilterWide4Bytes {
    let p7x2 = i16x8_add(p7, p7);
    let p7x4 = i16x8_add(p7x2, p7x2);
    let p7x7 = i16x8_add(i16x8_add(p7x4, p7x2), p7);
    let mut sum = i16x8_add(
        i16x8_add(p7x7, i16x8_add(p6, p6)),
        i16x8_add(
            i16x8_add(add3_i16x8(p5, p4, p3), add3_i16x8(p2, p1, p0)),
            q0,
        ),
    );

    let out_p6 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q1, p5, p7, p6);
    let out_p5 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q2, p4, p7, p5);
    let out_p4 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q3, p3, p7, p4);
    let out_p3 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q4, p2, p7, p3);
    let out_p2 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q5, p1, p7, p2);
    let out_p1 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q6, p0, p7, p1);
    let out_p0 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q7, q0, p7, p0);
    let out_q0 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q7, q1, p6, q0);
    let out_q1 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q7, q2, p5, q1);
    let out_q2 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q7, q3, p4, q2);
    let out_q3 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q7, q4, p3, q3);
    let out_q4 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q7, q5, p2, q4);
    let out_q5 = round_loop_filter_sum_u8x8(sum, 4);
    sum = slide_loop_filter_sum_i16x8(sum, q7, q6, p1, q5);
    let out_q6 = round_loop_filter_sum_u8x8(sum, 4);

    LoopFilterWide4Bytes {
        p6: out_p6,
        p5: out_p5,
        p4: out_p4,
        p3: out_p3,
        p2: out_p2,
        p1: out_p1,
        p0: out_p0,
        q0: out_q0,
        q1: out_q1,
        q2: out_q2,
        q3: out_q3,
        q4: out_q4,
        q5: out_q5,
        q6: out_q6,
    }
}

#[inline(always)]
pub(super) fn load_loop_filter_row_8(data: &[u8], offset: usize) -> v128 {
    unsafe { v128_load64_zero(data.as_ptr().add(offset).cast::<u64>()) }
}

#[inline(always)]
pub(super) fn store_loop_filter_row_8(data: &mut [u8], offset: usize, value: v128) {
    unsafe { v128_store64_lane::<0>(value, data.as_mut_ptr().add(offset).cast::<u64>()) };
}

#[inline(always)]
pub(super) fn signed_sample_i16x8(samples: v128) -> v128 {
    // Convert u8 samples to the signed VP9 loop-filter domain (sample - 128)
    // without a separate subtract.
    i16x8_extend_low_i8x16(v128_xor(samples, u8x16_splat(0x80)))
}

#[inline(always)]
pub(super) fn abs_diff_i16x8(a: v128, b: v128) -> v128 {
    i16x8_abs(i16x8_sub(a, b))
}

#[inline(always)]
pub(super) fn filter4_clamp_i16x8(value: v128) -> v128 {
    i16x8_min(i16x8_max(value, i16x8_splat(-128)), i16x8_splat(127))
}

#[inline(always)]
pub(super) fn i16x8_mask_to_u8x8(mask: v128) -> v128 {
    // The blend mask is held as i16 lanes. Pick each lane's low byte so every
    // stored u8 column gets 0xff for true and 0x00 for false.
    u8x16_shuffle::<0, 2, 4, 6, 8, 10, 12, 14, 16, 16, 16, 16, 16, 16, 16, 16>(mask, i16x8_splat(0))
}

#[inline(always)]
pub(super) fn sample_filter_direct(
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
pub(super) fn narrow_filter_direct(
    data: &mut [u8],
    base: usize,
    step: usize,
    samples: [u8; 4],
    hev: bool,
) {
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
pub(super) fn wide_filter_direct(
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
pub(super) fn loop_filter_direct_sample(p: &[u8; 8], q: &[u8; 8], offset: i32) -> u8 {
    if offset >= 0 {
        q[offset as usize]
    } else {
        p[(-offset - 1) as usize]
    }
}

#[inline(always)]
pub(super) fn loop_filter_direct_index(base: usize, step: usize, offset: i32) -> usize {
    if offset >= 0 {
        base + offset as usize * step
    } else {
        base - ((-offset) as usize) * step
    }
}

pub(super) fn sample_filter(
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

pub(super) fn filter_masks(
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

pub(super) fn narrow_filter(
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

pub(super) fn wide_filter(
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

pub(super) fn loop_filter_sample(
    plane: &CurrentPlaneMut<'_>,
    x: i32,
    y: i32,
) -> Result<u8, TileSyntaxError> {
    if plane.width == 0 || plane.height == 0 {
        return Err(TileSyntaxError::InvalidBitstream);
    }
    let max_x = i32::try_from(plane.width - 1).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let max_y = i32::try_from(plane.height - 1).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let x = usize::try_from(clip3(0, max_x, x)).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    let y = usize::try_from(clip3(0, max_y, y)).map_err(|_| TileSyntaxError::InvalidBitstream)?;
    plane.sample_clamped(x, y)
}

pub(super) fn loop_filter_set(
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

pub(super) fn abs_diff(a: u8, b: u8) -> i32 {
    (i32::from(a) - i32::from(b)).abs()
}

pub(super) fn filter4_clamp(value: i32) -> i32 {
    clip3(-128, 127, value)
}

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn horizontal_loop_filter_segment_matches_scalar_reference() -> Result<(), TileSyntaxError> {
        const FILTER_SIZES: [TxSize; 4] = [
            TxSize::Tx4x4,
            TxSize::Tx8x8,
            TxSize::Tx16x16,
            TxSize::Tx32x32,
        ];
        const LENS: [usize; 4] = [1, 4, 7, 8];
        const STRENGTHS: [LoopFilterStrength; 4] = [
            LoopFilterStrength {
                lvl: 1,
                limit: 1,
                blimit: 3,
                thresh: 0,
            },
            LoopFilterStrength {
                lvl: 16,
                limit: 4,
                blimit: 40,
                thresh: 1,
            },
            LoopFilterStrength {
                lvl: 32,
                limit: 16,
                blimit: 96,
                thresh: 2,
            },
            LoopFilterStrength {
                lvl: 63,
                limit: 63,
                blimit: 193,
                thresh: 3,
            },
        ];

        let mut seed = 0x53a9_1f2du32;
        for filter_size in FILTER_SIZES {
            for strength in STRENGTHS {
                for len in LENS {
                    for trial in 0..8 {
                        let mut initial = [0u8; LOOP_FILTER_TEST_STRIDE * LOOP_FILTER_TEST_HEIGHT];
                        fill_pseudorandom(&mut initial, &mut seed);
                        check_horizontal_loop_filter_segment(
                            initial,
                            len,
                            filter_size,
                            strength,
                            trial,
                        )?;
                    }

                    for pattern in 0..5 {
                        let mut initial = [0u8; LOOP_FILTER_TEST_STRIDE * LOOP_FILTER_TEST_HEIGHT];
                        fill_pseudorandom(&mut initial, &mut seed);
                        paint_loop_filter_pattern(&mut initial, len, strength, pattern);
                        check_horizontal_loop_filter_segment(
                            initial,
                            len,
                            filter_size,
                            strength,
                            100 + pattern,
                        )?;
                    }
                }
            }
        }

        Ok(())
    }

    #[test]
    fn horizontal_wide_loop_filter_kernels_match_scalar_reference_direct()
    -> Result<(), TileSyntaxError> {
        const FILTER_SIZES: [TxSize; 3] = [TxSize::Tx8x8, TxSize::Tx16x16, TxSize::Tx32x32];
        const STRENGTHS: [LoopFilterStrength; 4] = [
            LoopFilterStrength {
                lvl: 1,
                limit: 1,
                blimit: 3,
                thresh: 0,
            },
            LoopFilterStrength {
                lvl: 16,
                limit: 4,
                blimit: 40,
                thresh: 1,
            },
            LoopFilterStrength {
                lvl: 32,
                limit: 16,
                blimit: 96,
                thresh: 2,
            },
            LoopFilterStrength {
                lvl: 63,
                limit: 63,
                blimit: 193,
                thresh: 3,
            },
        ];

        let mut seed = 0x2f37_9badu32;
        for filter_size in FILTER_SIZES {
            for strength in STRENGTHS {
                for pattern in 0..5 {
                    let mut initial = [0u8; LOOP_FILTER_TEST_STRIDE * LOOP_FILTER_TEST_HEIGHT];
                    fill_pseudorandom(&mut initial, &mut seed);
                    paint_loop_filter_pattern(&mut initial, 8, strength, pattern);

                    let mut scalar_data = initial;
                    let mut simd_data = initial;
                    let base = LOOP_FILTER_TEST_Y * LOOP_FILTER_TEST_STRIDE + LOOP_FILTER_TEST_X;
                    for offset in 0..8 {
                        sample_filter_direct(
                            &mut scalar_data,
                            base + offset,
                            LOOP_FILTER_TEST_STRIDE,
                            filter_size,
                            strength,
                        );
                    }

                    let ok = match filter_size {
                        TxSize::Tx8x8 => super::loop_filter_tx8x8_horizontal_8(
                            &mut simd_data,
                            base,
                            LOOP_FILTER_TEST_STRIDE,
                            strength,
                        ),
                        TxSize::Tx16x16 | TxSize::Tx32x32 => {
                            super::loop_filter_tx16x16_horizontal_8(
                                &mut simd_data,
                                base,
                                LOOP_FILTER_TEST_STRIDE,
                                strength,
                            )
                        }
                        TxSize::Tx4x4 => unreachable!(),
                    };
                    assert!(ok);

                    if scalar_data != simd_data {
                        let mismatch = scalar_data
                            .iter()
                            .zip(simd_data.iter())
                            .position(|(scalar, simd)| scalar != simd)
                            .expect("mismatch exists");
                        panic!(
                            "direct wide horizontal loop filter mismatch: pattern={pattern} \
                             filter_size={filter_size:?} strength={strength:?} index={mismatch} \
                             scalar={} simd={}",
                            scalar_data[mismatch], simd_data[mismatch]
                        );
                    }
                }
            }
        }

        Ok(())
    }

    #[test]
    fn loop_filter_simd_filter4_clamp_preserves_negative_in_range() {
        let value = super::filter4_clamp_i16x8(core::arch::wasm32::i16x8_splat(-1));
        let mut lanes = [0i16; 8];
        unsafe {
            core::arch::wasm32::v128_store(
                lanes.as_mut_ptr().cast::<core::arch::wasm32::v128>(),
                value,
            )
        };
        assert_eq!(lanes, [-1; 8]);
    }
}

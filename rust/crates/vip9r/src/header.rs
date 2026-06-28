use crate::bitstream::FixedBitReader;
use crate::error::ParserError;

const NUM_REF_FRAMES: usize = 8;
const REFS_PER_FRAME: usize = 3;
const SIGN_BIAS_FRAMES: usize = 4;
const MAX_REF_FRAMES: usize = 4;
const MAX_MODE_LF_DELTAS: usize = 2;
pub(crate) const MAX_SEGMENTS: usize = 8;
pub(crate) const SEG_LVL_MAX: usize = 4;
pub(crate) const SEG_LVL_ALT_Q: usize = 0;
pub(crate) const SEG_LVL_ALT_L: usize = 1;
pub(crate) const SEG_LVL_REF_FRAME: usize = 2;
pub(crate) const SEG_LVL_SKIP: usize = 3;
const LAST_FRAME: usize = 1;
const MAX_TILE_WIDTH_B64: u32 = 64;
const MIN_TILE_WIDTH_B64: u32 = 4;
const CS_RGB: u32 = 7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrameType {
    Key,
    NonKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InterpolationFilter {
    EightTapSmooth,
    EightTap,
    EightTapSharp,
    Bilinear,
    Switchable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct QuantizationParams {
    base_q_idx: u8,
    delta_q_y_dc: i32,
    delta_q_uv_dc: i32,
    delta_q_uv_ac: i32,
    lossless: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SegmentationParams {
    pub(crate) enabled: bool,
    pub(crate) update_map: bool,
    pub(crate) temporal_update: bool,
    pub(crate) tree_probs: [u8; MAX_SEGMENTS - 1],
    pub(crate) pred_probs: [u8; 3],
    pub(crate) abs_or_delta_update: bool,
    pub(crate) feature_enabled: [[bool; SEG_LVL_MAX]; MAX_SEGMENTS],
    pub(crate) feature_data: [[i16; SEG_LVL_MAX]; MAX_SEGMENTS],
}

impl SegmentationParams {
    pub(crate) const fn disabled() -> Self {
        Self {
            enabled: false,
            update_map: false,
            temporal_update: false,
            tree_probs: [255; MAX_SEGMENTS - 1],
            pred_probs: [255; 3],
            abs_or_delta_update: false,
            feature_enabled: [[false; SEG_LVL_MAX]; MAX_SEGMENTS],
            feature_data: [[0; SEG_LVL_MAX]; MAX_SEGMENTS],
        }
    }

    pub(crate) fn feature_active(self, segment_id: u8, feature: usize) -> bool {
        self.enabled
            && self
                .feature_enabled
                .get(usize::from(segment_id))
                .and_then(|features| features.get(feature))
                .copied()
                .unwrap_or(false)
    }

    pub(crate) fn feature_data(self, segment_id: u8, feature: usize) -> Option<i16> {
        self.feature_data
            .get(usize::from(segment_id))
            .and_then(|features| features.get(feature))
            .copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SegmentationState {
    abs_or_delta_update: bool,
    feature_enabled: [[bool; SEG_LVL_MAX]; MAX_SEGMENTS],
    feature_data: [[i16; SEG_LVL_MAX]; MAX_SEGMENTS],
}

impl SegmentationState {
    const fn new() -> Self {
        Self {
            abs_or_delta_update: false,
            feature_enabled: [[false; SEG_LVL_MAX]; MAX_SEGMENTS],
            feature_data: [[0; SEG_LVL_MAX]; MAX_SEGMENTS],
        }
    }

    fn params(
        self,
        enabled: bool,
        update_map: bool,
        temporal_update: bool,
        tree_probs: [u8; MAX_SEGMENTS - 1],
        pred_probs: [u8; 3],
    ) -> SegmentationParams {
        SegmentationParams {
            enabled,
            update_map,
            temporal_update,
            tree_probs,
            pred_probs,
            abs_or_delta_update: self.abs_or_delta_update,
            feature_enabled: self.feature_enabled,
            feature_data: self.feature_data,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReferenceFrameInfo {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) render_width: u32,
    pub(crate) render_height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HeaderParserState {
    reference_frames: [Option<ReferenceFrameInfo>; NUM_REF_FRAMES],
    loop_filter: LoopFilterState,
    segmentation: SegmentationState,
}

impl HeaderParserState {
    pub(crate) const fn new() -> Self {
        Self {
            reference_frames: [None; NUM_REF_FRAMES],
            loop_filter: LoopFilterState::new(),
            segmentation: SegmentationState::new(),
        }
    }

    fn setup_past_independence(&mut self) {
        self.loop_filter = LoopFilterState::new();
        self.segmentation = SegmentationState::new();
    }

    pub(crate) fn update_references(&mut self, header: &UncompressedFrameHeader) {
        if header.show_existing_frame {
            return;
        }

        let reference = ReferenceFrameInfo {
            width: header.frame_width,
            height: header.frame_height,
            render_width: header.render_width,
            render_height: header.render_height,
        };

        for (index, slot) in self.reference_frames.iter_mut().enumerate() {
            if header.refresh_frame_flags & (1u8 << index) != 0 {
                *slot = Some(reference);
            }
        }
    }

    fn reference(&self, index: u8) -> Result<ReferenceFrameInfo, ParserError> {
        self.reference_frames[usize::from(index)].ok_or(ParserError::InvalidBitstream)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LoopFilterState {
    ref_deltas: [i8; MAX_REF_FRAMES],
    mode_deltas: [i8; MAX_MODE_LF_DELTAS],
}

impl LoopFilterState {
    const fn new() -> Self {
        Self {
            ref_deltas: [1, 0, -1, -1],
            mode_deltas: [0, 0],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LoopFilterParams {
    pub(crate) level: u8,
    pub(crate) sharpness: u8,
    pub(crate) delta_enabled: bool,
    pub(crate) ref_deltas: [i8; MAX_REF_FRAMES],
    pub(crate) mode_deltas: [i8; MAX_MODE_LF_DELTAS],
}

impl LoopFilterParams {
    pub(crate) const fn disabled() -> Self {
        Self {
            level: 0,
            sharpness: 0,
            delta_enabled: false,
            ref_deltas: [0; MAX_REF_FRAMES],
            mode_deltas: [0; MAX_MODE_LF_DELTAS],
        }
    }
}

/// Fields parsed from the uncompressed header and retained for upcoming decode stages.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UncompressedFrameHeader {
    pub(crate) profile: u8,
    pub(crate) bit_depth: u8,
    pub(crate) frame_type: FrameType,
    pub(crate) show_frame: bool,
    pub(crate) show_existing_frame: bool,
    pub(crate) frame_to_show_map_idx: Option<u8>,
    pub(crate) error_resilient_mode: bool,
    pub(crate) intra_only: bool,
    pub(crate) frame_is_intra: bool,
    pub(crate) reset_frame_context: u8,
    pub(crate) refresh_frame_context: bool,
    pub(crate) frame_parallel_decoding_mode: bool,
    pub(crate) raw_frame_context_idx: u8,
    pub(crate) frame_context_idx: u8,
    pub(crate) refresh_frame_flags: u8,
    pub(crate) ref_frame_idx: [u8; REFS_PER_FRAME],
    pub(crate) ref_frame_sign_bias: [bool; SIGN_BIAS_FRAMES],
    pub(crate) allow_high_precision_mv: bool,
    pub(crate) interpolation_filter: Option<InterpolationFilter>,
    pub(crate) frame_width: u32,
    pub(crate) frame_height: u32,
    pub(crate) render_width: u32,
    pub(crate) render_height: u32,
    pub(crate) base_q_idx: u8,
    pub(crate) delta_q_y_dc: i32,
    pub(crate) delta_q_uv_dc: i32,
    pub(crate) delta_q_uv_ac: i32,
    pub(crate) lossless: bool,
    pub(crate) loop_filter: LoopFilterParams,
    pub(crate) segmentation: SegmentationParams,
    pub(crate) segmentation_enabled: bool,
    pub(crate) segmentation_update_map: bool,
    pub(crate) tile_cols_log2: u8,
    pub(crate) tile_rows_log2: u8,
    pub(crate) header_size_in_bytes: usize,
    pub(crate) compressed_header_offset: usize,
    pub(crate) tile_data_offset: usize,
}

pub(crate) fn parse_uncompressed_frame_header(
    frame: &[u8],
    state: &mut HeaderParserState,
) -> Result<UncompressedFrameHeader, ParserError> {
    let mut reader = FixedBitReader::new(frame);

    let frame_marker = reader.read_f(2)?;
    if frame_marker != 0b10 {
        return Err(ParserError::InvalidBitstream);
    }

    let profile_low_bit = reader.read_f(1)? as u8;
    let profile_high_bit = reader.read_f(1)? as u8;
    let profile = (profile_high_bit << 1) | profile_low_bit;
    if profile == 3 {
        let reserved_zero = reader.read_f(1)?;
        if reserved_zero != 0 {
            return Err(ParserError::InvalidBitstream);
        }
    }

    if matches!(profile, 1 | 3) {
        return Err(ParserError::UnsupportedProfile(profile));
    }

    let show_existing_frame = reader.read_bool()?;
    if show_existing_frame {
        if profile == 2 {
            return Err(ParserError::UnsupportedBitDepth(10));
        }

        let frame_to_show_map_idx = reader.read_f(3)? as u8;
        let reference = state.reference(frame_to_show_map_idx)?;
        let compressed_header_offset = finish_uncompressed_header(&mut reader, 0, frame.len())?.0;
        return Ok(UncompressedFrameHeader {
            profile,
            bit_depth: 8,
            frame_type: FrameType::NonKey,
            show_frame: true,
            show_existing_frame: true,
            frame_to_show_map_idx: Some(frame_to_show_map_idx),
            error_resilient_mode: false,
            intra_only: false,
            frame_is_intra: false,
            reset_frame_context: 0,
            refresh_frame_context: false,
            frame_parallel_decoding_mode: false,
            raw_frame_context_idx: 0,
            frame_context_idx: 0,
            refresh_frame_flags: 0,
            ref_frame_idx: [0; REFS_PER_FRAME],
            ref_frame_sign_bias: [false; SIGN_BIAS_FRAMES],
            allow_high_precision_mv: false,
            interpolation_filter: None,
            frame_width: reference.width,
            frame_height: reference.height,
            render_width: reference.render_width,
            render_height: reference.render_height,
            base_q_idx: 0,
            delta_q_y_dc: 0,
            delta_q_uv_dc: 0,
            delta_q_uv_ac: 0,
            lossless: false,
            loop_filter: LoopFilterParams::disabled(),
            segmentation: SegmentationParams::disabled(),
            segmentation_enabled: false,
            segmentation_update_map: false,
            tile_cols_log2: 0,
            tile_rows_log2: 0,
            header_size_in_bytes: 0,
            compressed_header_offset,
            tile_data_offset: compressed_header_offset,
        });
    }

    let frame_type = if reader.read_bool()? {
        FrameType::NonKey
    } else {
        FrameType::Key
    };
    let show_frame = reader.read_bool()?;
    let error_resilient_mode = reader.read_bool()?;

    let mut bit_depth = 8u8;
    let intra_only;
    let frame_is_intra;
    let refresh_frame_flags;
    let mut ref_frame_idx = [0u8; REFS_PER_FRAME];
    let mut ref_frame_sign_bias = [false; SIGN_BIAS_FRAMES];
    let mut allow_high_precision_mv = false;
    let mut interpolation_filter = None;
    let reset_frame_context;
    let frame_width;
    let frame_height;
    let render_width;
    let render_height;

    if frame_type == FrameType::Key {
        frame_sync_code(&mut reader)?;
        bit_depth = color_config(&mut reader, profile)?;
        let size = frame_size(&mut reader)?;
        frame_width = size.0;
        frame_height = size.1;
        let render_size = render_size(&mut reader, frame_width, frame_height)?;
        render_width = render_size.0;
        render_height = render_size.1;
        refresh_frame_flags = 0xff;
        intra_only = false;
        frame_is_intra = true;
        reset_frame_context = 0;
    } else {
        intra_only = if show_frame {
            false
        } else {
            reader.read_bool()?
        };
        frame_is_intra = intra_only;

        reset_frame_context = if error_resilient_mode {
            0
        } else {
            reader.read_f(2)? as u8
        };

        if intra_only {
            frame_sync_code(&mut reader)?;
            if profile > 0 {
                bit_depth = color_config(&mut reader, profile)?;
            }
            refresh_frame_flags = reader.read_f(8)? as u8;
            let size = frame_size(&mut reader)?;
            frame_width = size.0;
            frame_height = size.1;
            let render_size = render_size(&mut reader, frame_width, frame_height)?;
            render_width = render_size.0;
            render_height = render_size.1;
        } else {
            if profile == 2 {
                return Err(ParserError::UnsupportedBitDepth(10));
            }

            refresh_frame_flags = reader.read_f(8)? as u8;
            for (index, ref_idx) in ref_frame_idx.iter_mut().enumerate() {
                *ref_idx = reader.read_f(3)? as u8;
                ref_frame_sign_bias[LAST_FRAME + index] = reader.read_bool()?;
            }
            for &ref_idx in &ref_frame_idx {
                state.reference(ref_idx)?;
            }

            let size = frame_size_with_refs(&mut reader, state, ref_frame_idx)?;
            frame_width = size.0;
            frame_height = size.1;
            render_width = size.2;
            render_height = size.3;
            validate_inter_frame_size(frame_width, frame_height, state, ref_frame_idx)?;

            allow_high_precision_mv = reader.read_bool()?;
            interpolation_filter = Some(read_interpolation_filter(&mut reader)?);
        }
    }

    let (refresh_frame_context, frame_parallel_decoding_mode) = if error_resilient_mode {
        (false, true)
    } else {
        (reader.read_bool()?, reader.read_bool()?)
    };
    let raw_frame_context_idx = reader.read_f(2)? as u8;
    let mut frame_context_idx = raw_frame_context_idx;
    if frame_is_intra || error_resilient_mode {
        state.setup_past_independence();
        frame_context_idx = 0;
    }

    let loop_filter = loop_filter_params(&mut reader, &mut state.loop_filter)?;
    let quantization = quantization_params(&mut reader)?;
    let segmentation = segmentation_params(&mut reader, &mut state.segmentation)?;
    let tile_info = tile_info(&mut reader, frame_width)?;
    let header_size_in_bytes = reader.read_f(16)? as usize;
    let (compressed_header_offset, tile_data_offset) =
        finish_uncompressed_header(&mut reader, header_size_in_bytes, frame.len())?;

    Ok(UncompressedFrameHeader {
        profile,
        bit_depth,
        frame_type,
        show_frame,
        show_existing_frame: false,
        frame_to_show_map_idx: None,
        error_resilient_mode,
        intra_only,
        frame_is_intra,
        reset_frame_context,
        refresh_frame_context,
        frame_parallel_decoding_mode,
        raw_frame_context_idx,
        frame_context_idx,
        refresh_frame_flags,
        ref_frame_idx,
        ref_frame_sign_bias,
        allow_high_precision_mv,
        interpolation_filter,
        frame_width,
        frame_height,
        render_width,
        render_height,
        base_q_idx: quantization.base_q_idx,
        delta_q_y_dc: quantization.delta_q_y_dc,
        delta_q_uv_dc: quantization.delta_q_uv_dc,
        delta_q_uv_ac: quantization.delta_q_uv_ac,
        lossless: quantization.lossless,
        loop_filter,
        segmentation,
        segmentation_enabled: segmentation.enabled,
        segmentation_update_map: segmentation.update_map,
        tile_cols_log2: tile_info.0,
        tile_rows_log2: tile_info.1,
        header_size_in_bytes,
        compressed_header_offset,
        tile_data_offset,
    })
}

fn frame_sync_code(reader: &mut FixedBitReader<'_>) -> Result<(), ParserError> {
    let sync = [reader.read_f(8)?, reader.read_f(8)?, reader.read_f(8)?];
    if sync != [0x49, 0x83, 0x42] {
        return Err(ParserError::InvalidBitstream);
    }
    Ok(())
}

fn color_config(reader: &mut FixedBitReader<'_>, profile: u8) -> Result<u8, ParserError> {
    let bit_depth = if profile >= 2 {
        if reader.read_bool()? { 12 } else { 10 }
    } else {
        8
    };
    if bit_depth != 8 {
        return Err(ParserError::UnsupportedBitDepth(bit_depth));
    }

    let color_space = reader.read_f(3)?;
    if color_space == CS_RGB && profile & 1 == 0 {
        return Err(ParserError::InvalidBitstream);
    }

    if color_space != CS_RGB {
        let _color_range = reader.read_bool()?;
        if profile == 1 || profile == 3 {
            let _subsampling_x = reader.read_bool()?;
            let _subsampling_y = reader.read_bool()?;
            let reserved_zero = reader.read_bool()?;
            if reserved_zero {
                return Err(ParserError::InvalidBitstream);
            }
        }
    } else if profile == 1 || profile == 3 {
        let reserved_zero = reader.read_bool()?;
        if reserved_zero {
            return Err(ParserError::InvalidBitstream);
        }
    }

    Ok(bit_depth)
}

fn frame_size(reader: &mut FixedBitReader<'_>) -> Result<(u32, u32), ParserError> {
    let width = reader.read_f(16)? + 1;
    let height = reader.read_f(16)? + 1;
    Ok((width, height))
}

fn render_size(
    reader: &mut FixedBitReader<'_>,
    frame_width: u32,
    frame_height: u32,
) -> Result<(u32, u32), ParserError> {
    if reader.read_bool()? {
        let render_width = reader.read_f(16)? + 1;
        let render_height = reader.read_f(16)? + 1;
        Ok((render_width, render_height))
    } else {
        Ok((frame_width, frame_height))
    }
}

fn frame_size_with_refs(
    reader: &mut FixedBitReader<'_>,
    state: &HeaderParserState,
    ref_frame_idx: [u8; REFS_PER_FRAME],
) -> Result<(u32, u32, u32, u32), ParserError> {
    let mut dimensions = None;
    for ref_idx in ref_frame_idx {
        if reader.read_bool()? {
            let reference = state.reference(ref_idx)?;
            dimensions = Some((reference.width, reference.height));
            break;
        }
    }

    let (frame_width, frame_height) = match dimensions {
        Some(dimensions) => dimensions,
        None => frame_size(reader)?,
    };
    let (render_width, render_height) = render_size(reader, frame_width, frame_height)?;
    Ok((frame_width, frame_height, render_width, render_height))
}

fn validate_inter_frame_size(
    frame_width: u32,
    frame_height: u32,
    state: &HeaderParserState,
    ref_frame_idx: [u8; REFS_PER_FRAME],
) -> Result<(), ParserError> {
    let width = u64::from(frame_width);
    let height = u64::from(frame_height);

    for ref_idx in ref_frame_idx {
        let reference = state.reference(ref_idx)?;
        let ref_width = u64::from(reference.width);
        let ref_height = u64::from(reference.height);
        if 2 * width >= ref_width
            && 2 * height >= ref_height
            && width <= 16 * ref_width
            && height <= 16 * ref_height
        {
            return Ok(());
        }
    }

    Err(ParserError::InvalidBitstream)
}

fn read_interpolation_filter(
    reader: &mut FixedBitReader<'_>,
) -> Result<InterpolationFilter, ParserError> {
    const LITERAL_TO_TYPE: [InterpolationFilter; 4] = [
        InterpolationFilter::EightTapSmooth,
        InterpolationFilter::EightTap,
        InterpolationFilter::EightTapSharp,
        InterpolationFilter::Bilinear,
    ];

    let is_filter_switchable = reader.read_bool()?;
    if is_filter_switchable {
        Ok(InterpolationFilter::Switchable)
    } else {
        let raw_interpolation_filter = reader.read_f(2)? as usize;
        Ok(LITERAL_TO_TYPE[raw_interpolation_filter])
    }
}

fn loop_filter_params(
    reader: &mut FixedBitReader<'_>,
    state: &mut LoopFilterState,
) -> Result<LoopFilterParams, ParserError> {
    let loop_filter_level = reader.read_f(6)? as u8;
    let loop_filter_sharpness = reader.read_f(3)? as u8;
    let loop_filter_delta_enabled = reader.read_bool()?;
    if loop_filter_delta_enabled {
        let loop_filter_delta_update = reader.read_bool()?;
        if loop_filter_delta_update {
            for ref_delta in &mut state.ref_deltas {
                if reader.read_bool()? {
                    *ref_delta = i8::try_from(reader.read_s(6)?)
                        .map_err(|_| ParserError::InvalidBitstream)?;
                }
            }
            for mode_delta in &mut state.mode_deltas {
                if reader.read_bool()? {
                    *mode_delta = i8::try_from(reader.read_s(6)?)
                        .map_err(|_| ParserError::InvalidBitstream)?;
                }
            }
        }
    }
    Ok(LoopFilterParams {
        level: loop_filter_level,
        sharpness: loop_filter_sharpness,
        delta_enabled: loop_filter_delta_enabled,
        ref_deltas: state.ref_deltas,
        mode_deltas: state.mode_deltas,
    })
}

fn quantization_params(reader: &mut FixedBitReader<'_>) -> Result<QuantizationParams, ParserError> {
    let base_q_idx = reader.read_f(8)? as u8;
    let delta_q_y_dc = read_delta_q(reader)?;
    let delta_q_uv_dc = read_delta_q(reader)?;
    let delta_q_uv_ac = read_delta_q(reader)?;
    let lossless = base_q_idx == 0 && delta_q_y_dc == 0 && delta_q_uv_dc == 0 && delta_q_uv_ac == 0;

    Ok(QuantizationParams {
        base_q_idx,
        delta_q_y_dc,
        delta_q_uv_dc,
        delta_q_uv_ac,
        lossless,
    })
}

fn read_delta_q(reader: &mut FixedBitReader<'_>) -> Result<i32, ParserError> {
    if reader.read_bool()? {
        reader.read_s(4)
    } else {
        Ok(0)
    }
}

fn segmentation_params(
    reader: &mut FixedBitReader<'_>,
    state: &mut SegmentationState,
) -> Result<SegmentationParams, ParserError> {
    let segmentation_enabled = reader.read_bool()?;
    if !segmentation_enabled {
        return Ok(state.params(false, false, false, [255; MAX_SEGMENTS - 1], [255; 3]));
    }

    let segmentation_update_map = reader.read_bool()?;
    let mut segmentation_tree_probs = [255; MAX_SEGMENTS - 1];
    let mut segmentation_temporal_update = false;
    let mut segmentation_pred_prob = [255; 3];
    if segmentation_update_map {
        for prob in &mut segmentation_tree_probs {
            *prob = read_prob(reader)?;
        }
        segmentation_temporal_update = reader.read_bool()?;
        if segmentation_temporal_update {
            for prob in &mut segmentation_pred_prob {
                *prob = read_prob(reader)?;
            }
        }
    }

    let segmentation_update_data = reader.read_bool()?;
    if segmentation_update_data {
        let segmentation_abs_or_delta_update = reader.read_bool()?;
        state.abs_or_delta_update = segmentation_abs_or_delta_update;
        for segment in 0..MAX_SEGMENTS {
            for feature in 0..SEG_LVL_MAX {
                let feature_enabled = reader.read_bool()?;
                state.feature_enabled[segment][feature] = feature_enabled;
                let mut feature_value = 0i16;
                if feature_enabled {
                    let bits_to_read = segmentation_feature_bits(feature);
                    feature_value = if bits_to_read > 0 {
                        i16::try_from(reader.read_f(bits_to_read)?)
                            .map_err(|_| ParserError::InvalidBitstream)?
                    } else {
                        0
                    };
                    if segmentation_feature_signed(feature) {
                        let feature_sign = reader.read_bool()?;
                        if segmentation_abs_or_delta_update && feature_sign {
                            return Err(ParserError::InvalidBitstream);
                        }
                        if feature_sign {
                            feature_value = -feature_value;
                        }
                    }
                }
                state.feature_data[segment][feature] = feature_value;
            }
        }
    }

    Ok(state.params(
        true,
        segmentation_update_map,
        segmentation_temporal_update,
        segmentation_tree_probs,
        segmentation_pred_prob,
    ))
}

fn read_prob(reader: &mut FixedBitReader<'_>) -> Result<u8, ParserError> {
    if reader.read_bool()? {
        Ok(reader.read_f(8)? as u8)
    } else {
        Ok(255)
    }
}

fn segmentation_feature_bits(feature: usize) -> u8 {
    [8, 6, 2, 0][feature]
}

fn segmentation_feature_signed(feature: usize) -> bool {
    [true, true, false, false][feature]
}

fn tile_info(reader: &mut FixedBitReader<'_>, frame_width: u32) -> Result<(u8, u8), ParserError> {
    let mi_cols = frame_width.div_ceil(8);
    let sb64_cols = mi_cols.div_ceil(8);

    let min_log2_tile_cols = calc_min_log2_tile_cols(sb64_cols);
    let max_log2_tile_cols = calc_max_log2_tile_cols(sb64_cols);
    let mut tile_cols_log2 = min_log2_tile_cols;
    while tile_cols_log2 < max_log2_tile_cols {
        if reader.read_bool()? {
            tile_cols_log2 += 1;
        } else {
            break;
        }
    }

    let mut tile_rows_log2 = if reader.read_bool()? { 1 } else { 0 };
    if tile_rows_log2 == 1 && reader.read_bool()? {
        tile_rows_log2 += 1;
    }

    if tile_cols_log2 > 6 {
        return Err(ParserError::InvalidBitstream);
    }

    Ok((tile_cols_log2, tile_rows_log2))
}

fn calc_min_log2_tile_cols(sb64_cols: u32) -> u8 {
    let mut min_log2 = 0u8;
    while (MAX_TILE_WIDTH_B64 << min_log2) < sb64_cols {
        min_log2 += 1;
    }
    min_log2
}

fn calc_max_log2_tile_cols(sb64_cols: u32) -> u8 {
    let mut max_log2 = 1u8;
    while (sb64_cols >> max_log2) >= MIN_TILE_WIDTH_B64 {
        max_log2 += 1;
    }
    max_log2 - 1
}

fn finish_uncompressed_header(
    reader: &mut FixedBitReader<'_>,
    header_size_in_bytes: usize,
    frame_len: usize,
) -> Result<(usize, usize), ParserError> {
    while reader.bit_position() & 7 != 0 {
        if reader.read_bool()? {
            return Err(ParserError::InvalidBitstream);
        }
    }

    let compressed_header_offset = reader.bit_position() / 8;
    let tile_data_offset = compressed_header_offset
        .checked_add(header_size_in_bytes)
        .ok_or(ParserError::InvalidBitstream)?;
    if tile_data_offset > frame_len {
        return Err(ParserError::InvalidBitstream);
    }
    Ok((compressed_header_offset, tile_data_offset))
}

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::{
        FrameType, HeaderParserState, InterpolationFilter, ParserError, ReferenceFrameInfo,
        UncompressedFrameHeader, parse_uncompressed_frame_header,
    };

    #[test]
    fn parses_minimal_profile0_key_frame_header() {
        let mut builder = HeaderBuilder::new();
        builder.f(0b10, 2); // frame marker
        builder.f(0, 1); // profile low
        builder.f(0, 1); // profile high
        builder.f(0, 1); // not show existing frame
        builder.f(0, 1); // key frame
        builder.f(1, 1); // show frame
        builder.f(0, 1); // error resilient mode
        builder.f(0x49, 8);
        builder.f(0x83, 8);
        builder.f(0x42, 8);
        builder.f(1, 3); // BT.601 color space
        builder.f(0, 1); // studio range
        builder.f(319, 16);
        builder.f(239, 16);
        builder.f(0, 1); // render size matches frame size
        builder.f(1, 1); // refresh frame context
        builder.f(0, 1); // frame parallel decoding mode
        builder.f(3, 2); // frame context idx (reset to 0 for intra frames)
        builder.f(0, 6); // loop filter level
        builder.f(0, 3); // loop filter sharpness
        builder.f(0, 1); // loop filter delta enabled
        builder.f(0, 8); // base q idx
        builder.f(0, 1); // y dc delta absent
        builder.f(0, 1); // uv dc delta absent
        builder.f(0, 1); // uv ac delta absent
        builder.f(0, 1); // segmentation disabled
        builder.f(0, 1); // tile rows log2
        builder.f(1, 16); // compressed header size
        builder.byte_align_zero();
        builder.byte(0); // one compressed-header byte so offsets are in range
        let frame = builder.finish();

        let header =
            parse_uncompressed_frame_header(&frame, &mut HeaderParserState::new()).unwrap();

        assert_eq!(header.profile, 0);
        assert_eq!(header.bit_depth, 8);
        assert_eq!(header.frame_type, FrameType::Key);
        assert!(header.show_frame);
        assert!(header.frame_is_intra);
        assert!(header.refresh_frame_context);
        assert!(!header.frame_parallel_decoding_mode);
        assert_eq!(header.frame_context_idx, 0);
        assert_eq!(header.refresh_frame_flags, 0xff);
        assert!(!header.allow_high_precision_mv);
        assert!(header.interpolation_filter.is_none());
        assert_eq!(header.base_q_idx, 0);
        assert_eq!(header.delta_q_y_dc, 0);
        assert_eq!(header.delta_q_uv_dc, 0);
        assert_eq!(header.delta_q_uv_ac, 0);
        assert!(header.lossless);
        assert_eq!(header.frame_width, 320);
        assert_eq!(header.frame_height, 240);
        assert_eq!(header.render_width, 320);
        assert_eq!(header.render_height, 240);
        assert_eq!(header.tile_cols_log2, 0);
        assert_eq!(header.tile_rows_log2, 0);
        assert_eq!(header.header_size_in_bytes, 1);
        assert_eq!(header.tile_data_offset, header.compressed_header_offset + 1);
    }

    #[test]
    fn quantization_is_non_lossless_when_base_or_any_delta_is_nonzero() {
        let cases = [
            (7, [0, 0, 0]),
            (0, [1, 0, 0]),
            (0, [0, -2, 0]),
            (0, [0, 0, 3]),
        ];

        for (base_q_idx, deltas) in cases {
            let header = parse_key_frame_with_quant(base_q_idx, deltas);

            assert_eq!(header.base_q_idx, base_q_idx);
            assert_eq!(header.delta_q_y_dc, deltas[0]);
            assert_eq!(header.delta_q_uv_dc, deltas[1]);
            assert_eq!(header.delta_q_uv_ac, deltas[2]);
            assert!(!header.lossless);
        }
    }

    #[test]
    fn show_existing_frame_retains_absent_compressed_header_state() {
        let mut state = HeaderParserState::new();
        state.reference_frames[2] = Some(ReferenceFrameInfo {
            width: 320,
            height: 240,
            render_width: 320,
            render_height: 240,
        });

        let mut builder = HeaderBuilder::new();
        builder.f(0b10, 2); // frame marker
        builder.f(0, 1); // profile low
        builder.f(0, 1); // profile high
        builder.f(1, 1); // show existing frame
        builder.f(2, 3); // frame to show map index
        let frame = builder.finish();

        let header = parse_uncompressed_frame_header(&frame, &mut state).unwrap();

        assert!(header.show_existing_frame);
        assert_eq!(header.frame_to_show_map_idx, Some(2));
        assert_eq!(header.header_size_in_bytes, 0);
        assert_eq!(header.compressed_header_offset, 1);
        assert_eq!(header.tile_data_offset, 1);
        assert!(!header.refresh_frame_context);
        assert!(!header.frame_parallel_decoding_mode);
        assert_eq!(header.frame_context_idx, 0);
        assert!(!header.allow_high_precision_mv);
        assert!(header.interpolation_filter.is_none());
        assert_eq!(header.base_q_idx, 0);
        assert!(!header.lossless);
    }

    #[test]
    fn rejects_bad_key_frame_sync_code() {
        let mut builder = HeaderBuilder::new();
        builder.f(0b10, 2);
        builder.f(0, 1);
        builder.f(0, 1);
        builder.f(0, 1);
        builder.f(0, 1);
        builder.f(1, 1);
        builder.f(0, 1);
        builder.f(0x49, 8);
        builder.f(0x83, 8);
        builder.f(0x43, 8);

        assert_eq!(
            parse_uncompressed_frame_header(&builder.finish(), &mut HeaderParserState::new()),
            Err(ParserError::InvalidBitstream)
        );
    }

    #[test]
    fn inter_frame_uses_reference_size_when_signalled() {
        let mut state = HeaderParserState::new();
        state.reference_frames[0] = Some(ReferenceFrameInfo {
            width: 320,
            height: 240,
            render_width: 320,
            render_height: 240,
        });
        state.reference_frames[1] = state.reference_frames[0];
        state.reference_frames[2] = state.reference_frames[0];

        let mut builder = HeaderBuilder::new();
        builder.f(0b10, 2);
        builder.f(0, 1);
        builder.f(0, 1);
        builder.f(0, 1); // not show existing
        builder.f(1, 1); // non-key
        builder.f(1, 1); // show frame
        builder.f(0, 1); // not error resilient
        builder.f(0, 2); // reset frame context
        builder.f(0x01, 8); // refresh flags
        for idx in 0..3 {
            builder.f(idx, 3);
            builder.f(0, 1);
        }
        builder.f(1, 1); // found LAST ref size
        builder.f(0, 1); // render size matches
        builder.f(1, 1); // allow high precision mv
        builder.f(1, 1); // switchable interpolation filter
        builder.f(1, 1); // refresh frame context
        builder.f(0, 1); // frame parallel decoding mode
        builder.f(2, 2); // frame context idx
        builder.f(0, 6);
        builder.f(0, 3);
        builder.f(0, 1);
        builder.f(0, 8);
        builder.f(0, 1);
        builder.f(0, 1);
        builder.f(0, 1);
        builder.f(0, 1);
        builder.f(0, 1); // tile rows log2
        builder.f(1, 16);
        builder.byte_align_zero();
        builder.byte(0);
        let frame = builder.finish();

        let header = parse_uncompressed_frame_header(&frame, &mut state).unwrap();

        assert_eq!(header.frame_width, 320);
        assert_eq!(header.frame_height, 240);
        assert!(!header.frame_is_intra);
        assert_eq!(header.ref_frame_idx, [0, 1, 2]);
        assert!(header.allow_high_precision_mv);
        assert_eq!(
            header.interpolation_filter,
            Some(InterpolationFilter::Switchable)
        );
        assert!(header.refresh_frame_context);
        assert!(!header.frame_parallel_decoding_mode);
        assert_eq!(header.frame_context_idx, 2);
        assert_eq!(header.base_q_idx, 0);
        assert!(header.lossless);
    }

    #[test]
    fn profile0_interpolation_raw_filter_mapping_follows_spec_literal_to_type() {
        let expected = [
            InterpolationFilter::EightTapSmooth,
            InterpolationFilter::EightTap,
            InterpolationFilter::EightTapSharp,
            InterpolationFilter::Bilinear,
        ];

        for (raw_filter, expected_filter) in expected.into_iter().enumerate() {
            let header = parse_inter_frame_with_raw_interpolation(raw_filter as u8);

            assert_eq!(header.interpolation_filter, Some(expected_filter));
        }
    }

    #[test]
    fn inter_frame_rejects_missing_reference_dimensions() {
        let mut builder = HeaderBuilder::new();
        builder.f(0b10, 2);
        builder.f(0, 1);
        builder.f(0, 1);
        builder.f(0, 1);
        builder.f(1, 1);
        builder.f(1, 1);
        builder.f(0, 1);
        builder.f(0, 2);
        builder.f(0, 8);
        for _ in 0..3 {
            builder.f(0, 3);
            builder.f(0, 1);
        }

        assert_eq!(
            parse_uncompressed_frame_header(&builder.finish(), &mut HeaderParserState::new()),
            Err(ParserError::InvalidBitstream)
        );
    }

    fn parse_key_frame_with_quant(base_q_idx: u8, deltas: [i32; 3]) -> UncompressedFrameHeader {
        let frame = key_frame_with_quant(base_q_idx, deltas);
        parse_uncompressed_frame_header(&frame, &mut HeaderParserState::new()).unwrap()
    }

    fn key_frame_with_quant(base_q_idx: u8, deltas: [i32; 3]) -> [u8; 128] {
        let mut builder = HeaderBuilder::new();
        builder.f(0b10, 2); // frame marker
        builder.f(0, 1); // profile low
        builder.f(0, 1); // profile high
        builder.f(0, 1); // not show existing frame
        builder.f(0, 1); // key frame
        builder.f(1, 1); // show frame
        builder.f(0, 1); // error resilient mode
        builder.f(0x49, 8);
        builder.f(0x83, 8);
        builder.f(0x42, 8);
        builder.f(1, 3); // BT.601 color space
        builder.f(0, 1); // studio range
        builder.f(319, 16);
        builder.f(239, 16);
        builder.f(0, 1); // render size matches frame size
        builder.f(1, 1); // refresh frame context
        builder.f(0, 1); // frame parallel decoding mode
        builder.f(3, 2); // frame context idx (reset to 0 for intra frames)
        builder.f(0, 6); // loop filter level
        builder.f(0, 3); // loop filter sharpness
        builder.f(0, 1); // loop filter delta enabled
        builder.quantization(base_q_idx, deltas);
        builder.f(0, 1); // segmentation disabled
        builder.f(0, 1); // tile rows log2
        builder.f(1, 16); // compressed header size
        builder.byte_align_zero();
        builder.byte(0);
        builder.finish()
    }

    fn parse_inter_frame_with_raw_interpolation(
        raw_interpolation_filter: u8,
    ) -> UncompressedFrameHeader {
        assert!(raw_interpolation_filter < 4);

        let mut state = HeaderParserState::new();
        state.reference_frames[0] = Some(ReferenceFrameInfo {
            width: 320,
            height: 240,
            render_width: 320,
            render_height: 240,
        });
        state.reference_frames[1] = state.reference_frames[0];
        state.reference_frames[2] = state.reference_frames[0];

        let mut builder = HeaderBuilder::new();
        builder.f(0b10, 2);
        builder.f(0, 1);
        builder.f(0, 1);
        builder.f(0, 1); // not show existing
        builder.f(1, 1); // non-key
        builder.f(1, 1); // show frame
        builder.f(0, 1); // not error resilient
        builder.f(0, 2); // reset frame context
        builder.f(0x01, 8); // refresh flags
        for idx in 0..3 {
            builder.f(idx, 3);
            builder.f(0, 1);
        }
        builder.f(1, 1); // found LAST ref size
        builder.f(0, 1); // render size matches
        builder.f(0, 1); // allow high precision mv
        builder.f(0, 1); // frame-level interpolation filter
        builder.f(u32::from(raw_interpolation_filter), 2);
        builder.f(0, 1); // refresh frame context
        builder.f(1, 1); // frame parallel decoding mode
        builder.f(1, 2); // frame context idx
        builder.f(0, 6);
        builder.f(0, 3);
        builder.f(0, 1);
        builder.quantization(0, [0, 0, 0]);
        builder.f(0, 1);
        builder.f(0, 1); // tile rows log2
        builder.f(1, 16);
        builder.byte_align_zero();
        builder.byte(0);
        let frame = builder.finish();

        parse_uncompressed_frame_header(&frame, &mut state).unwrap()
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

        fn quantization(&mut self, base_q_idx: u8, deltas: [i32; 3]) {
            self.f(u32::from(base_q_idx), 8);
            for delta in deltas {
                self.delta_q(delta);
            }
        }

        fn delta_q(&mut self, value: i32) {
            if value == 0 {
                self.f(0, 1);
                return;
            }

            let magnitude = value.unsigned_abs();
            assert!(magnitude <= 15);
            self.f(1, 1);
            self.f(magnitude, 4);
            self.f(u32::from(value < 0), 1);
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

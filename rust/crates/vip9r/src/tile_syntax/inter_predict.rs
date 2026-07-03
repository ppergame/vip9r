use super::*;

use core::arch::wasm32::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InterPredictionWrite {
    Store,
    Average,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UnscaledInterPrediction {
    pub(super) src_x: i32,
    pub(super) src_y: i32,
    pub(super) x_phase: usize,
    pub(super) y_phase: usize,
    pub(super) context: InterPredictionContext,
    pub(super) write: InterPredictionWrite,
}

#[inline(never)]
pub(super) fn inter_predict_unscaled_block(
    reference: ReferencePlane<'_>,
    scaled: ScaledMotion,
    interp_filter: usize,
    plane: &mut CurrentPlaneMut<'_>,
    context: InterPredictionContext,
    write: InterPredictionWrite,
    buffer: &mut InterpBuffer,
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
        return inter_predict_integer_unscaled_block(reference, plane, request, buffer);
    }

    let x_filter = SUBPEL_FILTERS
        .get(interp_filter)
        .and_then(|filters| filters.get(request.x_phase))
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let y_filter = SUBPEL_FILTERS
        .get(interp_filter)
        .and_then(|filters| filters.get(request.y_phase))
        .ok_or(TileSyntaxError::InvalidBitstream)?;

    inter_predict_subpel_unscaled_block(reference, plane, request, x_filter, y_filter, buffer)
}

#[inline(never)]
pub(super) fn inter_predict_integer_unscaled_block(
    reference: ReferencePlane<'_>,
    plane: &mut CurrentPlaneMut<'_>,
    request: UnscaledInterPrediction,
    buffer: &mut InterpBuffer,
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

    inter_predict_integer_edge_block(reference, plane, request, buffer)
}

#[inline(never)]
pub(super) fn inter_predict_integer_edge_block(
    reference: ReferencePlane<'_>,
    plane: &mut CurrentPlaneMut<'_>,
    request: UnscaledInterPrediction,
    buffer: &mut InterpBuffer,
) -> Result<(), TileSyntaxError> {
    let context = request.context;
    gather_clamped_reference_rect(
        reference,
        request.src_x,
        request.src_y,
        context.width,
        context.height,
        buffer,
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
pub(super) fn inter_predict_subpel_unscaled_block(
    reference: ReferencePlane<'_>,
    plane: &mut CurrentPlaneMut<'_>,
    request: UnscaledInterPrediction,
    x_filter: &[i16; INTERP_TAPS],
    y_filter: &[i16; INTERP_TAPS],
    buffer: &mut InterpBuffer,
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

    if reference_rect_inside(
        reference,
        source_left,
        source_top,
        source_width,
        source_height,
    )? {
        let left = usize::try_from(source_left).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let top = usize::try_from(source_top).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        horizontal_filter_reference_rect::<true>(
            reference,
            (left, top),
            (context.width, source_height),
            request.x_phase,
            x_filter,
            buffer,
        )?;
    } else {
        gather_clamped_reference_rect(
            reference,
            source_left,
            source_top,
            source_width,
            source_height,
            buffer,
        )?;
        horizontal_filter_buffer_in_place::<true>(
            context.width,
            source_height,
            request.x_phase,
            x_filter,
            buffer,
        )?;
    }

    write_vertical_filtered_block::<true>(plane, request, y_filter, buffer)
}

#[cfg(feature = "wasm-tests")]
#[inline(never)]
pub(super) fn inter_predict_subpel_unscaled_block_scalar(
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
        horizontal_filter_reference_rect::<false>(
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
        horizontal_filter_buffer_in_place::<false>(
            context.width,
            source_height,
            request.x_phase,
            x_filter,
            &mut buffer,
        )?;
    }

    write_vertical_filtered_block::<false>(plane, request, y_filter, &buffer)
}

pub(super) fn reference_rect_inside(
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

pub(super) fn gather_clamped_reference_rect(
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

pub(super) fn horizontal_filter_reference_rect<const USE_SIMD: bool>(
    reference: ReferencePlane<'_>,
    origin: (usize, usize),
    size: (usize, usize),
    x_phase: usize,
    filter: &[i16; INTERP_TAPS],
    buffer: &mut InterpBuffer,
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
        horizontal_filter_row_to_buffer::<USE_SIMD>(src, width, x_phase, filter, dst);
    }

    Ok(())
}

pub(super) fn horizontal_filter_buffer_in_place<const USE_SIMD: bool>(
    width: usize,
    height: usize,
    x_phase: usize,
    filter: &[i16; INTERP_TAPS],
    buffer: &mut InterpBuffer,
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
        horizontal_filter_row_in_place::<USE_SIMD>(source_row, width, x_phase, filter);
    }

    Ok(())
}

#[inline(always)]
pub(super) fn horizontal_filter_row_to_buffer<const USE_SIMD: bool>(
    src: &[u8],
    width: usize,
    x_phase: usize,
    filter: &[i16; INTERP_TAPS],
    dst: &mut [u8],
) {
    if x_phase == 0 {
        dst[..width].copy_from_slice(&src[3..3 + width]);
        return;
    }

    let scalar_start = if USE_SIMD {
        horizontal_filter_row_to_buffer_simd(src, width, filter, dst)
    } else {
        0
    };
    horizontal_filter_row_to_buffer_scalar_nonzero(src, filter, dst, scalar_start, width);
}

#[inline(always)]
pub(super) fn horizontal_filter_row_to_buffer_scalar_nonzero(
    src: &[u8],
    filter: &[i16; INTERP_TAPS],
    dst: &mut [u8],
    start: usize,
    end: usize,
) {
    let c0 = i32::from(filter[0]);
    let c1 = i32::from(filter[1]);
    let c2 = i32::from(filter[2]);
    let c3 = i32::from(filter[3]);
    let c4 = i32::from(filter[4]);
    let c5 = i32::from(filter[5]);
    let c6 = i32::from(filter[6]);
    let c7 = i32::from(filter[7]);

    for col in start..end {
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
pub(super) fn horizontal_filter_row_in_place<const USE_SIMD: bool>(
    row: &mut [u8],
    width: usize,
    x_phase: usize,
    filter: &[i16; INTERP_TAPS],
) {
    if x_phase == 0 {
        row.copy_within(3..3 + width, 0);
        return;
    }

    let scalar_start = if USE_SIMD {
        horizontal_filter_row_in_place_simd(row, width, filter)
    } else {
        0
    };
    horizontal_filter_row_in_place_scalar_nonzero(row, filter, scalar_start, width);
}

#[inline(always)]
pub(super) fn horizontal_filter_row_in_place_scalar_nonzero(
    row: &mut [u8],
    filter: &[i16; INTERP_TAPS],
    start: usize,
    end: usize,
) {
    let c0 = i32::from(filter[0]);
    let c1 = i32::from(filter[1]);
    let c2 = i32::from(filter[2]);
    let c3 = i32::from(filter[3]);
    let c4 = i32::from(filter[4]);
    let c5 = i32::from(filter[5]);
    let c6 = i32::from(filter[6]);
    let c7 = i32::from(filter[7]);

    for col in start..end {
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

pub(super) fn write_vertical_filtered_block<const USE_SIMD: bool>(
    plane: &mut CurrentPlaneMut<'_>,
    request: UnscaledInterPrediction,
    filter: &[i16; INTERP_TAPS],
    buffer: &InterpBuffer,
) -> Result<(), TileSyntaxError> {
    if USE_SIMD {
        return write_vertical_filtered_block_simd(plane, request, filter, buffer);
    }

    write_vertical_filtered_block_scalar(plane, request, filter, buffer)
}

pub(super) fn write_vertical_filtered_block_scalar(
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

        vertical_filter_row_scalar(
            buffer,
            row_start,
            (c0, c1, c2, c3, c4, c5, c6, c7),
            dst,
            request.write,
            0,
        );
    }

    Ok(())
}

#[inline(always)]
pub(super) fn vertical_filter_row_scalar(
    buffer: &[u8; MAX_INTERP_BUFFER],
    row_start: usize,
    filter: (i32, i32, i32, i32, i32, i32, i32, i32),
    dst: &mut [u8],
    write: InterPredictionWrite,
    start_col: usize,
) {
    let (c0, c1, c2, c3, c4, c5, c6, c7) = filter;
    match write {
        InterPredictionWrite::Store => {
            for (col, dst_sample) in dst.iter_mut().enumerate().skip(start_col) {
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
            for (col, dst_sample) in dst.iter_mut().enumerate().skip(start_col) {
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

#[derive(Clone, Copy)]
pub(super) struct WasmInterpCoefficients {
    pub(super) c0: v128,
    pub(super) c1: v128,
    pub(super) c2: v128,
    pub(super) c3: v128,
    pub(super) c4: v128,
    pub(super) c5: v128,
    pub(super) c6: v128,
    pub(super) c7: v128,
}

impl WasmInterpCoefficients {
    #[inline(always)]
    fn new(filter: &[i16; INTERP_TAPS]) -> Self {
        Self {
            c0: i16x8_splat(filter[0]),
            c1: i16x8_splat(filter[1]),
            c2: i16x8_splat(filter[2]),
            c3: i16x8_splat(filter[3]),
            c4: i16x8_splat(filter[4]),
            c5: i16x8_splat(filter[5]),
            c6: i16x8_splat(filter[6]),
            c7: i16x8_splat(filter[7]),
        }
    }
}

#[inline(always)]
pub(super) fn horizontal_filter_row_to_buffer_simd(
    src: &[u8],
    width: usize,
    filter: &[i16; INTERP_TAPS],
    dst: &mut [u8],
) -> usize {
    debug_assert!(src.len() >= width + INTERP_TAPS - 1);
    debug_assert!(dst.len() >= width);

    let coeffs = WasmInterpCoefficients::new(filter);
    let mut col = 0;
    while col + 8 <= width {
        let prediction = horizontal_filter_8(src.as_ptr().wrapping_add(col), coeffs);
        unsafe {
            v128_store64_lane::<0>(prediction, dst.as_mut_ptr().wrapping_add(col).cast::<u64>());
        }
        col += 8;
    }
    if col + 4 <= width {
        let prediction = horizontal_filter_4(src.as_ptr().wrapping_add(col), coeffs);
        unsafe {
            v128_store32_lane::<0>(prediction, dst.as_mut_ptr().wrapping_add(col).cast::<u32>());
        }
        col += 4;
    }
    col
}

#[inline(always)]
pub(super) fn horizontal_filter_row_in_place_simd(
    row: &mut [u8],
    width: usize,
    filter: &[i16; INTERP_TAPS],
) -> usize {
    debug_assert!(row.len() >= width + INTERP_TAPS - 1);

    let coeffs = WasmInterpCoefficients::new(filter);
    let src = row.as_ptr();
    let dst = row.as_mut_ptr();
    let mut col = 0;
    while col + 8 <= width {
        let prediction = horizontal_filter_8(src.wrapping_add(col), coeffs);
        unsafe {
            v128_store64_lane::<0>(prediction, dst.wrapping_add(col).cast::<u64>());
        }
        col += 8;
    }
    if col + 4 <= width {
        let prediction = horizontal_filter_4(src.wrapping_add(col), coeffs);
        unsafe {
            v128_store32_lane::<0>(prediction, dst.wrapping_add(col).cast::<u32>());
        }
        col += 4;
    }
    col
}

#[inline(always)]
pub(super) fn write_vertical_filtered_block_simd(
    plane: &mut CurrentPlaneMut<'_>,
    request: UnscaledInterPrediction,
    filter: &[i16; INTERP_TAPS],
    buffer: &InterpBuffer,
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
            write_inter_prediction_row_simd(
                plane,
                context.start_x,
                dst_y,
                prediction,
                request.write,
            )?;
        }
        return Ok(());
    }

    let coeffs = WasmInterpCoefficients::new(filter);
    let scalar_filter = (
        i32::from(filter[0]),
        i32::from(filter[1]),
        i32::from(filter[2]),
        i32::from(filter[3]),
        i32::from(filter[4]),
        i32::from(filter[5]),
        i32::from(filter[6]),
        i32::from(filter[7]),
    );

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

        let scalar_start = vertical_filter_row_simd(
            buffer.as_ptr().wrapping_add(row_start),
            coeffs,
            dst,
            request.write,
        );
        vertical_filter_row_scalar(
            buffer,
            row_start,
            scalar_filter,
            dst,
            request.write,
            scalar_start,
        );
    }

    Ok(())
}

#[inline(always)]
pub(super) fn vertical_filter_row_simd(
    src: *const u8,
    coeffs: WasmInterpCoefficients,
    dst: &mut [u8],
    write: InterPredictionWrite,
) -> usize {
    let mut col = 0;
    while col + 8 <= dst.len() {
        let prediction = vertical_filter_8(src.wrapping_add(col), coeffs);
        let dst_ptr = dst.as_mut_ptr().wrapping_add(col);
        match write {
            InterPredictionWrite::Store => unsafe {
                v128_store64_lane::<0>(prediction, dst_ptr.cast::<u64>());
            },
            InterPredictionWrite::Average => average_prediction_8(dst_ptr, prediction),
        }
        col += 8;
    }
    if col + 4 <= dst.len() {
        let prediction = vertical_filter_4(src.wrapping_add(col), coeffs);
        let dst_ptr = dst.as_mut_ptr().wrapping_add(col);
        match write {
            InterPredictionWrite::Store => unsafe {
                v128_store32_lane::<0>(prediction, dst_ptr.cast::<u32>());
            },
            InterPredictionWrite::Average => average_prediction_4(dst_ptr, prediction),
        }
        col += 4;
    }
    col
}

#[inline(always)]
pub(super) fn write_inter_prediction_row_simd(
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
        InterPredictionWrite::Average => average_prediction_row_simd(dst, prediction),
    }

    Ok(())
}

#[inline(always)]
pub(super) fn average_prediction_row_simd(dst: &mut [u8], prediction: &[u8]) {
    debug_assert!(prediction.len() >= dst.len());

    let mut col = 0;
    while col + 16 <= dst.len() {
        let dst_ptr = dst.as_mut_ptr().wrapping_add(col);
        let pred_ptr = prediction.as_ptr().wrapping_add(col);
        unsafe {
            let dst_values = v128_load(dst_ptr.cast::<v128>());
            let pred_values = v128_load(pred_ptr.cast::<v128>());
            v128_store(dst_ptr.cast::<v128>(), u8x16_avgr(dst_values, pred_values));
        }
        col += 16;
    }
    while col + 8 <= dst.len() {
        let prediction =
            unsafe { v128_load64_zero(prediction.as_ptr().wrapping_add(col).cast::<u64>()) };
        average_prediction_8(dst.as_mut_ptr().wrapping_add(col), prediction);
        col += 8;
    }
    if col + 4 <= dst.len() {
        let prediction =
            unsafe { v128_load32_zero(prediction.as_ptr().wrapping_add(col).cast::<u32>()) };
        average_prediction_4(dst.as_mut_ptr().wrapping_add(col), prediction);
        col += 4;
    }
    for (dst, &prediction) in dst[col..].iter_mut().zip(prediction[col..].iter()) {
        *dst = avg2(*dst, prediction);
    }
}

#[inline(always)]
pub(super) fn average_prediction_8(dst: *mut u8, prediction: v128) {
    unsafe {
        let dst_values = v128_load64_zero(dst.cast::<u64>());
        let average = u8x16_avgr(dst_values, prediction);
        v128_store64_lane::<0>(average, dst.cast::<u64>());
    }
}

#[inline(always)]
pub(super) fn average_prediction_4(dst: *mut u8, prediction: v128) {
    unsafe {
        let dst_values = v128_load32_zero(dst.cast::<u32>());
        let average = u8x16_avgr(dst_values, prediction);
        v128_store32_lane::<0>(average, dst.cast::<u32>());
    }
}

#[inline(always)]
pub(super) fn horizontal_filter_8(src: *const u8, coeffs: WasmInterpCoefficients) -> v128 {
    let mut lo = i32x4_splat(0);
    let mut hi = i32x4_splat(0);

    accumulate_u8x8(&mut lo, &mut hi, load_u8x8(src, 0), coeffs.c0);
    accumulate_u8x8(&mut lo, &mut hi, load_u8x8(src, 1), coeffs.c1);
    accumulate_u8x8(&mut lo, &mut hi, load_u8x8(src, 2), coeffs.c2);
    accumulate_u8x8(&mut lo, &mut hi, load_u8x8(src, 3), coeffs.c3);
    accumulate_u8x8(&mut lo, &mut hi, load_u8x8(src, 4), coeffs.c4);
    accumulate_u8x8(&mut lo, &mut hi, load_u8x8(src, 5), coeffs.c5);
    accumulate_u8x8(&mut lo, &mut hi, load_u8x8(src, 6), coeffs.c6);
    accumulate_u8x8(&mut lo, &mut hi, load_u8x8(src, 7), coeffs.c7);

    round_shift_pack_u8(lo, hi)
}

#[inline(always)]
pub(super) fn horizontal_filter_4(src: *const u8, coeffs: WasmInterpCoefficients) -> v128 {
    let mut sum = i32x4_splat(0);

    accumulate_u8x4(&mut sum, load_u8x4(src, 0), coeffs.c0);
    accumulate_u8x4(&mut sum, load_u8x4(src, 1), coeffs.c1);
    accumulate_u8x4(&mut sum, load_u8x4(src, 2), coeffs.c2);
    accumulate_u8x4(&mut sum, load_u8x4(src, 3), coeffs.c3);
    accumulate_u8x4(&mut sum, load_u8x4(src, 4), coeffs.c4);
    accumulate_u8x4(&mut sum, load_u8x4(src, 5), coeffs.c5);
    accumulate_u8x4(&mut sum, load_u8x4(src, 6), coeffs.c6);
    accumulate_u8x4(&mut sum, load_u8x4(src, 7), coeffs.c7);

    round_shift_pack_u8(sum, i32x4_splat(0))
}

#[inline(always)]
pub(super) fn vertical_filter_8(src: *const u8, coeffs: WasmInterpCoefficients) -> v128 {
    let mut lo = i32x4_splat(0);
    let mut hi = i32x4_splat(0);

    accumulate_u8x8(&mut lo, &mut hi, load_vertical_u8x8(src, 0), coeffs.c0);
    accumulate_u8x8(&mut lo, &mut hi, load_vertical_u8x8(src, 1), coeffs.c1);
    accumulate_u8x8(&mut lo, &mut hi, load_vertical_u8x8(src, 2), coeffs.c2);
    accumulate_u8x8(&mut lo, &mut hi, load_vertical_u8x8(src, 3), coeffs.c3);
    accumulate_u8x8(&mut lo, &mut hi, load_vertical_u8x8(src, 4), coeffs.c4);
    accumulate_u8x8(&mut lo, &mut hi, load_vertical_u8x8(src, 5), coeffs.c5);
    accumulate_u8x8(&mut lo, &mut hi, load_vertical_u8x8(src, 6), coeffs.c6);
    accumulate_u8x8(&mut lo, &mut hi, load_vertical_u8x8(src, 7), coeffs.c7);

    round_shift_pack_u8(lo, hi)
}

#[inline(always)]
pub(super) fn vertical_filter_4(src: *const u8, coeffs: WasmInterpCoefficients) -> v128 {
    let mut sum = i32x4_splat(0);

    accumulate_u8x4(&mut sum, load_vertical_u8x4(src, 0), coeffs.c0);
    accumulate_u8x4(&mut sum, load_vertical_u8x4(src, 1), coeffs.c1);
    accumulate_u8x4(&mut sum, load_vertical_u8x4(src, 2), coeffs.c2);
    accumulate_u8x4(&mut sum, load_vertical_u8x4(src, 3), coeffs.c3);
    accumulate_u8x4(&mut sum, load_vertical_u8x4(src, 4), coeffs.c4);
    accumulate_u8x4(&mut sum, load_vertical_u8x4(src, 5), coeffs.c5);
    accumulate_u8x4(&mut sum, load_vertical_u8x4(src, 6), coeffs.c6);
    accumulate_u8x4(&mut sum, load_vertical_u8x4(src, 7), coeffs.c7);

    round_shift_pack_u8(sum, i32x4_splat(0))
}

#[inline(always)]
pub(super) fn load_u8x8(src: *const u8, offset: usize) -> v128 {
    unsafe { v128_load64_zero(src.wrapping_add(offset).cast::<u64>()) }
}

#[inline(always)]
pub(super) fn load_u8x4(src: *const u8, offset: usize) -> v128 {
    unsafe { v128_load32_zero(src.wrapping_add(offset).cast::<u32>()) }
}

#[inline(always)]
pub(super) fn load_vertical_u8x8(src: *const u8, tap: usize) -> v128 {
    load_u8x8(src, tap * MAX_INTERP_SOURCE_DIM)
}

#[inline(always)]
pub(super) fn load_vertical_u8x4(src: *const u8, tap: usize) -> v128 {
    load_u8x4(src, tap * MAX_INTERP_SOURCE_DIM)
}

#[inline(always)]
pub(super) fn accumulate_u8x8(lo: &mut v128, hi: &mut v128, samples: v128, coeff: v128) {
    let samples = i16x8_extend_low_u8x16(samples);
    *lo = i32x4_add(*lo, i32x4_extmul_low_i16x8(samples, coeff));
    *hi = i32x4_add(*hi, i32x4_extmul_high_i16x8(samples, coeff));
}

#[inline(always)]
pub(super) fn accumulate_u8x4(sum: &mut v128, samples: v128, coeff: v128) {
    let samples = i16x8_extend_low_u8x16(samples);
    *sum = i32x4_add(*sum, i32x4_extmul_low_i16x8(samples, coeff));
}

#[inline(always)]
pub(super) fn round_shift_pack_u8(lo: v128, hi: v128) -> v128 {
    let rounding = i32x4_splat(1 << 6);
    let lo = i32x4_shr(i32x4_add(lo, rounding), 7);
    let hi = i32x4_shr(i32x4_add(hi, rounding), 7);
    let packed_i16 = i16x8_narrow_i32x4(lo, hi);
    u8x16_narrow_i16x8(packed_i16, i16x8_splat(0))
}

pub(super) fn write_inter_prediction_row(
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

pub(super) fn inter_prediction_row_mut<'a>(
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

pub(super) fn inter_predict_sample(
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

pub(super) fn horizontal_intermediate_sample(
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

pub(super) fn subpel_filter(
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

pub(super) const SUBPEL_FILTERS: [[[i16; 8]; 16]; 4] = [
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

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn simd_subpel_convolution_matches_scalar_reference() -> Result<(), TileSyntaxError> {
        const REFERENCE_STRIDE: usize = 96;
        const REFERENCE_HEIGHT: usize = 96;
        const PLANE_STRIDE: usize = 80;
        const PLANE_HEIGHT: usize = 80;
        const WIDTHS: [usize; 5] = [4, 8, 16, 32, 64];
        const HEIGHTS: [usize; 5] = [4, 8, 16, 32, 64];
        const FILTER_BANKS: [usize; 3] = [0, 1, 2];
        const SOURCE_ORIGINS: [(i32, i32); 3] = [(11, 9), (1, 2), (91, 92)];

        let mut seed = 0x1234_5678u32;
        let mut reference_data = [0u8; REFERENCE_STRIDE * REFERENCE_HEIGHT];
        let mut initial_plane = [0u8; PLANE_STRIDE * PLANE_HEIGHT];
        fill_pseudorandom(&mut reference_data, &mut seed);
        fill_pseudorandom(&mut initial_plane, &mut seed);

        let reference = ReferencePlane {
            data: &reference_data,
            width: REFERENCE_STRIDE,
            height: REFERENCE_HEIGHT,
            stride: REFERENCE_STRIDE,
        };
        let block = test_block(false);

        for filter_index in FILTER_BANKS {
            let bank = &SUBPEL_FILTERS[filter_index];
            for (x_phase, x_filter) in bank.iter().enumerate() {
                for (y_phase, y_filter) in bank.iter().enumerate() {
                    for (origin_index, (src_x, src_y)) in SOURCE_ORIGINS.into_iter().enumerate() {
                        for width in WIDTHS {
                            for height in HEIGHTS {
                                for write in
                                    [InterPredictionWrite::Store, InterPredictionWrite::Average]
                                {
                                    let context = InterPredictionContext {
                                        plane: 0,
                                        mi_row: 0,
                                        mi_col: 0,
                                        start_x: 5,
                                        start_y: 7,
                                        width,
                                        height,
                                        block_idx: 0,
                                        mi_size: BlockSize::Block8x8,
                                        block,
                                    };
                                    let request = UnscaledInterPrediction {
                                        src_x,
                                        src_y,
                                        x_phase,
                                        y_phase,
                                        context,
                                        write,
                                    };
                                    let mut scalar_data = initial_plane;
                                    let mut simd_data = initial_plane;
                                    let mut simd_buffer = [0; MAX_INTERP_BUFFER];
                                    let mut scalar_plane = CurrentPlaneMut {
                                        data: &mut scalar_data,
                                        width: PLANE_STRIDE,
                                        height: PLANE_HEIGHT,
                                        stride: PLANE_STRIDE,
                                    };
                                    let mut simd_plane = CurrentPlaneMut {
                                        data: &mut simd_data,
                                        width: PLANE_STRIDE,
                                        height: PLANE_HEIGHT,
                                        stride: PLANE_STRIDE,
                                    };

                                    inter_predict_subpel_unscaled_block_scalar(
                                        reference,
                                        &mut scalar_plane,
                                        request,
                                        x_filter,
                                        y_filter,
                                    )?;
                                    inter_predict_subpel_unscaled_block(
                                        reference,
                                        &mut simd_plane,
                                        request,
                                        x_filter,
                                        y_filter,
                                        &mut simd_buffer,
                                    )?;

                                    if scalar_data != simd_data {
                                        let mismatch = scalar_data
                                            .iter()
                                            .zip(simd_data.iter())
                                            .position(|(scalar, simd)| scalar != simd)
                                            .expect("mismatch exists");
                                        panic!(
                                            "SIMD subpel mismatch: filter={filter_index} \
                                             x_phase={x_phase} y_phase={y_phase} width={width} \
                                             height={height} write={write:?} \
                                             origin={origin_index} index={mismatch} \
                                             scalar={} simd={}",
                                            scalar_data[mismatch], simd_data[mismatch]
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
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
            let mut parser = TileParser {
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
                intra: IntraPredictionBuffers::new(),
                residual: ResidualBuffers::new(),
                interp_buffer: [0; MAX_INTERP_BUFFER],
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
}

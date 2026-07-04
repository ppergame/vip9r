use super::*;

use core::arch::wasm32::*;

pub(super) fn intra_prediction_edges(
    plane: &CurrentPlaneMut<'_>,
    context: IntraPredictionContext,
    edges: &mut IntraPredictionEdges,
) -> Result<(), TileSyntaxError> {
    let size = transform_width(context.tx_size);
    let above_len = size
        .checked_mul(2)
        .ok_or(TileSyntaxError::InvalidBitstream)?;

    if context.have_above {
        let above_y = context
            .start_y
            .checked_sub(1)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        if !store_intra_edge_plane_row(
            plane,
            context.start_x,
            above_y,
            size,
            &mut edges.above_row[..size],
        )? {
            for i in 0..size {
                edges.above_row[i] = plane.sample_clamped(
                    context
                        .start_x
                        .checked_add(i)
                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                    above_y,
                )?;
            }
        }

        if context.not_on_right && context.tx_size == TxSize::Tx4x4 {
            for i in size..above_len {
                edges.above_row[i] = plane.sample_clamped(
                    context
                        .start_x
                        .checked_add(i)
                        .ok_or(TileSyntaxError::InvalidBitstream)?,
                    above_y,
                )?;
            }
        } else {
            let edge = edges.above_row[size - 1];
            fill_intra_edge_span(&mut edges.above_row[size..above_len], edge, size);
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
    } else {
        edges.above_left = 127;
        fill_intra_edge_span(&mut edges.above_row[..above_len], 127, above_len);
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
    } else {
        fill_intra_edge_span(&mut edges.left_col[..size], 129, size);
    }

    Ok(())
}

pub(super) fn store_intra_edge_plane_row(
    plane: &CurrentPlaneMut<'_>,
    start_x: usize,
    y: usize,
    len: usize,
    dst: &mut [u8],
) -> Result<bool, TileSyntaxError> {
    debug_assert_eq!(dst.len(), len);

    let Some(end_x) = start_x.checked_add(len) else {
        return Ok(false);
    };
    if plane.width == 0 || plane.height == 0 || y >= plane.height || end_x > plane.width {
        return Ok(false);
    }
    plane.check_band_span(start_x, len)?;

    let start = y
        .checked_mul(plane.stride)
        .and_then(|row| row.checked_add(start_x))
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let end = start
        .checked_add(len)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let src = plane
        .data
        .get(start..end)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    store_intra_edge_span(dst, src, len);
    Ok(true)
}

#[inline(always)]
pub(super) fn fill_intra_edge_span(dst: &mut [u8], value: u8, len: usize) {
    debug_assert_eq!(dst.len(), len);

    unsafe {
        let value = u8x16_splat(value);
        match len {
            4 => v128_store32_lane::<0>(value, dst.as_mut_ptr().cast::<u32>()),
            8 => v128_store64_lane::<0>(value, dst.as_mut_ptr().cast::<u64>()),
            16 => v128_store(dst.as_mut_ptr().cast::<v128>(), value),
            32 => {
                v128_store(dst.as_mut_ptr().cast::<v128>(), value);
                v128_store(dst.as_mut_ptr().add(16).cast::<v128>(), value);
            }
            64 => {
                v128_store(dst.as_mut_ptr().cast::<v128>(), value);
                v128_store(dst.as_mut_ptr().add(16).cast::<v128>(), value);
                v128_store(dst.as_mut_ptr().add(32).cast::<v128>(), value);
                v128_store(dst.as_mut_ptr().add(48).cast::<v128>(), value);
            }
            _ => unreachable!("invalid intra edge span width"),
        }
    }
}

#[inline(always)]
pub(super) fn store_intra_edge_span(dst: &mut [u8], src: &[u8], len: usize) {
    debug_assert_eq!(dst.len(), len);
    debug_assert!(src.len() >= len);

    unsafe {
        match len {
            4 => {
                let value = v128_load32_zero(src.as_ptr().cast::<u32>());
                v128_store32_lane::<0>(value, dst.as_mut_ptr().cast::<u32>());
            }
            8 => {
                let value = v128_load64_zero(src.as_ptr().cast::<u64>());
                v128_store64_lane::<0>(value, dst.as_mut_ptr().cast::<u64>());
            }
            16 => {
                let value = v128_load(src.as_ptr().cast::<v128>());
                v128_store(dst.as_mut_ptr().cast::<v128>(), value);
            }
            32 => {
                let lo = v128_load(src.as_ptr().cast::<v128>());
                let hi = v128_load(src.as_ptr().add(16).cast::<v128>());
                v128_store(dst.as_mut_ptr().cast::<v128>(), lo);
                v128_store(dst.as_mut_ptr().add(16).cast::<v128>(), hi);
            }
            64 => {
                let a = v128_load(src.as_ptr().cast::<v128>());
                let b = v128_load(src.as_ptr().add(16).cast::<v128>());
                let c = v128_load(src.as_ptr().add(32).cast::<v128>());
                let d = v128_load(src.as_ptr().add(48).cast::<v128>());
                v128_store(dst.as_mut_ptr().cast::<v128>(), a);
                v128_store(dst.as_mut_ptr().add(16).cast::<v128>(), b);
                v128_store(dst.as_mut_ptr().add(32).cast::<v128>(), c);
                v128_store(dst.as_mut_ptr().add(48).cast::<v128>(), d);
            }
            _ => unreachable!("invalid intra edge span width"),
        }
    }
}

pub(super) fn intra_predict_block(
    request: IntraPredictionRequest,
    edges: &IntraPredictionEdges,
    pred: &mut [u8; MAX_TX_COEFFS],
) -> Result<(), TileSyntaxError> {
    validate_intra_prediction_size(request.size)?;

    match request.mode {
        IntraMode::Dc => {
            let value = dc_prediction_value(request, edges);
            fill_prediction_block(pred, request.size, value)?;
        }
        IntraMode::V => {
            for row in 0..request.size {
                let dst = prediction_buffer_row_mut(pred, row, request.size)?;
                store_prediction_row(dst, &edges.above_row, request.size);
            }
        }
        IntraMode::H => {
            for row in 0..request.size {
                let dst = prediction_buffer_row_mut(pred, row, request.size)?;
                fill_prediction_row(dst, edges.left_col[row], request.size);
            }
        }
        IntraMode::Tm => {
            for row in 0..request.size {
                let dst = prediction_buffer_row_mut(pred, row, request.size)?;
                true_motion_prediction_row(
                    dst,
                    &edges.above_row,
                    edges.left_col[row],
                    edges.above_left,
                    request.size,
                );
            }
        }
        _ => intra_predict_block_scalar(request, edges, pred)?,
    }

    Ok(())
}

pub(super) fn validate_intra_prediction_size(size: usize) -> Result<(), TileSyntaxError> {
    if matches!(size, 4 | 8 | 16 | 32) {
        Ok(())
    } else {
        Err(TileSyntaxError::InvalidBitstream)
    }
}

pub(super) fn write_common_intra_prediction_direct(
    plane: &mut CurrentPlaneMut<'_>,
    start_x: usize,
    start_y: usize,
    request: IntraPredictionRequest,
    edges: &IntraPredictionEdges,
) -> Result<bool, TileSyntaxError> {
    if !matches!(
        request.mode,
        IntraMode::Dc | IntraMode::V | IntraMode::H | IntraMode::Tm
    ) {
        return Ok(false);
    }

    let size = request.size;
    validate_intra_prediction_size(size)?;
    if !residual_block_inside(plane, (start_x, start_y), size) {
        return Ok(false);
    }

    match request.mode {
        IntraMode::Dc => {
            let value = dc_prediction_value(request, edges);
            for row in 0..size {
                let dst = direct_intra_prediction_row_mut(plane, start_x, start_y, row, size)?;
                fill_prediction_row(dst, value, size);
            }
        }
        IntraMode::V => {
            for row in 0..size {
                let dst = direct_intra_prediction_row_mut(plane, start_x, start_y, row, size)?;
                store_prediction_row(dst, &edges.above_row, size);
            }
        }
        IntraMode::H => {
            for row in 0..size {
                let dst = direct_intra_prediction_row_mut(plane, start_x, start_y, row, size)?;
                fill_prediction_row(dst, edges.left_col[row], size);
            }
        }
        IntraMode::Tm => {
            for row in 0..size {
                let dst = direct_intra_prediction_row_mut(plane, start_x, start_y, row, size)?;
                true_motion_prediction_row(
                    dst,
                    &edges.above_row,
                    edges.left_col[row],
                    edges.above_left,
                    size,
                );
            }
        }
        _ => unreachable!("mode was filtered above"),
    }

    Ok(true)
}

pub(super) fn direct_intra_prediction_row_mut<'a>(
    plane: &'a mut CurrentPlaneMut<'_>,
    start_x: usize,
    start_y: usize,
    row: usize,
    size: usize,
) -> Result<&'a mut [u8], TileSyntaxError> {
    let y = start_y
        .checked_add(row)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    intra_prediction_row_mut(plane, start_x, y, size)?.ok_or(TileSyntaxError::InvalidBitstream)
}

pub(super) fn intra_predict_block_scalar(
    request: IntraPredictionRequest,
    edges: &IntraPredictionEdges,
    pred: &mut [u8; MAX_TX_COEFFS],
) -> Result<(), TileSyntaxError> {
    let size = request.size;
    validate_intra_prediction_size(size)?;

    match request.mode {
        IntraMode::Dc => dc_predict_scalar(request, edges, pred),
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

pub(super) fn dc_predict_scalar(
    request: IntraPredictionRequest,
    edges: &IntraPredictionEdges,
    pred: &mut [u8; MAX_TX_COEFFS],
) {
    let size = request.size;
    let value = dc_prediction_value(request, edges);

    for row in 0..size {
        for col in 0..size {
            pred[row * size + col] = value;
        }
    }
}

pub(super) fn dc_prediction_value(
    request: IntraPredictionRequest,
    edges: &IntraPredictionEdges,
) -> u8 {
    let size = request.size;
    let log2_size = tx_width_log2(size);
    if request.have_left && request.have_above {
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
    }
}

pub(super) fn fill_prediction_block(
    pred: &mut [u8; MAX_TX_COEFFS],
    size: usize,
    value: u8,
) -> Result<(), TileSyntaxError> {
    for row in 0..size {
        let dst = prediction_buffer_row_mut(pred, row, size)?;
        fill_prediction_row(dst, value, size);
    }
    Ok(())
}

pub(super) fn prediction_buffer_row_mut(
    pred: &mut [u8; MAX_TX_COEFFS],
    row: usize,
    size: usize,
) -> Result<&mut [u8], TileSyntaxError> {
    let start = row
        .checked_mul(size)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let end = start
        .checked_add(size)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    pred.get_mut(start..end)
        .ok_or(TileSyntaxError::InvalidBitstream)
}

#[inline(always)]
pub(super) fn fill_prediction_row(dst: &mut [u8], value: u8, size: usize) {
    debug_assert_eq!(dst.len(), size);

    unsafe {
        let value = u8x16_splat(value);
        match size {
            4 => v128_store32_lane::<0>(value, dst.as_mut_ptr().cast::<u32>()),
            8 => v128_store64_lane::<0>(value, dst.as_mut_ptr().cast::<u64>()),
            16 => v128_store(dst.as_mut_ptr().cast::<v128>(), value),
            32 => {
                v128_store(dst.as_mut_ptr().cast::<v128>(), value);
                v128_store(dst.as_mut_ptr().add(16).cast::<v128>(), value);
            }
            _ => unreachable!("invalid intra prediction row width"),
        }
    }
}

#[inline(always)]
pub(super) fn true_motion_prediction_row(
    dst: &mut [u8],
    above: &[u8; MAX_INTRA_ABOVE],
    left: u8,
    above_left: u8,
    size: usize,
) {
    debug_assert_eq!(dst.len(), size);

    unsafe {
        let delta = i16x8_splat(i16::from(left) - i16::from(above_left));
        match size {
            4 => {
                let above = v128_load32_zero(above.as_ptr().cast::<u32>());
                let values = true_motion_prediction_8(above, delta);
                v128_store32_lane::<0>(values, dst.as_mut_ptr().cast::<u32>());
            }
            8 => {
                let above = v128_load64_zero(above.as_ptr().cast::<u64>());
                let values = true_motion_prediction_8(above, delta);
                v128_store64_lane::<0>(values, dst.as_mut_ptr().cast::<u64>());
            }
            16 => {
                let values = true_motion_prediction_16(above.as_ptr(), delta);
                v128_store(dst.as_mut_ptr().cast::<v128>(), values);
            }
            32 => {
                let lo = true_motion_prediction_16(above.as_ptr(), delta);
                let hi = true_motion_prediction_16(above.as_ptr().add(16), delta);
                v128_store(dst.as_mut_ptr().cast::<v128>(), lo);
                v128_store(dst.as_mut_ptr().add(16).cast::<v128>(), hi);
            }
            _ => unreachable!("invalid intra prediction row width"),
        }
    }
}

#[inline(always)]
pub(super) fn true_motion_prediction_8(above: v128, delta: v128) -> v128 {
    let above = i16x8_extend_low_u8x16(above);
    let values = i16x8_add(above, delta);
    u8x16_narrow_i16x8(values, i16x8_splat(0))
}

#[inline(always)]
pub(super) fn true_motion_prediction_16(above: *const u8, delta: v128) -> v128 {
    let lo = unsafe { v128_load64_zero(above.cast::<u64>()) };
    let hi = unsafe { v128_load64_zero(above.add(8).cast::<u64>()) };
    let lo = i16x8_add(i16x8_extend_low_u8x16(lo), delta);
    let hi = i16x8_add(i16x8_extend_low_u8x16(hi), delta);
    u8x16_narrow_i16x8(lo, hi)
}

pub(super) fn write_prediction_block(
    plane: &mut CurrentPlaneMut<'_>,
    start_x: usize,
    start_y: usize,
    size: usize,
    pred: &[u8; MAX_TX_COEFFS],
) -> Result<(), TileSyntaxError> {
    if matches!(size, 4 | 8 | 16 | 32) && residual_block_inside(plane, (start_x, start_y), size) {
        return write_prediction_block_interior(plane, start_x, start_y, size, pred);
    }

    write_prediction_block_scalar(plane, start_x, start_y, size, pred)
}

pub(super) fn write_prediction_block_scalar(
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

pub(super) fn write_prediction_block_interior(
    plane: &mut CurrentPlaneMut<'_>,
    start_x: usize,
    start_y: usize,
    size: usize,
    pred: &[u8; MAX_TX_COEFFS],
) -> Result<(), TileSyntaxError> {
    debug_assert!(residual_block_inside(plane, (start_x, start_y), size));
    debug_assert!(matches!(size, 4 | 8 | 16 | 32));

    for row in 0..size {
        let y = start_y
            .checked_add(row)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let dst = intra_prediction_row_mut(plane, start_x, y, size)?
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let pred_start = row
            .checked_mul(size)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let pred_end = pred_start
            .checked_add(size)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        let pred_row = pred
            .get(pred_start..pred_end)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        store_prediction_row(dst, pred_row, size);
    }

    Ok(())
}

pub(super) fn intra_prediction_row_mut<'a>(
    plane: &'a mut CurrentPlaneMut<'_>,
    x: usize,
    y: usize,
    width: usize,
) -> Result<Option<&'a mut [u8]>, TileSyntaxError> {
    if x >= plane.width || y >= plane.height {
        return Ok(None);
    }
    let visible_width = core::cmp::min(width, plane.width - x);
    plane.check_band_span(x, visible_width)?;
    let start = y
        .checked_mul(plane.stride)
        .and_then(|row| row.checked_add(x))
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    let end = start
        .checked_add(visible_width)
        .ok_or(TileSyntaxError::InvalidBitstream)?;
    plane
        .data
        .get_mut(start..end)
        .map(Some)
        .ok_or(TileSyntaxError::InvalidBitstream)
}

#[inline(always)]
pub(super) fn store_prediction_row(dst: &mut [u8], src: &[u8], size: usize) {
    debug_assert_eq!(dst.len(), size);
    debug_assert!(src.len() >= size);

    unsafe {
        match size {
            4 => {
                let value = v128_load32_zero(src.as_ptr().cast::<u32>());
                v128_store32_lane::<0>(value, dst.as_mut_ptr().cast::<u32>());
            }
            8 => {
                let value = v128_load64_zero(src.as_ptr().cast::<u64>());
                v128_store64_lane::<0>(value, dst.as_mut_ptr().cast::<u64>());
            }
            16 => {
                let value = v128_load(src.as_ptr().cast::<v128>());
                v128_store(dst.as_mut_ptr().cast::<v128>(), value);
            }
            32 => {
                let lo = v128_load(src.as_ptr().cast::<v128>());
                let hi = v128_load(src.as_ptr().add(16).cast::<v128>());
                v128_store(dst.as_mut_ptr().cast::<v128>(), lo);
                v128_store(dst.as_mut_ptr().add(16).cast::<v128>(), hi);
            }
            _ => unreachable!("invalid intra prediction row width"),
        }
    }
}

pub(super) fn above_with_left(edges: &IntraPredictionEdges, index: isize) -> u8 {
    if index < 0 {
        edges.above_left
    } else {
        edges.above_row[index as usize]
    }
}

pub(super) fn avg3(a: u8, b: u8, c: u8) -> u8 {
    ((u16::from(a) + 2 * u16::from(b) + u16::from(c) + 2) >> 2) as u8
}

pub(super) fn avg3_last_weighted(a: u8, b: u8) -> u8 {
    ((u16::from(a) + 3 * u16::from(b) + 2) >> 2) as u8
}

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::super::test_support::*;
    use super::*;

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
    fn optimized_intra_prediction_matches_scalar_sweep() {
        const MODES: [IntraMode; 10] = [
            IntraMode::Dc,
            IntraMode::V,
            IntraMode::H,
            IntraMode::D45,
            IntraMode::D135,
            IntraMode::D117,
            IntraMode::D153,
            IntraMode::D207,
            IntraMode::D63,
            IntraMode::Tm,
        ];

        let mut seed = 0x1357_2468;
        for size in [4, 8, 16, 32] {
            for mode in MODES {
                for have_left in [false, true] {
                    for have_above in [false, true] {
                        let edges = random_prediction_edges(&mut seed);
                        let request = IntraPredictionRequest {
                            mode,
                            have_left,
                            have_above,
                            size,
                        };
                        let mut scalar = [0x5a; MAX_TX_COEFFS];
                        let mut optimized = [0xa5; MAX_TX_COEFFS];

                        intra_predict_block_scalar(request, &edges, &mut scalar).unwrap();
                        intra_predict_block(request, &edges, &mut optimized).unwrap();

                        assert_eq!(
                            &optimized[..size * size],
                            &scalar[..size * size],
                            "mode={mode:?} size={size} have_left={have_left} \
                             have_above={have_above}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn direct_common_intra_prediction_matches_scalar_write() {
        const MODES: [IntraMode; 4] = [IntraMode::Dc, IntraMode::V, IntraMode::H, IntraMode::Tm];
        const STRIDE: usize = 48;
        const HEIGHT: usize = 40;
        const START_X: usize = 3;
        const START_Y: usize = 2;

        let mut seed = 0x89ab_cdef;
        for size in [4, 8, 16, 32] {
            for mode in MODES {
                for have_left in [false, true] {
                    for have_above in [false, true] {
                        let edges = random_prediction_edges(&mut seed);
                        let request = IntraPredictionRequest {
                            mode,
                            have_left,
                            have_above,
                            size,
                        };
                        let mut scalar_data = [0x33; STRIDE * HEIGHT];
                        let mut direct_data = [0x33; STRIDE * HEIGHT];
                        let mut pred = [0; MAX_TX_COEFFS];

                        intra_predict_block_scalar(request, &edges, &mut pred).unwrap();
                        {
                            let mut scalar_plane = CurrentPlaneMut {
                                data: &mut scalar_data,
                                width: 40,
                                height: HEIGHT,
                                stride: STRIDE,
                                band_x_start: 0,
                                band_x_end: 40,
                                band_y_start: 0,
                                band_y_end: HEIGHT,
                            };
                            write_prediction_block_scalar(
                                &mut scalar_plane,
                                START_X,
                                START_Y,
                                size,
                                &pred,
                            )
                            .unwrap();
                        }
                        {
                            let mut direct_plane = CurrentPlaneMut {
                                data: &mut direct_data,
                                width: 40,
                                height: HEIGHT,
                                stride: STRIDE,
                                band_x_start: 0,
                                band_x_end: 40,
                                band_y_start: 0,
                                band_y_end: HEIGHT,
                            };
                            assert!(
                                write_common_intra_prediction_direct(
                                    &mut direct_plane,
                                    START_X,
                                    START_Y,
                                    request,
                                    &edges,
                                )
                                .unwrap()
                            );
                        }

                        assert_eq!(
                            direct_data, scalar_data,
                            "mode={mode:?} size={size} have_left={have_left} \
                             have_above={have_above}"
                        );
                    }
                }
            }
        }
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
}

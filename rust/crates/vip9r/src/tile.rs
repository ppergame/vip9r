use core::cmp::min;

use crate::error::ParserError;
use crate::header::UncompressedFrameHeader;

pub(crate) const MAX_TILE_COLS_LOG2: u8 = 6;
pub(crate) const MAX_TILE_ROWS_LOG2: u8 = 2;
pub(crate) const MAX_TILES: usize = (1usize << MAX_TILE_COLS_LOG2) * (1usize << MAX_TILE_ROWS_LOG2);

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TileDescriptor {
    pub(crate) payload_start: usize,
    pub(crate) payload_end: usize,
    pub(crate) tile_row: u8,
    pub(crate) tile_col: u8,
    pub(crate) mi_row_start: u32,
    pub(crate) mi_row_end: u32,
    pub(crate) mi_col_start: u32,
    pub(crate) mi_col_end: u32,
}

impl TileDescriptor {
    const EMPTY: Self = Self {
        payload_start: 0,
        payload_end: 0,
        tile_row: 0,
        tile_col: 0,
        mi_row_start: 0,
        mi_row_end: 0,
        mi_col_start: 0,
        mi_col_end: 0,
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TileLayout {
    tiles: [TileDescriptor; MAX_TILES],
    len: usize,
}

impl TileLayout {
    fn new() -> Self {
        Self {
            tiles: [TileDescriptor::EMPTY; MAX_TILES],
            len: 0,
        }
    }

    pub(crate) fn as_slice(&self) -> &[TileDescriptor] {
        &self.tiles[..self.len]
    }

    fn push(&mut self, descriptor: TileDescriptor) -> Result<(), ParserError> {
        if self.len >= self.tiles.len() {
            return Err(ParserError::InvalidBitstream);
        }

        self.tiles[self.len] = descriptor;
        self.len += 1;
        Ok(())
    }
}

pub(crate) fn parse_tile_layout(
    frame: &[u8],
    header: &UncompressedFrameHeader,
) -> Result<TileLayout, ParserError> {
    if header.show_existing_frame || header.header_size_in_bytes == 0 {
        return Err(ParserError::InvalidBitstream);
    }

    let expected_tile_data_offset = header
        .compressed_header_offset
        .checked_add(header.header_size_in_bytes)
        .ok_or(ParserError::InvalidBitstream)?;
    if expected_tile_data_offset != header.tile_data_offset || header.tile_data_offset > frame.len()
    {
        return Err(ParserError::InvalidBitstream);
    }

    if header.tile_cols_log2 > MAX_TILE_COLS_LOG2 || header.tile_rows_log2 > MAX_TILE_ROWS_LOG2 {
        return Err(ParserError::InvalidBitstream);
    }

    let mi_cols = mi_size(header.frame_width)?;
    let mi_rows = mi_size(header.frame_height)?;
    if mi_cols == 0 || mi_rows == 0 {
        return Err(ParserError::InvalidBitstream);
    }

    let tile_cols = 1u32
        .checked_shl(u32::from(header.tile_cols_log2))
        .ok_or(ParserError::InvalidBitstream)?;
    let tile_rows = 1u32
        .checked_shl(u32::from(header.tile_rows_log2))
        .ok_or(ParserError::InvalidBitstream)?;
    let tile_count = tile_cols
        .checked_mul(tile_rows)
        .and_then(|count| usize::try_from(count).ok())
        .ok_or(ParserError::InvalidBitstream)?;
    if tile_count == 0 || tile_count > MAX_TILES {
        return Err(ParserError::InvalidBitstream);
    }

    let mut layout = TileLayout::new();
    let mut cursor = header.tile_data_offset;

    for tile_row in 0..tile_rows {
        for tile_col in 0..tile_cols {
            let tile_index = layout.len;
            let is_last_tile = tile_index == tile_count - 1;
            let payload_start;
            let payload_end;

            if is_last_tile {
                payload_start = cursor;
                payload_end = frame.len();
            } else {
                let size_field_end = cursor.checked_add(4).ok_or(ParserError::InvalidBitstream)?;
                if size_field_end > frame.len() {
                    return Err(ParserError::InvalidBitstream);
                }

                // tile_size is specified as f(32); at this byte-aligned point
                // that is an MSB-first 32-bit value.
                let tile_size = read_be_u32(&frame[cursor..size_field_end])?;
                payload_start = size_field_end;
                payload_end = payload_start
                    .checked_add(tile_size)
                    .ok_or(ParserError::InvalidBitstream)?;
                if payload_end > frame.len() {
                    return Err(ParserError::InvalidBitstream);
                }
                cursor = payload_end;
            }

            let descriptor = TileDescriptor {
                payload_start,
                payload_end,
                tile_row: u8::try_from(tile_row).map_err(|_| ParserError::InvalidBitstream)?,
                tile_col: u8::try_from(tile_col).map_err(|_| ParserError::InvalidBitstream)?,
                mi_row_start: get_tile_offset(tile_row, mi_rows, header.tile_rows_log2)?,
                mi_row_end: get_tile_offset(tile_row + 1, mi_rows, header.tile_rows_log2)?,
                mi_col_start: get_tile_offset(tile_col, mi_cols, header.tile_cols_log2)?,
                mi_col_end: get_tile_offset(tile_col + 1, mi_cols, header.tile_cols_log2)?,
            };
            layout.push(descriptor)?;
        }
    }

    Ok(layout)
}

fn mi_size(pixels: u32) -> Result<u32, ParserError> {
    pixels
        .checked_add(7)
        .map(|value| value >> 3)
        .ok_or(ParserError::InvalidBitstream)
}

fn get_tile_offset(tile_num: u32, mis: u32, tile_sz_log2: u8) -> Result<u32, ParserError> {
    let sbs = (u64::from(mis) + 7) >> 3;
    let offset = u64::from(tile_num)
        .checked_mul(sbs)
        .map(|value| (value >> tile_sz_log2) << 3)
        .ok_or(ParserError::InvalidBitstream)?;
    u32::try_from(min(offset, u64::from(mis))).map_err(|_| ParserError::InvalidBitstream)
}

fn read_be_u32(bytes: &[u8]) -> Result<usize, ParserError> {
    let value = (u32::from(bytes[0]) << 24)
        | (u32::from(bytes[1]) << 16)
        | (u32::from(bytes[2]) << 8)
        | u32::from(bytes[3]);
    usize::try_from(value).map_err(|_| ParserError::InvalidBitstream)
}

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::{ParserError, TileDescriptor, parse_tile_layout};
    use crate::header::{FrameType, LoopFilterParams, UncompressedFrameHeader};

    #[test]
    fn one_tile_frame_uses_remaining_payload_without_size_prefix() {
        let frame = [0xaa, 0xbb, 1, 2, 3, 4];
        let header = test_header(64, 64, 0, 0, 2);

        let layout = parse_tile_layout(&frame, &header).unwrap();

        assert_eq!(
            layout.as_slice(),
            [TileDescriptor {
                payload_start: 2,
                payload_end: 6,
                tile_row: 0,
                tile_col: 0,
                mi_row_start: 0,
                mi_row_end: 8,
                mi_col_start: 0,
                mi_col_end: 8,
            }]
        );
    }

    #[test]
    fn multiple_tiles_consume_big_endian_non_final_size_prefixes() {
        let frame = [0xcc, 0, 0, 0, 2, 0xa0, 0xa1, 0xb0, 0xb1, 0xb2];
        let header = test_header(512, 64, 1, 0, 1);

        let layout = parse_tile_layout(&frame, &header).unwrap();

        assert_eq!(layout.as_slice().len(), 2);
        assert_eq!(layout.as_slice()[0].payload_start, 5);
        assert_eq!(layout.as_slice()[0].payload_end, 7);
        assert_eq!(layout.as_slice()[0].tile_col, 0);
        assert_eq!(layout.as_slice()[0].mi_col_start, 0);
        assert_eq!(layout.as_slice()[0].mi_col_end, 32);
        assert_eq!(layout.as_slice()[1].payload_start, 7);
        assert_eq!(layout.as_slice()[1].payload_end, 10);
        assert_eq!(layout.as_slice()[1].tile_col, 1);
        assert_eq!(layout.as_slice()[1].mi_col_start, 32);
        assert_eq!(layout.as_slice()[1].mi_col_end, 64);
    }

    #[test]
    fn truncated_non_final_tile_size_field_is_rejected() {
        let frame = [0xcc, 2, 0, 0];
        let header = test_header(512, 64, 1, 0, 1);

        assert_eq!(
            parse_tile_layout(&frame, &header),
            Err(ParserError::InvalidBitstream)
        );
    }

    #[test]
    fn tile_size_overrunning_remaining_payload_is_rejected() {
        let frame = [0xcc, 0, 0, 0, 3, 0xa0, 0xa1];
        let header = test_header(512, 64, 1, 0, 1);

        assert_eq!(
            parse_tile_layout(&frame, &header),
            Err(ParserError::InvalidBitstream)
        );
    }

    #[test]
    fn odd_dimensions_use_spec_mi_tile_bounds() {
        let frame = [
            0xcc, // compressed header byte
            0, 0, 0, 1, 0xa0, // row 0, col 0
            0, 0, 0, 2, 0xb0, 0xb1, // row 0, col 1
            0, 0, 0, 3, 0xc0, 0xc1, 0xc2, // row 1, col 0
            0xd0, 0xd1, 0xd2, 0xd3, // final tile: row 1, col 1
        ];
        let header = test_header(449, 65, 1, 1, 1);

        let layout = parse_tile_layout(&frame, &header).unwrap();

        let tiles = layout.as_slice();
        assert_eq!(tiles.len(), 4);
        assert_eq!((tiles[0].mi_row_start, tiles[0].mi_row_end), (0, 8));
        assert_eq!((tiles[0].mi_col_start, tiles[0].mi_col_end), (0, 32));
        assert_eq!((tiles[1].mi_row_start, tiles[1].mi_row_end), (0, 8));
        assert_eq!((tiles[1].mi_col_start, tiles[1].mi_col_end), (32, 57));
        assert_eq!((tiles[2].mi_row_start, tiles[2].mi_row_end), (8, 9));
        assert_eq!((tiles[2].mi_col_start, tiles[2].mi_col_end), (0, 32));
        assert_eq!((tiles[3].mi_row_start, tiles[3].mi_row_end), (8, 9));
        assert_eq!((tiles[3].mi_col_start, tiles[3].mi_col_end), (32, 57));
    }

    fn test_header(
        frame_width: u32,
        frame_height: u32,
        tile_cols_log2: u8,
        tile_rows_log2: u8,
        tile_data_offset: usize,
    ) -> UncompressedFrameHeader {
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
            frame_width,
            frame_height,
            render_width: frame_width,
            render_height: frame_height,
            base_q_idx: 0,
            delta_q_y_dc: 0,
            delta_q_uv_dc: 0,
            delta_q_uv_ac: 0,
            lossless: true,
            loop_filter: LoopFilterParams::disabled(),
            segmentation: crate::header::SegmentationParams::disabled(),
            segmentation_enabled: false,
            segmentation_update_map: false,
            tile_cols_log2,
            tile_rows_log2,
            header_size_in_bytes: tile_data_offset,
            compressed_header_offset: 0,
            tile_data_offset,
        }
    }
}

use crate::error::ParserError;
use crate::{CodedFrameRange, CodedFrameRanges, MAX_CODED_FRAMES_PER_PACKET};

pub(crate) fn split_packet(packet: &[u8]) -> Result<CodedFrameRanges, ParserError> {
    let Some(&marker) = packet.last() else {
        return Err(ParserError::InvalidBitstream);
    };

    if marker >> 5 != 0b110 {
        return Ok(single_frame(packet.len()));
    }

    let size_bytes = usize::from(((marker >> 3) & 0b11) + 1);
    let frame_count = usize::from((marker & 0b111) + 1);
    let index_len = 2 + frame_count * size_bytes;
    if index_len > packet.len() {
        return Err(ParserError::InvalidBitstream);
    }

    let index_start = packet.len() - index_len;
    if packet[index_start] != marker {
        return Err(ParserError::InvalidBitstream);
    }

    let payload_len = index_start;
    let mut frames = [CodedFrameRange { start: 0, len: 0 }; MAX_CODED_FRAMES_PER_PACKET];
    let mut frame_offset = 0usize;
    let mut size_offset = index_start + 1;

    for frame in frames.iter_mut().take(frame_count) {
        let frame_size = read_le_size(&packet[size_offset..size_offset + size_bytes])?;
        size_offset += size_bytes;

        let next_frame_offset = frame_offset
            .checked_add(frame_size)
            .ok_or(ParserError::InvalidBitstream)?;
        if next_frame_offset > payload_len {
            return Err(ParserError::InvalidBitstream);
        }

        *frame = CodedFrameRange {
            start: frame_offset,
            len: frame_size,
        };
        frame_offset = next_frame_offset;
    }

    if frame_offset != payload_len {
        return Err(ParserError::InvalidBitstream);
    }

    Ok(CodedFrameRanges {
        ranges: frames,
        len: frame_count,
    })
}

fn single_frame(len: usize) -> CodedFrameRanges {
    let mut frames = [CodedFrameRange { start: 0, len: 0 }; MAX_CODED_FRAMES_PER_PACKET];
    frames[0] = CodedFrameRange { start: 0, len };
    CodedFrameRanges {
        ranges: frames,
        len: 1,
    }
}

fn read_le_size(bytes: &[u8]) -> Result<usize, ParserError> {
    let mut size = 0u32;
    for (index, &byte) in bytes.iter().enumerate() {
        size |= u32::from(byte) << (index * 8);
    }
    usize::try_from(size).map_err(|_| ParserError::InvalidBitstream)
}

#[vip9r_wasm_test_macros::wasm_tests]
mod tests {
    use super::{ParserError, split_packet};
    use crate::CodedFrameRange;

    #[test]
    fn non_superframe_packet_is_one_frame() {
        let packet = [1, 2, 3];

        let frames = split_packet(&packet).unwrap();

        assert_eq!(frames.as_slice(), [CodedFrameRange { start: 0, len: 3 }]);
    }

    #[test]
    fn valid_superframe_uses_index_sizes() {
        let packet = [1, 2, 3, 4, 5, 0xc1, 2, 3, 0xc1];

        let frames = split_packet(&packet).unwrap();

        assert_eq!(
            frames.as_slice(),
            [
                CodedFrameRange { start: 0, len: 2 },
                CodedFrameRange { start: 2, len: 3 }
            ]
        );
    }

    #[test]
    fn truncated_superframe_index_is_rejected() {
        let packet = [1, 2, 0xc1];

        assert_eq!(split_packet(&packet), Err(ParserError::InvalidBitstream));
    }

    #[test]
    fn superframe_marker_mismatch_is_rejected() {
        let packet = [1, 2, 3, 4, 5, 0xc0, 2, 3, 0xc1];

        assert_eq!(split_packet(&packet), Err(ParserError::InvalidBitstream));
    }

    #[test]
    fn superframe_size_overrun_is_rejected() {
        let packet = [1, 2, 3, 4, 5, 0xc1, 4, 2, 0xc1];

        assert_eq!(split_packet(&packet), Err(ParserError::InvalidBitstream));
    }
}

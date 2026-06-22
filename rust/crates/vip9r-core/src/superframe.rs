use crate::error::ParserError;

pub(crate) const MAX_FRAMES_PER_SUPERFRAME: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameSlices<'a> {
    frames: [&'a [u8]; MAX_FRAMES_PER_SUPERFRAME],
    len: usize,
}

impl<'a> FrameSlices<'a> {
    pub(crate) fn as_slice(&self) -> &[&'a [u8]] {
        &self.frames[..self.len]
    }
}

pub(crate) fn split_superframe(packet: &[u8]) -> Result<FrameSlices<'_>, ParserError> {
    let Some(&marker) = packet.last() else {
        return Err(ParserError::InvalidBitstream);
    };

    if marker >> 5 != 0b110 {
        return Ok(single_frame(packet));
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
    let mut frames = [&[][..]; MAX_FRAMES_PER_SUPERFRAME];
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

        *frame = &packet[frame_offset..next_frame_offset];
        frame_offset = next_frame_offset;
    }

    if frame_offset != payload_len {
        return Err(ParserError::InvalidBitstream);
    }

    Ok(FrameSlices {
        frames,
        len: frame_count,
    })
}

fn single_frame(packet: &[u8]) -> FrameSlices<'_> {
    let mut frames = [&[][..]; MAX_FRAMES_PER_SUPERFRAME];
    frames[0] = packet;
    FrameSlices { frames, len: 1 }
}

fn read_le_size(bytes: &[u8]) -> Result<usize, ParserError> {
    let mut size = 0u32;
    for (index, &byte) in bytes.iter().enumerate() {
        size |= u32::from(byte) << (index * 8);
    }
    usize::try_from(size).map_err(|_| ParserError::InvalidBitstream)
}

#[cfg(test)]
mod tests {
    use super::{ParserError, split_superframe};

    #[test]
    fn non_superframe_packet_is_one_frame() {
        let packet = [1, 2, 3];

        let frames = split_superframe(&packet).unwrap();

        assert_eq!(frames.as_slice(), [&packet[..]]);
    }

    #[test]
    fn valid_superframe_uses_index_sizes() {
        let packet = [1, 2, 3, 4, 5, 0xc1, 2, 3, 0xc1];

        let frames = split_superframe(&packet).unwrap();

        assert_eq!(frames.as_slice(), [&packet[0..2], &packet[2..5]]);
    }

    #[test]
    fn truncated_superframe_index_is_rejected() {
        let packet = [1, 2, 0xc1];

        assert_eq!(
            split_superframe(&packet),
            Err(ParserError::InvalidBitstream)
        );
    }

    #[test]
    fn superframe_marker_mismatch_is_rejected() {
        let packet = [1, 2, 3, 4, 5, 0xc0, 2, 3, 0xc1];

        assert_eq!(
            split_superframe(&packet),
            Err(ParserError::InvalidBitstream)
        );
    }

    #[test]
    fn superframe_size_overrun_is_rejected() {
        let packet = [1, 2, 3, 4, 5, 0xc1, 4, 2, 0xc1];

        assert_eq!(
            split_superframe(&packet),
            Err(ParserError::InvalidBitstream)
        );
    }
}

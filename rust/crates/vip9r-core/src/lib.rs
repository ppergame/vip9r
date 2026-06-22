#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

use core::convert::Infallible;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecoderOptions {
    pub limits: DecoderLimits,
}

impl DecoderOptions {
    pub const fn new(limits: DecoderLimits) -> Self {
        Self { limits }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecoderLimits {
    pub max_width: u32,
    pub max_height: u32,
}

impl DecoderLimits {
    pub const fn new(max_width: u32, max_height: u32) -> Self {
        Self {
            max_width,
            max_height,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketReport {
    pub coded_frames: u32,
    pub shown_frames: u32,
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
pub struct Plane<'a> {
    pub data: &'a [u8],
    pub stride: usize,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct I420Frame<'a> {
    pub info: FrameInfo,
    pub y: Plane<'a>,
    pub u: Plane<'a>,
    pub v: Plane<'a>,
}

impl I420Frame<'_> {
    pub fn write_compact(&self, output: &mut [u8]) -> Result<usize, FrameCopyError> {
        let byte_len = self.info.i420_len().ok_or(FrameCopyError::InvalidPlane)?;
        if output.len() < byte_len {
            return Err(FrameCopyError::OutputTooSmall { required: byte_len });
        }
        self.validate_layout()?;

        let mut written = 0;
        written += copy_plane(&mut output[written..], self.y)?;
        written += copy_plane(&mut output[written..], self.u)?;
        written += copy_plane(&mut output[written..], self.v)?;
        Ok(written)
    }

    fn validate_layout(&self) -> Result<(), FrameCopyError> {
        let (chroma_width, chroma_height) =
            chroma_dimensions(self.info.visible_width, self.info.visible_height);
        if self.y.width != self.info.visible_width
            || self.y.height != self.info.visible_height
            || self.u.width != chroma_width
            || self.u.height != chroma_height
            || self.v.width != chroma_width
            || self.v.height != chroma_height
        {
            return Err(FrameCopyError::InvalidPlane);
        }
        Ok(())
    }
}

pub trait FrameSink {
    type Error;

    /// Receives one shown frame.
    ///
    /// `frame` is only valid for this call. The decoder may reuse or reset the
    /// storage backing it as soon as this method returns.
    fn frame(&mut self, frame: I420Frame<'_>) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameCopyError {
    OutputTooSmall { required: usize },
    InvalidPlane,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError<SinkError = Infallible> {
    InvalidConfig,
    OutputTooSmall { required: usize },
    ResourceLimit,
    UnsupportedProfile(u8),
    UnsupportedBitDepth(u8),
    InvalidBitstream,
    Sink(SinkError),
    Unimplemented,
}

impl<SinkError> DecodeError<SinkError> {
    pub const fn code(&self) -> i32 {
        match self {
            Self::InvalidConfig => -1,
            Self::OutputTooSmall { .. } => -2,
            Self::ResourceLimit => -3,
            Self::UnsupportedProfile(_) => -4,
            Self::UnsupportedBitDepth(_) => -5,
            Self::InvalidBitstream => -6,
            Self::Sink(_) => -7,
            Self::Unimplemented => -8,
        }
    }
}

#[derive(Debug)]
pub struct Decoder {
    options: DecoderOptions,
}

impl Decoder {
    pub fn new(options: DecoderOptions) -> Result<Self, DecodeError> {
        if options.limits.max_width == 0
            || options.limits.max_height == 0
            || required_i420_len(options.limits.max_width, options.limits.max_height).is_none()
        {
            return Err(DecodeError::InvalidConfig);
        }

        Ok(Self { options })
    }

    pub const fn options(&self) -> DecoderOptions {
        self.options
    }

    /// Decodes one demuxed VP9 packet.
    ///
    /// The packet is an IVF frame payload or WebM block payload. VP9 superframe
    /// splitting belongs inside this call. Shown frames are emitted
    /// synchronously through `sink`; no output frame may borrow decoder storage
    /// after the sink callback returns.
    pub fn decode_packet<Sink: FrameSink>(
        &mut self,
        _packet: &[u8],
        _sink: &mut Sink,
    ) -> Result<PacketReport, DecodeError<Sink::Error>> {
        Err(DecodeError::Unimplemented)
    }

    pub fn reset(&mut self) {}
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

fn chroma_dimensions(width: u32, height: u32) -> (u32, u32) {
    (width / 2 + width % 2, height / 2 + height % 2)
}

fn copy_plane(output: &mut [u8], plane: Plane<'_>) -> Result<usize, FrameCopyError> {
    let width = usize::try_from(plane.width).map_err(|_| FrameCopyError::InvalidPlane)?;
    let height = usize::try_from(plane.height).map_err(|_| FrameCopyError::InvalidPlane)?;
    if width == 0 || height == 0 || plane.stride < width {
        return Err(FrameCopyError::InvalidPlane);
    }

    let last_row = plane
        .stride
        .checked_mul(height - 1)
        .ok_or(FrameCopyError::InvalidPlane)?;
    let required_input = last_row
        .checked_add(width)
        .ok_or(FrameCopyError::InvalidPlane)?;
    if plane.data.len() < required_input {
        return Err(FrameCopyError::InvalidPlane);
    }

    let required_output = width
        .checked_mul(height)
        .ok_or(FrameCopyError::InvalidPlane)?;
    if output.len() < required_output {
        return Err(FrameCopyError::OutputTooSmall {
            required: required_output,
        });
    }

    for row in 0..height {
        let input_start = plane
            .stride
            .checked_mul(row)
            .ok_or(FrameCopyError::InvalidPlane)?;
        let input_end = input_start + width;
        let output_start = width.checked_mul(row).ok_or(FrameCopyError::InvalidPlane)?;
        let output_end = output_start + width;
        output[output_start..output_end].copy_from_slice(&plane.data[input_start..input_end]);
    }

    Ok(required_output)
}

#[cfg(test)]
mod tests {
    use super::{
        Decoder, DecoderLimits, DecoderOptions, FrameInfo, I420Frame, Plane, required_i420_len,
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
    fn decoder_rejects_zero_limits() {
        assert_eq!(
            Decoder::new(DecoderOptions::new(DecoderLimits::new(0, 720))).unwrap_err(),
            super::DecodeError::InvalidConfig
        );
    }

    #[test]
    fn compact_i420_write_strips_stride_and_orders_planes() {
        let info = FrameInfo::i420(3, 3, 3, 3, 0).unwrap();
        let y = [
            1, 2, 3, 99, //
            4, 5, 6, 99, //
            7, 8, 9, 99,
        ];
        let u = [
            10, 11, 99, //
            12, 13, 99,
        ];
        let v = [
            14, 15, 99, //
            16, 17, 99,
        ];
        let frame = I420Frame {
            info,
            y: Plane {
                data: &y,
                stride: 4,
                width: 3,
                height: 3,
            },
            u: Plane {
                data: &u,
                stride: 3,
                width: 2,
                height: 2,
            },
            v: Plane {
                data: &v,
                stride: 3,
                width: 2,
                height: 2,
            },
        };

        let mut output = [0; 17];
        assert_eq!(frame.write_compact(&mut output), Ok(17));
        assert_eq!(
            output,
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]
        );
    }

    #[test]
    fn compact_i420_write_reports_whole_frame_len_when_output_is_too_small() {
        let frame = tiny_i420_frame();
        let mut output = [0; 5];

        assert_eq!(
            frame.write_compact(&mut output),
            Err(super::FrameCopyError::OutputTooSmall { required: 6 })
        );
    }

    #[test]
    fn compact_i420_write_rejects_plane_dimension_mismatch() {
        let mut frame = tiny_i420_frame();
        frame.u.width = 2;
        let mut output = [0; 6];

        assert_eq!(
            frame.write_compact(&mut output),
            Err(super::FrameCopyError::InvalidPlane)
        );
    }

    fn tiny_i420_frame() -> I420Frame<'static> {
        I420Frame {
            info: FrameInfo::i420(2, 2, 2, 2, 0).unwrap(),
            y: Plane {
                data: &[1, 2, 3, 4],
                stride: 2,
                width: 2,
                height: 2,
            },
            u: Plane {
                data: &[5],
                stride: 1,
                width: 1,
                height: 1,
            },
            v: Plane {
                data: &[6],
                stride: 1,
                width: 1,
                height: 1,
            },
        }
    }
}

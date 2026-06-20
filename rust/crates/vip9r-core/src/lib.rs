#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecoderConfig {
    pub width: u32,
    pub height: u32,
}

impl DecoderConfig {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn yuv420_len(self) -> Option<usize> {
        let width = self.width as usize;
        let height = self.height as usize;
        let luma = width.checked_mul(height)?;
        let chroma_width = width.checked_add(1)? / 2;
        let chroma_height = height.checked_add(1)? / 2;
        let chroma_plane = chroma_width.checked_mul(chroma_height)?;
        luma.checked_add(chroma_plane.checked_mul(2)?)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeStatus {
    FrameReady { bytes_written: usize },
    NeedMoreInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    InvalidConfig,
    OutputTooSmall,
    Unimplemented,
}

impl DecodeError {
    pub const fn code(self) -> i32 {
        match self {
            Self::InvalidConfig => -1,
            Self::OutputTooSmall => -2,
            Self::Unimplemented => -3,
        }
    }
}

pub struct Decoder {
    config: DecoderConfig,
}

impl Decoder {
    pub const fn new(config: DecoderConfig) -> Self {
        Self { config }
    }

    pub const fn config(&self) -> DecoderConfig {
        self.config
    }

    pub fn decode_frame(
        &mut self,
        _input: &[u8],
        _output: &mut [u8],
    ) -> Result<DecodeStatus, DecodeError> {
        Err(DecodeError::Unimplemented)
    }
}

#[cfg(test)]
mod tests {
    use super::DecoderConfig;

    #[test]
    fn yuv420_len_counts_luma_and_two_quarter_chroma_planes() {
        assert_eq!(DecoderConfig::new(1280, 720).yuv420_len(), Some(1_382_400));
    }

    #[test]
    fn yuv420_len_rounds_chroma_planes_up_for_odd_sizes() {
        assert_eq!(DecoderConfig::new(3, 3).yuv420_len(), Some(17));
    }
}

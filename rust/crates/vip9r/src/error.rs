use crate::DecodeError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParserError {
    InvalidBitstream,
    UnsupportedProfile(u8),
    UnsupportedBitDepth(u8),
}

impl ParserError {
    pub(crate) const fn into_decode_error(self) -> DecodeError {
        match self {
            Self::InvalidBitstream => DecodeError::InvalidBitstream,
            Self::UnsupportedProfile(profile) => DecodeError::UnsupportedProfile(profile),
            Self::UnsupportedBitDepth(bit_depth) => DecodeError::UnsupportedBitDepth(bit_depth),
        }
    }
}

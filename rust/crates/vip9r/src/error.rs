use crate::DecodeError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParserError {
    InvalidBitstream,
    UnsupportedProfile(u8),
    UnsupportedBitDepth(u8),
}

impl From<ParserError> for DecodeError {
    fn from(error: ParserError) -> Self {
        match error {
            ParserError::InvalidBitstream => Self::InvalidBitstream,
            ParserError::UnsupportedProfile(profile) => Self::UnsupportedProfile(profile),
            ParserError::UnsupportedBitDepth(bit_depth) => Self::UnsupportedBitDepth(bit_depth),
        }
    }
}

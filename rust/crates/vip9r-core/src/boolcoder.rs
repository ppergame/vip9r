use crate::error::ParserError;

const MARKER_PROBABILITY: u8 = 128;
const LITERAL_PROBABILITY: u8 = 128;

#[allow(dead_code)]
pub(crate) struct BoolDecoder<'a> {
    data: &'a [u8],
    value: u16,
    range: u16,
    bit_offset: usize,
}

#[allow(dead_code)]
impl<'a> BoolDecoder<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Result<Self, ParserError> {
        let initial_value = data.first().ok_or(ParserError::InvalidBitstream)?;
        let mut decoder = Self {
            data,
            value: u16::from(*initial_value),
            range: 255,
            bit_offset: 0,
        };

        if decoder.read_bool(MARKER_PROBABILITY)? {
            return Err(ParserError::InvalidBitstream);
        }

        Ok(decoder)
    }

    pub(crate) fn read_bool(&mut self, probability: u8) -> Result<bool, ParserError> {
        let split = 1 + (((self.range - 1) * u16::from(probability)) >> 8);
        let decoded;

        if self.value < split {
            self.range = split;
            decoded = false;
        } else {
            self.range -= split;
            self.value -= split;
            decoded = true;
        }

        while self.range < 128 {
            let new_bit = self.read_input_bit()?;
            self.range *= 2;
            self.value = (self.value << 1) + new_bit;
        }

        Ok(decoded)
    }

    pub(crate) fn read_literal(&mut self, bits: u8) -> Result<u32, ParserError> {
        if bits > 32 {
            return Err(ParserError::InvalidBitstream);
        }

        let mut value = 0u32;
        for _ in 0..bits {
            let bit = if self.read_bool(LITERAL_PROBABILITY)? {
                1
            } else {
                0
            };
            value = (value << 1) | bit;
        }
        Ok(value)
    }

    pub(crate) fn finish(mut self) -> Result<(), ParserError> {
        while self.has_input_bit()? {
            if self.read_input_bit()? != 0 {
                return Err(ParserError::InvalidBitstream);
            }
        }
        Ok(())
    }

    fn read_input_bit(&mut self) -> Result<u16, ParserError> {
        let byte = self.current_input_byte()?;
        let bit_in_byte = 7 - (self.bit_offset & 7);
        let bit = (byte >> bit_in_byte) & 1;
        self.bit_offset = self
            .bit_offset
            .checked_add(1)
            .ok_or(ParserError::InvalidBitstream)?;
        Ok(u16::from(bit))
    }

    fn has_input_bit(&self) -> Result<bool, ParserError> {
        Ok(self.input_byte_index()? < self.data.len())
    }

    fn current_input_byte(&self) -> Result<u8, ParserError> {
        self.data
            .get(self.input_byte_index()?)
            .copied()
            .ok_or(ParserError::InvalidBitstream)
    }

    fn input_byte_index(&self) -> Result<usize, ParserError> {
        1usize
            .checked_add(self.bit_offset / 8)
            .ok_or(ParserError::InvalidBitstream)
    }
}

#[cfg(test)]
mod tests {
    use super::BoolDecoder;
    use crate::error::ParserError;

    #[test]
    fn empty_input_is_rejected() {
        assert!(matches!(
            BoolDecoder::new(&[]),
            Err(ParserError::InvalidBitstream)
        ));
    }

    #[test]
    fn non_zero_marker_is_rejected() {
        assert!(matches!(
            BoolDecoder::new(&[0x80]),
            Err(ParserError::InvalidBitstream)
        ));
        assert!(matches!(
            BoolDecoder::new(&[0x80, 0x00]),
            Err(ParserError::InvalidBitstream)
        ));
    }

    #[test]
    fn all_zero_one_byte_input_initializes_and_finishes() {
        let decoder = BoolDecoder::new(&[0x00]).unwrap();

        assert_eq!(decoder.finish(), Ok(()));
    }

    #[test]
    fn unbiased_true_path_renormalizes_from_next_byte() {
        // The marker B(128) sees value 0x40 < split 128, so it is zero and
        // leaves range=128, value=64. The next B(128) has split 64, takes the
        // true branch (value >= split), then renormalizes range 64 by consuming
        // the zero MSB from the second byte.
        let mut decoder = BoolDecoder::new(&[0x40, 0x00]).unwrap();

        assert_eq!(decoder.read_bool(128), Ok(true));
        assert_eq!(decoder.finish(), Ok(()));
    }

    #[test]
    fn literal_reads_two_unbiased_true_bits() {
        // After the zero marker, 0x60 leaves range=128, value=96. The first
        // literal bit is true and consumes a zero renormalization bit, leaving
        // range=128, value=64. The second bit is true for the same split 64 and
        // consumes the next zero renormalization bit, producing binary 11.
        let mut decoder = BoolDecoder::new(&[0x60, 0x00]).unwrap();

        assert_eq!(decoder.read_literal(2), Ok(0b11));
        assert_eq!(decoder.finish(), Ok(()));
    }

    #[test]
    fn truncated_renormalization_is_rejected() {
        let mut decoder = BoolDecoder::new(&[0x40]).unwrap();

        assert_eq!(decoder.read_bool(128), Err(ParserError::InvalidBitstream));
    }

    #[test]
    fn non_zero_exit_padding_is_rejected() {
        let decoder = BoolDecoder::new(&[0x00, 0x01]).unwrap();

        assert_eq!(decoder.finish(), Err(ParserError::InvalidBitstream));
    }

    #[test]
    fn literal_width_greater_than_32_is_rejected() {
        let mut decoder = BoolDecoder::new(&[0x00]).unwrap();

        assert_eq!(decoder.read_literal(33), Err(ParserError::InvalidBitstream));
    }

    #[test]
    fn probability_extremes_use_the_spec_split_formula() {
        let mut zero_probability = BoolDecoder::new(&[0x00, 0x00]).unwrap();
        assert_eq!(zero_probability.read_bool(0), Ok(false));
        assert_eq!(zero_probability.finish(), Ok(()));

        let mut full_probability = BoolDecoder::new(&[0x7f, 0x00]).unwrap();
        assert_eq!(full_probability.read_bool(255), Ok(true));
        assert_eq!(full_probability.finish(), Ok(()));
    }
}

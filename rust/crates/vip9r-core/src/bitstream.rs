use crate::error::ParserError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FixedBitReader<'a> {
    data: &'a [u8],
    bit_position: usize,
}

impl<'a> FixedBitReader<'a> {
    pub(crate) const fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            bit_position: 0,
        }
    }

    pub(crate) const fn bit_position(&self) -> usize {
        self.bit_position
    }

    pub(crate) fn read_bool(&mut self) -> Result<bool, ParserError> {
        Ok(self.read_f(1)? != 0)
    }

    pub(crate) fn read_f(&mut self, bits: u8) -> Result<u32, ParserError> {
        if bits > 32 || self.remaining_bits() < usize::from(bits) {
            return Err(ParserError::InvalidBitstream);
        }

        let mut value = 0u32;
        for _ in 0..bits {
            let byte = self.data[self.bit_position / 8];
            let bit_in_byte = 7 - (self.bit_position & 7);
            let bit = (byte >> bit_in_byte) & 1;
            value = (value << 1) | u32::from(bit);
            self.bit_position += 1;
        }
        Ok(value)
    }

    pub(crate) fn read_s(&mut self, bits: u8) -> Result<i32, ParserError> {
        if bits > 31 {
            return Err(ParserError::InvalidBitstream);
        }

        let value = self.read_f(bits)? as i32;
        let sign = self.read_bool()?;
        Ok(if sign { -value } else { value })
    }

    fn remaining_bits(&self) -> usize {
        self.data
            .len()
            .saturating_mul(8)
            .saturating_sub(self.bit_position)
    }
}

#[cfg(test)]
mod tests {
    use super::{FixedBitReader, ParserError};

    #[test]
    fn f_reads_across_byte_boundaries_msb_first() {
        let mut reader = FixedBitReader::new(&[0b1010_1100, 0b0110_0000]);

        assert_eq!(reader.read_f(6), Ok(0b101011));
        assert_eq!(reader.read_f(5), Ok(0b00011));
        assert_eq!(reader.bit_position(), 11);
    }

    #[test]
    fn f_reads_exact_byte_boundaries() {
        let mut reader = FixedBitReader::new(&[0b1010_1100, 0b0101_0011]);

        assert_eq!(reader.read_f(4), Ok(0b1010));
        assert_eq!(reader.read_f(4), Ok(0b1100));
        assert_eq!(reader.read_f(8), Ok(0b0101_0011));
        assert_eq!(reader.bit_position(), 16);
    }

    #[test]
    fn s_reads_value_then_sign() {
        let mut negative = FixedBitReader::new(&[0b1011_0000]);
        let mut positive = FixedBitReader::new(&[0b0110_0000]);

        assert_eq!(negative.read_s(3), Ok(-5));
        assert_eq!(positive.read_s(3), Ok(3));
    }

    #[test]
    fn f_reports_truncated_input() {
        let mut reader = FixedBitReader::new(&[0xff]);

        assert_eq!(reader.read_f(8), Ok(0xff));
        assert_eq!(reader.read_f(1), Err(ParserError::InvalidBitstream));
    }

    #[test]
    fn s_reports_truncated_sign_bit() {
        let mut reader = FixedBitReader::new(&[0b1111_1111]);

        assert_eq!(reader.read_f(5), Ok(0b11111));
        assert_eq!(reader.read_s(3), Err(ParserError::InvalidBitstream));
    }
}

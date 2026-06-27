use crate::header::UncompressedFrameHeader;

use super::{MAX_TX_COEFFS, PLANES, TileSyntaxError, TxSize, TxType};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TransformBlock {
    pub(super) tx_size: TxSize,
    pub(super) tx_type: TxType,
    pub(super) plane: usize,
    pub(super) start: (usize, usize),
}

impl TransformBlock {
    pub(super) const fn new(
        plane: usize,
        start: (usize, usize),
        tx_size: TxSize,
        tx_type: TxType,
    ) -> Self {
        Self {
            tx_size,
            tx_type,
            plane,
            start,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TransformCoefficients {
    pub(super) block: TransformBlock,
    pub(super) coefficients: [i16; MAX_TX_COEFFS],
    pub(super) eob: usize,
}

impl TransformCoefficients {
    pub(super) fn new(
        plane: usize,
        start: (usize, usize),
        tx_size: TxSize,
        tx_type: TxType,
    ) -> Result<Self, TileSyntaxError> {
        if plane >= PLANES {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        Ok(Self {
            block: TransformBlock::new(plane, start, tx_size, tx_type),
            coefficients: [0; MAX_TX_COEFFS],
            eob: 0,
        })
    }

    pub(super) fn set_signed(
        &mut self,
        pos: usize,
        magnitude: u32,
        sign_bit: u32,
    ) -> Result<(), TileSyntaxError> {
        let magnitude = i16::try_from(magnitude).map_err(|_| TileSyntaxError::InvalidBitstream)?;
        let coefficient = if sign_bit == 0 {
            magnitude
        } else {
            magnitude
                .checked_neg()
                .ok_or(TileSyntaxError::InvalidBitstream)?
        };
        self.set_quantized(pos, coefficient)
    }

    pub(super) fn set_quantized(
        &mut self,
        pos: usize,
        coefficient: i16,
    ) -> Result<(), TileSyntaxError> {
        if pos >= coefficient_count(self.block.tx_size) {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        let slot = self
            .coefficients
            .get_mut(pos)
            .ok_or(TileSyntaxError::InvalidBitstream)?;
        *slot = coefficient;
        Ok(())
    }

    pub(super) fn set_eob(&mut self, eob: usize) -> Result<(), TileSyntaxError> {
        if eob > coefficient_count(self.block.tx_size) {
            return Err(TileSyntaxError::InvalidBitstream);
        }

        self.eob = eob;
        Ok(())
    }

    pub(super) const fn nonzero_context(&self) -> bool {
        self.eob > 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DequantizedCoefficients {
    pub(super) block: TransformBlock,
    pub(super) coefficients: [i32; MAX_TX_COEFFS],
    pub(super) eob: usize,
}

impl DequantizedCoefficients {
    fn new(block: TransformBlock, eob: usize) -> Self {
        Self {
            block,
            coefficients: [0; MAX_TX_COEFFS],
            eob,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FrameDequant {
    base_q_idx: u8,
    delta_q_y_dc: i32,
    delta_q_uv_dc: i32,
    delta_q_uv_ac: i32,
}

impl FrameDequant {
    pub(super) const fn new(
        base_q_idx: u8,
        delta_q_y_dc: i32,
        delta_q_uv_dc: i32,
        delta_q_uv_ac: i32,
    ) -> Self {
        Self {
            base_q_idx,
            delta_q_y_dc,
            delta_q_uv_dc,
            delta_q_uv_ac,
        }
    }

    pub(super) const fn from_header(header: &UncompressedFrameHeader) -> Self {
        Self::new(
            header.base_q_idx,
            header.delta_q_y_dc,
            header.delta_q_uv_dc,
            header.delta_q_uv_ac,
        )
    }

    pub(super) const fn get_qindex(self) -> i32 {
        // Segmentation is rejected by tile parsing for now, so every transform
        // block uses the frame base quantizer index.
        self.base_q_idx as i32
    }

    pub(super) fn get_dc_quant(self, plane: usize) -> i32 {
        let delta = if plane == 0 {
            self.delta_q_y_dc
        } else {
            self.delta_q_uv_dc
        };
        dc_q(self.get_qindex() + delta)
    }

    pub(super) fn get_ac_quant(self, plane: usize) -> i32 {
        let delta = if plane == 0 { 0 } else { self.delta_q_uv_ac };
        ac_q(self.get_qindex() + delta)
    }

    pub(super) fn dequantize(self, input: &TransformCoefficients) -> DequantizedCoefficients {
        let mut output = DequantizedCoefficients::new(input.block, input.eob);
        let count = coefficient_count(input.block.tx_size);
        let dq_denom = dq_denom(input.block.tx_size);
        let dc_quant = self.get_dc_quant(input.block.plane);
        let ac_quant = self.get_ac_quant(input.block.plane);

        for pos in 0..count {
            let quant = if pos == 0 { dc_quant } else { ac_quant };
            output.coefficients[pos] = (i32::from(input.coefficients[pos]) * quant) / dq_denom;
        }

        output
    }
}

pub(super) const fn coefficient_count(tx_size: TxSize) -> usize {
    16 << (tx_size.index() << 1)
}

const fn dq_denom(tx_size: TxSize) -> i32 {
    match tx_size {
        TxSize::Tx32x32 => 2,
        TxSize::Tx4x4 | TxSize::Tx8x8 | TxSize::Tx16x16 => 1,
    }
}

fn dc_q(q_index: i32) -> i32 {
    DC_QLOOKUP_8BIT[clipped_q_index(q_index)]
}

fn ac_q(q_index: i32) -> i32 {
    AC_QLOOKUP_8BIT[clipped_q_index(q_index)]
}

fn clipped_q_index(q_index: i32) -> usize {
    q_index.clamp(0, 255) as usize
}

const DC_QLOOKUP_8BIT: [i32; 256] = [
    4, 8, 8, 9, 10, 11, 12, 12, 13, 14, 15, 16, 17, 18, 19, 19, 20, 21, 22, 23, 24, 25, 26, 26, 27,
    28, 29, 30, 31, 32, 32, 33, 34, 35, 36, 37, 38, 38, 39, 40, 41, 42, 43, 43, 44, 45, 46, 47, 48,
    48, 49, 50, 51, 52, 53, 53, 54, 55, 56, 57, 57, 58, 59, 60, 61, 62, 62, 63, 64, 65, 66, 66, 67,
    68, 69, 70, 70, 71, 72, 73, 74, 74, 75, 76, 77, 78, 78, 79, 80, 81, 81, 82, 83, 84, 85, 85, 87,
    88, 90, 92, 93, 95, 96, 98, 99, 101, 102, 104, 105, 107, 108, 110, 111, 113, 114, 116, 117,
    118, 120, 121, 123, 125, 127, 129, 131, 134, 136, 138, 140, 142, 144, 146, 148, 150, 152, 154,
    156, 158, 161, 164, 166, 169, 172, 174, 177, 180, 182, 185, 187, 190, 192, 195, 199, 202, 205,
    208, 211, 214, 217, 220, 223, 226, 230, 233, 237, 240, 243, 247, 250, 253, 257, 261, 265, 269,
    272, 276, 280, 284, 288, 292, 296, 300, 304, 309, 313, 317, 322, 326, 330, 335, 340, 344, 349,
    354, 359, 364, 369, 374, 379, 384, 389, 395, 400, 406, 411, 417, 423, 429, 435, 441, 447, 454,
    461, 467, 475, 482, 489, 497, 505, 513, 522, 530, 539, 549, 559, 569, 579, 590, 602, 614, 626,
    640, 654, 668, 684, 700, 717, 736, 755, 775, 796, 819, 843, 869, 896, 925, 955, 988, 1022,
    1058, 1098, 1139, 1184, 1232, 1282, 1336,
];

const AC_QLOOKUP_8BIT: [i32; 256] = [
    4, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54,
    55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78,
    79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101,
    102, 104, 106, 108, 110, 112, 114, 116, 118, 120, 122, 124, 126, 128, 130, 132, 134, 136, 138,
    140, 142, 144, 146, 148, 150, 152, 155, 158, 161, 164, 167, 170, 173, 176, 179, 182, 185, 188,
    191, 194, 197, 200, 203, 207, 211, 215, 219, 223, 227, 231, 235, 239, 243, 247, 251, 255, 260,
    265, 270, 275, 280, 285, 290, 295, 300, 305, 311, 317, 323, 329, 335, 341, 347, 353, 359, 366,
    373, 380, 387, 394, 401, 408, 416, 424, 432, 440, 448, 456, 465, 474, 483, 492, 501, 510, 520,
    530, 540, 550, 560, 571, 582, 593, 604, 615, 627, 639, 651, 663, 676, 689, 702, 715, 729, 743,
    757, 771, 786, 801, 816, 832, 848, 864, 881, 898, 915, 933, 951, 969, 988, 1007, 1026, 1046,
    1066, 1087, 1108, 1129, 1151, 1173, 1196, 1219, 1243, 1267, 1292, 1317, 1343, 1369, 1396, 1423,
    1451, 1479, 1508, 1537, 1567, 1597, 1628, 1660, 1692, 1725, 1759, 1793, 1828,
];

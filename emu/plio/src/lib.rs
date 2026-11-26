//! Access to the PL of the board.
//!
//! - [`regmap`]: the register map, from `spec/regmap.toml` through `make gen`.
//!   This file has the types that the generated code uses.
//! - [`mmio`]: the memory mappings. It is the only module with `unsafe` code.
//! - [`board`]: the board interface on Linux (UIO device, reserved region,
//!   checks before the first register read).

#[rustfmt::skip]
pub mod regmap;

pub mod board;
#[allow(unsafe_code)]
pub mod mmio;

/// A 32-bit register in the register window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reg {
    /// Byte offset from the start of the window.
    pub offset: usize,
    /// Value after reset.
    pub reset: u32,
}

/// A bit field of a register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    /// Number of the least significant bit.
    pub lsb: u32,
    /// Number of bits, from 1 to 32.
    pub width: u32,
}

/// A memory of 32-bit words in the register window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mem {
    /// Byte offset of word 0 from the start of the window.
    pub offset: usize,
    /// Number of words.
    pub depth: usize,
}

impl Field {
    /// The largest value of the field.
    pub const fn max(self) -> u32 {
        u32::MAX >> (32 - self.width)
    }

    /// The bits of the field in the register.
    pub const fn mask(self) -> u32 {
        self.max() << self.lsb
    }

    /// The value of the field in a register value.
    pub const fn get(self, reg: u32) -> u32 {
        (reg >> self.lsb) & self.max()
    }

    /// The register value with the field set to `value`. `None` if `value`
    /// does not fit the field.
    pub const fn put(self, reg: u32, value: u32) -> Option<u32> {
        if value > self.max() {
            return None;
        }
        Some((reg & !self.mask()) | (value << self.lsb))
    }
}

impl Mem {
    /// Byte offset of word `index`. `None` if `index` is not in the memory.
    pub const fn word_offset(self, index: usize) -> Option<usize> {
        if index >= self.depth {
            return None;
        }
        Some(self.offset + 4 * index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use regmap::memprobe as mp;

    #[test]
    fn field_get_put() {
        let f = Field { lsb: 8, width: 4 };
        assert_eq!(f.max(), 0xF);
        assert_eq!(f.mask(), 0x0000_0F00);
        assert_eq!(f.get(0xABCD_1234), 0x2);
        assert_eq!(f.put(0xFFFF_FFFF, 0x5), Some(0xFFFF_F5FF));
        assert_eq!(f.put(0, 0x10), None);
    }

    #[test]
    fn field_full_width() {
        let f = Field { lsb: 0, width: 32 };
        assert_eq!(f.max(), u32::MAX);
        assert_eq!(f.mask(), u32::MAX);
        assert_eq!(f.get(0x1234_5678), 0x1234_5678);
        assert_eq!(f.put(0, u32::MAX), Some(u32::MAX));
    }

    #[test]
    fn field_top_bit() {
        let f = Field { lsb: 31, width: 1 };
        assert_eq!(f.mask(), 0x8000_0000);
        assert_eq!(f.put(0, 1), Some(0x8000_0000));
    }

    #[test]
    fn mem_word_offset() {
        let m = Mem {
            offset: 0x1000,
            depth: 4,
        };
        assert_eq!(m.word_offset(0), Some(0x1000));
        assert_eq!(m.word_offset(3), Some(0x100C));
        assert_eq!(m.word_offset(4), None);
    }

    // The expected values are from spec/regmap.toml, read by hand.
    #[test]
    fn generated_memprobe_map() {
        assert_eq!(mp::ID.reset, mp::ID_VALUE);
        assert_eq!(mp::ID_VALUE, 0x4D50_5242);
        assert_eq!(mp::STATUS.offset, 0x014);
        let mode = mp::MODE.reset;
        assert_eq!(mp::mode::OP.get(mode), mp::mode::op::READ);
        assert_eq!(mp::mode::PATTERN.get(mode), mp::mode::pattern::SEQ);
        assert_eq!(mp::mode::LEN.get(mode), 15);
        assert_eq!(mp::mode::SIZE.get(mode), mp::mode::size::B8);
        assert_eq!(mp::span_log2::VALUE.get(mp::SPAN_LOG2.reset), 22);
        assert_eq!(mp::HIST_WR_RESP.word_offset(1023), Some(0x3FFC));
        assert_eq!(mp::HP_BASE % mp::HP_SIZE, 0);
    }
}

//! The dispatch ROM word: 36 bits for each opcode, bit 0 at the left end
//! **[microcode]** (the D words of the ITS microcode, `ucode::DROM`).

use crate::fields::Field;

/// The number of dispatch ROM words: one for each opcode.
pub const DROM_WORDS: usize = 512;

/// The value of a field of a 36-bit dispatch ROM word (bit 0 at the left).
pub fn drom_field(word: u64, f: Field) -> u32 {
    let width = u32::from(f.last) - u32::from(f.first) + 1;
    ((word >> (35 - u32::from(f.last))) & ((1u64 << width) - 1)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::d;

    #[test]
    fn fields_count_from_the_left() {
        // D 0001 of the ITS microcode: I, SJCL, J/L-CMS.
        assert_eq!(drom_field(0o0001_1740_2100, d::B), 0o1);
        assert_eq!(drom_field(0o0001_1740_2100, d::J), 0o1740);
        assert_eq!(crate::ucode::DROM.len(), DROM_WORDS);
        assert_eq!(crate::ucode::DROM[1], 0o0001_1740_2100);
    }
}

//! The microword and the RAM file entry.
//!
//! A microword has 108 bits in the numbering of MICRO: bit 0 is the left
//! end (`.WIDTH/108`) **[microcode]** (ks10, "MICROCODE FIELDS"). The
//! control RAM holds only the fields that the RAM file carries; the other
//! bits (`STATE`, for example) exist only in the assembler.
//!
//! The RAM file entry of one control RAM word is three 36-bit words, 108
//! bits with bit 0 at the left end of the first word **[microcode]** (ks10,
//! ".RAMFILE"; micro, RAMFL4). [`fields::RAMFILE`] gives the place of each
//! field, each pad and each parity bit in it.

use pdp10::W36;

use crate::fields::{self, Field, RamItem};

/// The number of control RAM words **[its]** (cmds.m80, DMEM2CR: the
/// loader stops at CRAM address 4000).
pub const CRAM_WORDS: usize = 0o4000;

/// The mask of a 108-bit word.
const MASK108: u128 = (1u128 << fields::WIDTH) - 1;

/// Bits `first..first + width` of a 108-bit value, bit 0 at the left end.
fn bits(v: u128, first: u32, width: u32) -> u128 {
    (v >> (fields::WIDTH - first - width)) & ((1u128 << width) - 1)
}

/// `v` with bits `first..first + width` replaced by `x`.
fn with_bits(v: u128, first: u32, width: u32, x: u128) -> u128 {
    let shift = fields::WIDTH - first - width;
    let mask = ((1u128 << width) - 1) << shift;
    (v & !mask) | ((x << shift) & mask)
}

/// A microword: 108 bits, bit 0 at the left end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Microword(u128);

impl Microword {
    /// The microword with these 108 bits (bit 0 of the word is bit 107 of
    /// the integer). Higher bits are dropped.
    pub const fn from_bits(v: u128) -> Self {
        Microword(v & MASK108)
    }

    pub const fn bits(self) -> u128 {
        self.0
    }

    /// The value of a field.
    pub fn get(self, f: Field) -> u32 {
        bits(self.0, u32::from(f.first), width(f)) as u32
    }

    /// The microword with a field set to `v` (truncated to the field).
    pub fn with(self, f: Field, v: u32) -> Self {
        Microword(with_bits(
            self.0,
            u32::from(f.first),
            width(f),
            u128::from(v),
        ))
    }
}

fn width(f: Field) -> u32 {
    u32::from(f.last) - u32::from(f.first) + 1
}

/// Why a RAM file entry is not valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RamError {
    /// A pad has bits that are not 0. `dest` is its first bit in the entry.
    Pad { dest: u8 },
    /// A parity bit is wrong. `dest` is its bit in the entry.
    Parity { dest: u8 },
}

/// The 108 bits of a RAM file entry.
fn entry_bits(words: [W36; 3]) -> u128 {
    words
        .iter()
        .fold(0u128, |acc, w| (acc << 36) | u128::from(w.get()))
}

/// The XOR of the entry bits in `ranges` (inclusive, bit 0 at the left).
fn xor_ranges(entry: u128, ranges: &[(u8, u8)]) -> u32 {
    ranges
        .iter()
        .map(|&(a, b)| {
            let (a, b) = (u32::from(a), u32::from(b));
            bits(entry, a, b - a + 1).count_ones()
        })
        .sum::<u32>()
        & 1
}

/// The microword of a RAM file entry. The pads must be 0 and the parity
/// bits must be right.
pub fn from_ram_entry(words: [W36; 3]) -> Result<Microword, RamError> {
    let entry = entry_bits(words);
    // The pads first: a pad bit is also in a parity range, and a bad pad is
    // the more exact report.
    for item in fields::RAMFILE {
        if let RamItem::Pad { width, dest } = *item
            && bits(entry, u32::from(dest), u32::from(width)) != 0
        {
            return Err(RamError::Pad { dest });
        }
    }
    for item in fields::RAMFILE {
        // The parity bit is in its own ranges; with it, the XOR is 0 for
        // "ODD PARITY" and 1 for "EVEN PARITY".
        if let RamItem::Parity { odd, ranges, dest } = *item
            && xor_ranges(entry, ranges) != u32::from(!odd)
        {
            return Err(RamError::Parity { dest });
        }
    }
    let mut mw = Microword::default();
    for item in fields::RAMFILE {
        if let RamItem::Field { field, dest } = *item {
            let v = bits(entry, u32::from(dest), width(field));
            mw = Microword(with_bits(mw.0, u32::from(field.first), width(field), v));
        }
    }
    Ok(mw)
}

/// The RAM file entry of a microword, as MICRO writes it (micro, PNTB90
/// and PNTB91): the fields, then each parity bit in the order of the
/// items.
pub fn to_ram_entry(mw: Microword) -> [W36; 3] {
    let mut entry = 0u128;
    for item in fields::RAMFILE {
        if let RamItem::Field { field, dest } = *item {
            let v = bits(mw.0, u32::from(field.first), width(field));
            entry = with_bits(entry, u32::from(dest), width(field), v);
        }
    }
    for item in fields::RAMFILE {
        if let RamItem::Parity { odd, ranges, dest } = *item {
            let p = xor_ranges(entry, ranges) ^ u32::from(!odd);
            entry ^= u128::from(p) << (fields::WIDTH - 1 - u32::from(dest));
        }
    }
    [
        W36::wrap((entry >> 72) as u64),
        W36::wrap((entry >> 36) as u64),
        W36::wrap(entry as u64),
    ]
}

/// The microword bits that the RAM file carries.
pub fn ram_mask() -> u128 {
    fields::RAMFILE.iter().fold(0u128, |m, item| match *item {
        RamItem::Field { field, .. } => {
            with_bits(m, u32::from(field.first), width(field), u128::MAX)
        }
        _ => m,
    })
}

/// Why a RAM file does not give a control RAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RamFileError {
    /// The file has `0` words; it must have 3 for each control RAM word.
    Length(usize),
    /// The entry of control RAM word `addr` is not valid.
    Entry { addr: usize, error: RamError },
}

/// The control RAM from a RAM file (three words for each control RAM
/// word, in the order of the addresses).
pub fn cram_from_ram_file(words: &[W36]) -> Result<Vec<Microword>, RamFileError> {
    if words.len() != 3 * CRAM_WORDS {
        return Err(RamFileError::Length(words.len()));
    }
    words
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .map(|(addr, w)| from_ram_entry(*w).map_err(|error| RamFileError::Entry { addr, error }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::u;

    #[test]
    fn the_ram_file_layout_fills_108_bits_without_overlap() {
        let mut used = 0u128;
        for item in fields::RAMFILE {
            let (dest, w) = match *item {
                RamItem::Field { field, dest } => (dest, width(field)),
                RamItem::Pad { width, dest } => (dest, u32::from(width)),
                RamItem::Parity { dest, .. } => (dest, 1),
            };
            let m = with_bits(0, u32::from(dest), w, u128::MAX);
            assert_eq!(used & m, 0, "overlap at {dest}");
            used |= m;
        }
        assert_eq!(used, MASK108);
    }

    #[test]
    fn a_field_reads_back_what_was_written() {
        let mw = Microword::default().with(u::J, 0o3777).with(u::AD, 0o55);
        assert_eq!(mw.get(u::J), 0o3777);
        assert_eq!(mw.get(u::AD), 0o55);
        assert_eq!(mw.bits() >> 96, 0o3777);
    }

    #[test]
    fn an_entry_decodes_to_its_microword() {
        // A pattern over all the bits that the RAM file carries.
        let mw = Microword::from_bits(0o123456701234567012345670123456701234 & ram_mask());
        let e = to_ram_entry(mw);
        assert_eq!(from_ram_entry(e), Ok(mw));
        assert_eq!(
            from_ram_entry(to_ram_entry(Microword::default())),
            Ok(Microword::default())
        );
    }

    #[test]
    fn a_wrong_parity_bit_or_pad_is_found() {
        let mw = Microword::default().with(u::J, 1);
        let e = to_ram_entry(mw);
        for item in fields::RAMFILE {
            let (dest, want) = match *item {
                RamItem::Parity { dest, .. } => (dest, RamError::Parity { dest }),
                RamItem::Pad { dest, .. } => (dest, RamError::Pad { dest }),
                RamItem::Field { .. } => continue,
            };
            let flip = 1u128 << (fields::WIDTH - 1 - u32::from(dest));
            let bad = entry_bits(e) ^ flip;
            let words = [
                W36::wrap((bad >> 72) as u64),
                W36::wrap((bad >> 36) as u64),
                W36::wrap(bad as u64),
            ];
            assert_eq!(from_ram_entry(words), Err(want));
        }
    }

    #[test]
    fn a_ram_file_has_three_words_for_each_cram_word() {
        assert_eq!(
            cram_from_ram_file(&[W36::ZERO; 5]),
            Err(RamFileError::Length(5))
        );
        let words = vec![W36::ZERO; 3 * CRAM_WORDS];
        assert_eq!(cram_from_ram_file(&words).map(|c| c.len()), Ok(CRAM_WORDS));
    }
}

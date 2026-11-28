//! The record of a completed instruction, for the co-simulation of the
//! model with the logic in `rtl/` (AGENTS.md, "Rules for the logic").
//!
//! The model takes a record at the end of each microword with
//! `SPEC/NICOND` (ks10, `NEXT INST`), after all writes of that word.
//! The logic runs the same microcode on the same data path, so the
//! record holds data path state, not the state of the PRM:
//!
//! - `pc` is the 2901 register PC. `INCPC` adds 1 to it at the start of
//!   an instruction, and `DONE` fetches from it, so at `NICOND` it holds
//!   the address of the next instruction (simple.42, `INCPC`; ks10,
//!   `DONE`) **[microcode]**.
//! - The console `JRST` of ST and BT is an instruction and has a record.
//!   The halt loop has none.
//! - `writes` and `write_hash` cover the memory writes of the processor
//!   since the record before: the bus cycles with a write, not the DMA of
//!   the devices. A BLT can write many words, so the record keeps a count
//!   and a hash, not the words.

use pdp10::{Half, W36};

use crate::machine::PcFlags;

/// The record of a completed instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    /// The completed instructions, this one included.
    pub count: u64,
    /// The microcycles from power on to the end of the `NICOND` word.
    pub cycles: u64,
    /// The 2901 register PC.
    pub pc: W36,
    /// The program flags in the left half bits of the PC word (PRM 2.9,
    /// "JRST"; DPE9 E209, E210, E305).
    pub flags: Half,
    /// The 16 ACs of the current block of the RAM file.
    pub ac: [W36; 16],
    /// The memory writes of the processor since the record before.
    pub writes: u64,
    /// FNV-1a (64 bits) of the address and the word of each such write, in
    /// order: 8 bytes of the address, then 8 bytes of the word, each little
    /// endian.
    pub write_hash: u64,
}

/// The start value of FNV-1a, 64 bits.
pub const HASH_START: u64 = 0xcbf2_9ce4_8422_2325;
const HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

/// `hash` after a memory write of `word` at `address`.
pub fn hash_write(hash: u64, address: u64, word: W36) -> u64 {
    address
        .to_le_bytes()
        .iter()
        .chain(word.get().to_le_bytes().iter())
        .fold(hash, |h, &b| (h ^ u64::from(b)).wrapping_mul(HASH_PRIME))
}

/// The program flags as the left half of a PC word: bit 0 overflow, 1
/// carry 0, 2 carry 1, 3 floating overflow, 4 first part done, 5 user,
/// 6 user I/O (PCU in executive mode), 9 trap 2, 10 trap 1, 11 floating
/// underflow, 12 no divide **[manual]** (PRM 2.9, "JRST").
pub fn flag_half(f: PcFlags) -> Half {
    [
        (f.ov, 0),
        (f.cry0, 1),
        (f.cry1, 2),
        (f.fov, 3),
        (f.fpd, 4),
        (f.user, 5),
        (f.pcu, 6),
        (f.trap2, 9),
        (f.trap1, 10),
        (f.fxu, 11),
        (f.nodiv, 12),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .fold(Half::ZERO, |h, &(_, bit)| {
        Half::wrap(h.get() | 1 << (17 - bit))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // FNV-1a of no data is the start value; the 64-bit value of one byte
    // 0x61 ("a") is af63dc4c8601ec8c (the test vectors of the FNV
    // reference by Fowler, Noll and Vo).
    #[test]
    fn fnv_1a_vectors() {
        let one = (HASH_START ^ 0x61).wrapping_mul(HASH_PRIME);
        assert_eq!(one, 0xaf63_dc4c_8601_ec8c);
    }

    // Left half bit b has the value 2^(17 - b).
    #[test]
    fn flags_in_the_left_half() {
        let f = PcFlags {
            ov: true,
            nodiv: true,
            ..PcFlags::default()
        };
        assert_eq!(flag_half(f).get(), 0o400000 | 0o000040);
        let u = PcFlags {
            user: true,
            trap1: true,
            ..PcFlags::default()
        };
        assert_eq!(flag_half(u).get(), 0o010000 | 0o000200);
    }
}

//! The program flags (PC flags).
//!
//! The flags are bits 0 to 12 of a saved flag word **[manual]** (PRM 1982,
//! 2.9 "Program Flags", page 2-65). A [`Flags`] value keeps them at those
//! positions of a left half, so that `flags,,PC` is the PC word.

use pdp10::{Half, W36};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags(pub u32);

impl Flags {
    /// Bit 0: Overflow. (The KS10 has no public mode: bit 0 is always
    /// Overflow, PRM 2-65.)
    pub const AROV: u32 = 1 << 17;
    /// Bit 1: Carry 0.
    pub const CRY0: u32 = 1 << 16;
    /// Bit 2: Carry 1.
    pub const CRY1: u32 = 1 << 15;
    /// Bit 3: Floating Overflow.
    pub const FOV: u32 = 1 << 14;
    /// Bit 4: First Part Done.
    pub const FPD: u32 = 1 << 13;
    /// Bit 5: User.
    pub const USER: u32 = 1 << 12;
    /// Bit 6: User In-out (user mode), Previous Context User (executive).
    pub const USERIO: u32 = 1 << 11;
    /// Bit 8 of a PC word: the ITS one-proceed bit. The KS10 has no flag
    /// there (ks10.47, OIPBIT: "BIT 8 OF PC IS ONE-PROCEED ON KS (NOT IN
    /// HARDWARE)"); JRSTF with this bit starts a one-proceed.
    pub const OIP: u32 = 1 << 9;
    /// Bit 9: Trap 2.
    pub const TRAP2: u32 = 1 << 8;
    /// Bit 10: Trap 1.
    pub const TRAP1: u32 = 1 << 7;
    /// Bit 11: Floating Underflow.
    pub const FXU: u32 = 1 << 6;
    /// Bit 12: No Divide.
    pub const NODIV: u32 = 1 << 5;
    /// All flags of bits 0 to 12 that the KS10 has: bit 7 (Public) and
    /// bit 8 are not used.
    pub const ALL: u32 = 0o777740 & !(1 << 10) & !(1 << 9);

    pub fn has(self, f: u32) -> bool {
        self.0 & f != 0
    }

    pub fn set(&mut self, f: u32) {
        self.0 |= f;
    }

    pub fn clear(&mut self, f: u32) {
        self.0 &= !f;
    }

    pub fn is_user(self) -> bool {
        self.has(Self::USER)
    }

    /// I/O instructions are legal: executive mode, or User In-out
    /// (ks10.47, "SKIP IO LEGAL").
    pub fn io_legal(self) -> bool {
        !self.is_user() || self.has(Self::USERIO)
    }

    /// The PC word: the flags in bits 0 to 12, 0 in bits 13 to 17, the PC
    /// in the right half (PRM 2-65).
    pub fn pc_word(self, pc: Half) -> W36 {
        W36::from_halves(Half::wrap(u64::from(self.0 & Self::ALL)), pc)
    }

    /// The flags of the result of an addition or a subtraction (PRM 2.2,
    /// "Fixed Point Arithmetic"): Carry 0 and Carry 1 from the carries; if
    /// they differ, Overflow and Trap 1. A flag that is set stays set.
    pub fn arith(&mut self, cry0: bool, cry1: bool) {
        if cry0 {
            self.set(Self::CRY0);
        }
        if cry1 {
            self.set(Self::CRY1);
        }
        if cry0 != cry1 {
            self.set(Self::AROV | Self::TRAP1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Flags;

    // PRM 2.2: an instruction sets flags and does not clear them; the trap
    // flags clear when the processor takes the trap (PRM 2.9).
    #[test]
    fn arithmetic_flags_only_set() {
        let mut f = Flags(Flags::TRAP1 | Flags::TRAP2 | Flags::FPD);
        f.arith(false, false);
        assert_eq!(f, Flags(Flags::TRAP1 | Flags::TRAP2 | Flags::FPD));
        let mut f = Flags(Flags::TRAP2);
        f.arith(true, false);
        assert_eq!(
            f,
            Flags(Flags::CRY0 | Flags::AROV | Flags::TRAP1 | Flags::TRAP2)
        );
    }
}

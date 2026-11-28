//! The implementation differences of `spec/impldef.toml` for the
//! differential runs (docs/conformance.md, "Masks of the differential
//! runs").
//!
//! [`CASES`] comes from the table through `make gen`. This module has the
//! types of the table and the predicates of its vocabulary: each name of
//! `PREDICATES` in `spec/gen_impldef.py` is one variant of [`Pred`], and
//! [`Pred::holds`] decides it from the operands of an instruction.
//!
//! A run uses [`matching`] to find the cases of an instruction. For a case
//! with [`Compare::Skip`] the run does not compare the instruction; for
//! [`Compare::Mask`] it leaves out the fields of [`Case::differs`]. A case
//! with [`Compare::NotGenerated`] that matches an instruction of the run
//! is an error of the table.

#[rustfmt::skip]
mod table;

pub use table::CASES;

use crate::W36;

/// What a differential run does with the instructions of a case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compare {
    /// The run compares the whole state.
    Exact,
    /// The run leaves out the fields of [`Case::differs`].
    Mask,
    /// The run does not compare the instruction.
    Skip,
    /// The run makes no instruction of the case.
    NotGenerated,
}

/// A differential run of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    /// `ks10-core-soft` against SIMH.
    Simh,
    /// `ks10-core-soft` against KLH10.
    Klh10,
    /// `ks10-core-uop` against `ks10-core-soft`.
    UopSoft,
}

/// A part of the state that differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Ac,
    Mem,
    Pc,
    Register,
    /// Program flags, as DEC bit numbers of the PC word.
    Flags(&'static [u8]),
    /// Bits of the page fail word, first to last.
    Pfw(u8, u8),
}

/// An opcode, and the AC for an instruction of an AC dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Op {
    pub code: u16,
    pub ac: Option<u8>,
}

/// The instructions of a case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ops {
    All,
    Codes(&'static [Op]),
}

/// The predicates of `when` (`PREDICATES` in `spec/gen_impldef.py` gives
/// the text of each one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pred {
    Any,
    /// (opcode, n): a left shift by more than n places.
    LeftShiftOver(&'static [(u16, i32)]),
    CountMinus256,
    ImulNegativeProductOverWord,
    IdivMinByUnit,
    DivMin70ByZero,
    HllePositiveBits13,
    DivHighEqualsDivisor,
    /// One of these flags (DEC bit numbers) is set at the start.
    StartFlags(&'static [u8]),
    ExponentDifferenceOver(u32),
    UnnormalizedOperand,
    PageFail,
    LostBitBeforeLastStep,
}

/// A case of the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Case {
    pub name: &'static str,
    pub ops: Ops,
    pub pred: Pred,
    pub differs: &'static [Field],
    pub simh: Compare,
    pub klh10: Compare,
    pub uop_soft: Compare,
}

impl Case {
    /// The comparison of the case in a run.
    pub fn compare(&self, run: Run) -> Compare {
        match run {
            Run::Simh => self.simh,
            Run::Klh10 => self.klh10,
            Run::UopSoft => self.uop_soft,
        }
    }

    /// Is the instruction one of the case?
    pub fn has_op(&self, inst: W36) -> bool {
        let code = inst.field(0, 8) as u16;
        let ac = inst.field(9, 12) as u8;
        match self.ops {
            Ops::All => true,
            Ops::Codes(ops) => ops
                .iter()
                .any(|o| o.code == code && o.ac.is_none_or(|a| a == ac)),
        }
    }
}

/// The mask of program flags (DEC bit numbers) in the left half of a PC
/// word, bit 0 at the left (PRM 1982, 2.9, "Program Flags").
pub const fn flag_mask(bits: &[u8]) -> u32 {
    let mut m = 0;
    let mut k = 0;
    while k < bits.len() {
        m |= 1 << (17 - bits[k] as u32);
        k += 1;
    }
    m
}

/// The inputs of one instruction: no paging, no index, no indirect.
#[derive(Clone, Copy, Debug)]
pub struct Operands {
    pub inst: W36,
    pub ac: [W36; 16],
    /// The words at E and E + 1 (an AC if the address is below 20).
    pub e: [W36; 2],
    /// The program flags at the start, as in the left half of a PC word.
    pub flags: u32,
}

const MAG: u64 = (1 << 35) - 1;

impl Operands {
    fn code(&self) -> u16 {
        self.inst.field(0, 8) as u16
    }
    fn a(&self) -> usize {
        self.inst.field(9, 12) as usize
    }
    fn y(&self) -> u64 {
        self.inst.field(18, 35)
    }
    fn acw(&self, k: usize) -> W36 {
        self.ac[k & 0o17]
    }
    /// The shift count of E: bits 18 and 28 to 35 (ks10.47, SHIFT).
    fn count(&self) -> i32 {
        let low = (self.y() & 0o377) as i32;
        if self.y() & 0o400000 != 0 {
            low - 256
        } else {
            low
        }
    }
    /// The operand of an instruction of 220 to 237: 0,,E for the
    /// immediate form (mode 1), else C(E).
    fn operand(&self) -> W36 {
        if self.code() & 3 == 1 {
            W36::wrap(self.y())
        } else {
            self.e[0]
        }
    }
}

/// The exponent of a single precision number: bits 1 to 8, complemented
/// for a negative number (PRM 1982, 2.6).
fn exponent(w: W36) -> u32 {
    let e = w.field(1, 8) as u32;
    if w.is_negative() { e ^ 0o377 } else { e }
}

/// A sign and 35 or 70 magnitude bits shifted left `n` places, one place in
/// each step (simple.42, ASHL, ASHCL): did a step before the last lose a
/// bit, and the last step none? A step loses a bit if the bit that leaves
/// bit 1 differs from the sign.
fn lost_before_last(sign: bool, mut mag: u128, bits: u32, n: i32) -> bool {
    let top = 1u128 << (bits - 1);
    let all = (1u128 << bits) - 1;
    let mut early = false;
    for step in 1..=n {
        let lost = (mag & top != 0) != sign;
        if step == n {
            return early && !lost;
        }
        early |= lost;
        mag = (mag << 1) & all;
    }
    false
}

impl Pred {
    /// Does the predicate hold for the operands? `None`: the operands do
    /// not decide it (a page failure needs a run; the floating point
    /// predicates decide only FAD, FSB, FMP and FDV with an operand in
    /// memory).
    pub fn holds(&self, o: &Operands) -> Option<bool> {
        let a = o.a();
        Some(match *self {
            Pred::Any => true,
            Pred::LeftShiftOver(list) => list
                .iter()
                .any(|&(code, n)| code == o.code() && o.count() > n),
            Pred::CountMinus256 => o.count() == -256,
            Pred::ImulNegativeProductOverWord => {
                let p = i128::from(o.acw(a).signed()) * i128::from(o.operand().signed());
                let m = p.unsigned_abs();
                p < 0 && m > 1 << 35 && m >> 35 == 1
            }
            Pred::IdivMinByUnit => {
                o.acw(a).get() == 1 << 35 && o.operand().signed().unsigned_abs() == 1
            }
            Pred::DivMin70ByZero => {
                o.acw(a).get() == 1 << 35 && o.acw(a + 1).get() & MAG == 0 && o.operand().get() == 0
            }
            Pred::HllePositiveBits13 => {
                // HLLEM (532) has the source in the AC.
                let src = if o.code() == 0o532 { o.acw(a) } else { o.e[0] };
                !src.is_negative() && src.field(1, 3) != 0
            }
            Pred::DivHighEqualsDivisor => {
                let d =
                    (i128::from(o.acw(a).signed()) << 35) | i128::from(o.acw(a + 1).get() & MAG);
                let dv = i128::from(o.operand().signed()).abs();
                dv != 0 && d.abs() >> 35 == dv
            }
            Pred::StartFlags(bits) => o.flags & flag_mask(bits) != 0,
            Pred::ExponentDifferenceOver(n) => {
                if !matches!(o.code(), 0o140 | 0o150 | 0o160 | 0o170) {
                    return None;
                }
                let (x, y) = (o.acw(a), o.e[0]);
                x.get() != 0 && y.get() != 0 && exponent(x).abs_diff(exponent(y)) > n
            }
            Pred::UnnormalizedOperand | Pred::PageFail => return None,
            Pred::LostBitBeforeLastStep => {
                let n = o.count();
                match o.code() {
                    0o240 => {
                        let w = o.acw(a);
                        lost_before_last(w.is_negative(), u128::from(w.get() & MAG), 35, n)
                    }
                    0o244 => {
                        let (h, l) = (o.acw(a), o.acw(a + 1));
                        let mag = (u128::from(h.get() & MAG) << 35) | u128::from(l.get() & MAG);
                        lost_before_last(h.is_negative(), mag, 70, n)
                    }
                    _ => false,
                }
            }
        })
    }
}

/// The cases of the table that apply to the operands in a run, with their
/// comparison. A predicate that the operands do not decide does not match.
pub fn matching(run: Run, o: &Operands) -> impl Iterator<Item = (&'static Case, Compare)> + '_ {
    CASES
        .iter()
        .filter(move |c| c.has_op(o.inst) && c.pred.holds(o) == Some(true))
        .map(move |c| (c, c.compare(run)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ops(inst: u64, ac: &[(usize, u64)], e: [u64; 2], flags: u32) -> Operands {
        let mut a = [W36::wrap(0); 16];
        for &(k, w) in ac {
            a[k] = W36::wrap(w);
        }
        Operands {
            inst: W36::wrap(inst),
            ac: a,
            e: e.map(W36::wrap),
            flags,
        }
    }
    fn i(op: u64, ac: u64, e: u64) -> u64 {
        (op << 27) | (ac << 23) | e
    }

    // The PRM gives the flag bits: Overflow 0, Trap 2 9, Trap 1 10
    // (PRM 1982, 2.9); bit 0 is bit 17 of the left half.
    #[test]
    fn flag_masks() {
        assert_eq!(flag_mask(&[0]), 0o400000);
        assert_eq!(flag_mask(&[9, 10]), 0o000600);
    }

    // discrepancies.md, "Trap 1 and Trap 2": ASH 1,2 of 200000,,1 loses a
    // bit in the first step only; ASH 1,1 loses it in the last step.
    #[test]
    fn lost_bit_before_last_step() {
        let p = Pred::LostBitBeforeLastStep;
        let w = [(1, 0o200000_000001)];
        assert_eq!(p.holds(&ops(i(0o240, 1, 2), &w, [0, 0], 0)), Some(true));
        assert_eq!(p.holds(&ops(i(0o240, 1, 1), &w, [0, 0], 0)), Some(false));
        // 300000,,0: both steps of a shift by 2 lose a bit.
        assert_eq!(
            p.holds(&ops(i(0o240, 1, 2), &[(1, 0o300000_000000)], [0, 0], 0)),
            Some(false)
        );
        // A negative number: 600000,,0 shifted 2 places: the first step
        // keeps 1 in bit 1, the second shifts out a 0 of bit 2: lost last.
        assert_eq!(
            p.holds(&ops(i(0o240, 1, 2), &[(1, 0o600000_000000)], [0, 0], 0)),
            Some(false)
        );
        // 500000,,0 is negative with bit 1 = 0: the first step loses a
        // bit, the second shifts out bit 2 = 1: kept.
        assert_eq!(
            p.holds(&ops(i(0o240, 1, 2), &[(1, 0o500000_000000)], [0, 0], 0)),
            Some(true)
        );
        // ASHC: the high word 200000,,0 and a low word 0, by 2 places.
        let d = [(1, 0o200000_000000), (2, 0)];
        assert_eq!(p.holds(&ops(i(0o244, 1, 2), &d, [0, 0], 0)), Some(true));
        // A right shift (count -2) loses no bit.
        assert_eq!(
            p.holds(&ops(i(0o240, 1, 0o777776), &w, [0, 0], 0)),
            Some(false)
        );
    }

    // ks10.47, SHIFT: the count is bits 18 and 28 to 35.
    #[test]
    fn shift_counts() {
        assert_eq!(
            Pred::CountMinus256.holds(&ops(i(0o240, 1, 0o400000), &[], [0, 0], 0)),
            Some(true)
        );
        assert_eq!(
            Pred::CountMinus256.holds(&ops(i(0o240, 1, 0o777400), &[], [0, 0], 0)),
            Some(true)
        );
        assert_eq!(
            Pred::CountMinus256.holds(&ops(i(0o240, 1, 0o777401), &[], [0, 0], 0)),
            Some(false)
        );
        let p = Pred::LeftShiftOver(&[(0o240, 28)]);
        assert_eq!(p.holds(&ops(i(0o240, 1, 29), &[], [0, 0], 0)), Some(true));
        assert_eq!(p.holds(&ops(i(0o240, 1, 28), &[], [0, 0], 0)), Some(false));
        assert_eq!(p.holds(&ops(i(0o244, 1, 40), &[], [0, 0], 0)), Some(false));
    }

    #[test]
    fn arithmetic() {
        // -2^35 / 1 and / -1; the immediate form IDIVI takes 0,,E.
        let min = [(1, 0o400000_000000)];
        let p = Pred::IdivMinByUnit;
        assert_eq!(
            p.holds(&ops(i(0o230, 1, 0o2000), &min, [0o777777_777777, 0], 0)),
            Some(true)
        );
        assert_eq!(p.holds(&ops(i(0o231, 1, 1), &min, [5, 0], 0)), Some(true));
        assert_eq!(
            p.holds(&ops(i(0o230, 1, 0o2000), &min, [2, 0], 0)),
            Some(false)
        );
        // -2^70 / 0.
        let p = Pred::DivMin70ByZero;
        assert_eq!(
            p.holds(&ops(
                i(0o234, 1, 0o2000),
                &[(1, 1 << 35), (2, 0)],
                [0, 0],
                0
            )),
            Some(true)
        );
        assert_eq!(
            p.holds(&ops(
                i(0o234, 1, 0o2000),
                &[(1, 1 << 35), (2, 1)],
                [0, 0],
                0
            )),
            Some(false)
        );
        // IMUL: 400000,,1 is -(2^35 - 1); times 2 it is -(2^36 - 2), a
        // magnitude between 2^35 + 1 and 2^36 - 1.
        let p = Pred::ImulNegativeProductOverWord;
        assert_eq!(
            p.holds(&ops(
                i(0o220, 1, 0o2000),
                &[(1, 0o400000_000001)],
                [2, 0],
                0
            )),
            Some(true)
        );
        // -2^35 itself fits.
        assert_eq!(
            p.holds(&ops(
                i(0o220, 1, 0o2000),
                &[(1, 0o400000_000000)],
                [1, 0],
                0
            )),
            Some(false)
        );
        // DIV: the high word 0,,5 with the divisor 5 (or -5).
        let p = Pred::DivHighEqualsDivisor;
        assert_eq!(
            p.holds(&ops(i(0o234, 1, 0o2000), &[(1, 0), (2, 5)], [5, 0], 0)),
            Some(false)
        );
        let hi = [(1, 0o000000_000005), (2, 0)];
        assert_eq!(
            p.holds(&ops(i(0o234, 1, 0o2000), &hi, [5, 0], 0)),
            Some(true)
        );
        assert_eq!(
            p.holds(&ops(i(0o234, 1, 0o2000), &hi, [0o777777_777773, 0], 0)),
            Some(true)
        );
    }

    #[test]
    fn hlle_and_flags() {
        let p = Pred::HllePositiveBits13;
        assert_eq!(
            p.holds(&ops(i(0o530, 1, 0o2000), &[], [0o100000_000000, 0], 0)),
            Some(true)
        );
        assert_eq!(
            p.holds(&ops(i(0o530, 1, 0o2000), &[], [0o500000_000000, 0], 0)),
            Some(false)
        );
        assert_eq!(
            p.holds(&ops(
                i(0o532, 1, 0o2000),
                &[(1, 0o040000_000000)],
                [0, 0],
                0
            )),
            Some(true)
        );
        let p = Pred::StartFlags(&[10]);
        assert_eq!(
            p.holds(&ops(i(0o270, 1, 0o2000), &[], [0, 0], 0o000200)),
            Some(true)
        );
        assert_eq!(
            p.holds(&ops(i(0o270, 1, 0o2000), &[], [0, 0], 0o000400)),
            Some(false)
        );
    }

    #[test]
    fn exponents() {
        // 201400,,0 is 1.0 (exponent 201); 576400,,0 is -1.0.
        let p = Pred::ExponentDifferenceOver(60);
        let one = 0o201400_000000;
        let big = 0o301400_000000; // exponent 301: 100 (octal) = 64 more
        assert_eq!(
            p.holds(&ops(i(0o140, 1, 0o2000), &[(1, one)], [big, 0], 0)),
            Some(true)
        );
        assert_eq!(
            p.holds(&ops(
                i(0o140, 1, 0o2000),
                &[(1, 0o576400_000000)],
                [big, 0],
                0
            )),
            Some(true)
        );
        assert_eq!(
            p.holds(&ops(i(0o140, 1, 0o2000), &[(1, one)], [one, 0], 0)),
            Some(false)
        );
        assert_eq!(
            p.holds(&ops(i(0o140, 1, 0o2000), &[(1, one)], [0, 0], 0)),
            Some(false)
        );
        assert_eq!(
            p.holds(&ops(i(0o240, 1, 0o2000), &[(1, one)], [big, 0], 0)),
            None
        );
    }

    #[test]
    fn the_table_names_instructions_of_the_table() {
        for c in CASES {
            if let Ops::Codes(list) = c.ops {
                for o in list {
                    assert!(
                        usize::from(o.code) < crate::instructions::OPS.len(),
                        "{}",
                        c.name
                    );
                }
            }
        }
        assert!(CASES.iter().any(|c| c.name == "ash-trap-last-step"));
    }
}

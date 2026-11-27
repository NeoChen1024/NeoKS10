//! The arithmetic unit: ten Am2901 slices of 4 bits, the 2902 carry
//! lookahead and the shift mixers (docs/microcode-model.md, "Arithmetic
//! unit" and "Shifts").
//!
//! The unit has 40 bits, numbered −2 to 37. Index 0 of a [`W40`] (its most
//! significant bit) is bit −2. A 36-bit word sits in bits 0 to 35: the D
//! input gets two copies of bit 0 at the left and two 0 bits at the right
//! **[manual]** (M8620-0-DPE1 rev A1, E505 D0–D3; M8620-0-DPE2 rev A1,
//! E523 D0–D3).

use pdp10::{W36, Word};

/// A value of the 40-bit unit.
pub type W40 = Word<40>;

/// The left half: bits −2 to 17, the five slices of DPE1.
const LEFT: u64 = 0xF_FFFF_u64 << 20;
/// The right half: bits 18 to 37, the five slices of DPE2.
const RIGHT: u64 = 0xF_FFFF;
const ALL: u64 = LEFT | RIGHT;

/// The value of a 36-bit word at the D input.
pub fn d_input(w: W36) -> W40 {
    let v = w.get();
    let sign = if w.is_negative() { 0b11 << 38 } else { 0 };
    W40::wrap(sign | (v << 2))
}

/// The 36-bit word in bits 0 to 35 of a 40-bit value (DP 00 to DP 35).
pub fn word(v: W40) -> W36 {
    W36::wrap(v.get() >> 2)
}

/// The bit at index `i` (0 is bit −2, 39 is bit 37).
fn bit(v: u64, i: u32) -> bool {
    (v >> (39 - i)) & 1 != 0
}

fn with_bit(v: u64, i: u32, b: bool) -> u64 {
    let m = 1u64 << (39 - i);
    if b { v | m } else { v & !m }
}

/// Index of bit 01, the right end of the leftmost slice.
const I01: u32 = 3;
/// Index of bit 02, the left end of the second slice.
const I02: u32 = 4;
/// Index of bit 37.
const I37: u32 = 39;

/// The 2901 controls of one microinstruction.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// The 2901 function I5–I3, after the DIVIDE and MULTI PREC logic.
    pub func: u8,
    /// The source I2–I0 of the left half (the `AD` field, bits 15–17).
    pub src_left: u8,
    /// The source I2–I0 of the right half (`RSRC`).
    pub src_right: u8,
    /// The `DEST` field (before the inversion of its 2 bit).
    pub dest: u8,
    pub a: u8,
    pub b: u8,
    /// DPE5 CARRY IN.
    pub carry_in: bool,
    /// `SPEC/INHCRY18`: no carry from bit 18 into bit 17.
    pub inhibit_carry_18: bool,
    /// `CLKL`, `CLKR`: clock the left and the right half.
    pub clock_left: bool,
    pub clock_right: bool,
    /// The right three bits of `SPEC`.
    pub shstyle: u8,
    /// DPE5 MULTI SHIFT and DIVIDE SHIFT.
    pub multi_shift: bool,
    pub divide_shift: bool,
}

/// The outputs of one microinstruction. A carry or a shift line is `None`
/// where no source of this project gives its value (a carry of a logic
/// function, a line that nothing drives).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outputs {
    /// The function output F.
    pub f: W40,
    /// The chip output Y, which is DP.
    pub y: W40,
    /// CARRY OUT: out of bit −2 (DPE2 E709).
    pub carry_out: Option<bool>,
    /// CARRY 02: out of bit 02 into bit 01 (DPE2 E710).
    pub carry_02: Option<bool>,
    /// The line DPE1 FL 02 between bit 01 and bit 02 of the main register.
    pub fl_02: Option<bool>,
    /// The line DPE1-2 QR 37 at the right end of Q.
    pub qr_37: Option<bool>,
}

impl Outputs {
    /// ADL SIGN: the F SIGN pin of the leftmost slice (DPE1 E505).
    pub fn adl_sign(&self) -> bool {
        bit(self.f.get(), 0)
    }
    /// ADR SIGN: the F SIGN pin of the slice of bits 18–21 (DPE2 E515).
    pub fn adr_sign(&self) -> bool {
        bit(self.f.get(), 20)
    }
    /// ADL=0 and ADR=0: the F=0 pins of each half (DPE1 E513, DPE2 E523).
    pub fn adl_zero(&self) -> bool {
        self.f.get() & LEFT == 0
    }
    pub fn adr_zero(&self) -> bool {
        self.f.get() & RIGHT == 0
    }
}

/// The 16 registers and Q of the ten slices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alu {
    pub reg: [W40; 16],
    pub q: W40,
}

/// Does a 2901 source code use the D input?
pub fn uses_d(src: u8) -> bool {
    src >= 5
}

impl Alu {
    pub fn new(fill: W40) -> Self {
        Alu {
            reg: [fill; 16],
            q: fill,
        }
    }

    /// The R and S operands of a source code (2901: AQ, AB, ZQ, ZB, ZA,
    /// DA, DQ, DZ).
    fn operands(&self, src: u8, a: u64, b: u64, d: u64) -> (u64, u64) {
        let q = self.q.get();
        match src & 7 {
            0 => (a, q),
            1 => (a, b),
            2 => (0, q),
            3 => (0, b),
            4 => (0, a),
            5 => (d, a),
            6 => (d, q),
            _ => (d, 0),
        }
    }

    /// The function output F and the carries, with D at the D input.
    fn function(&self, c: &Controls, d: W40) -> (u64, Option<bool>, Option<bool>) {
        let a = self.reg[usize::from(c.a & 15)].get();
        let b = self.reg[usize::from(c.b & 15)].get();
        let (rl, sl) = self.operands(c.src_left, a, b, d.get());
        let (rr, sr) = self.operands(c.src_right, a, b, d.get());
        let r = (rl & LEFT) | (rr & RIGHT);
        let s = (sl & LEFT) | (sr & RIGHT);
        let (x, y) = match c.func & 7 {
            0 => (r, s),        // ADD: R plus S
            1 => (s, !r & ALL), // SUBR: S minus R
            2 => (r, !s & ALL), // SUBS: R minus S
            3 => return (r | s, None, None),
            4 => return (r & s, None, None),
            5 => return (!r & s & ALL, None, None),
            6 => return (r ^ s, None, None),
            _ => return (!(r ^ s) & ALL, None, None),
        };
        // Right half, bits 18–37 (shift 0–19).
        let right = (x & RIGHT) + (y & RIGHT) + u64::from(c.carry_in);
        let carry_18 = right >> 20 != 0 && !c.inhibit_carry_18;
        // Bits 02–17 (shift 20–35).
        let mid = ((x >> 20) & 0xFFFF) + ((y >> 20) & 0xFFFF) + u64::from(carry_18);
        let carry_02 = mid >> 16 != 0;
        // Bits −2 to 01 (shift 36–39).
        let top = ((x >> 36) & 0xF) + ((y >> 36) & 0xF) + u64::from(carry_02);
        let f = ((top & 0xF) << 36) | ((mid & 0xFFFF) << 20) | (right & RIGHT);
        (f, Some(top >> 4 != 0), Some(carry_02))
    }

    /// The output Y before the registers change. It does not depend on D
    /// if `DEST/A` selects the A register or if no source uses D; then
    /// `d` is not used.
    pub fn y_without_d(&self, c: &Controls) -> Option<W40> {
        if c.dest & 7 == 0 {
            return Some(self.reg[usize::from(c.a & 15)]);
        }
        if uses_d(c.src_left) || uses_d(c.src_right) {
            return None;
        }
        Some(W40::wrap(self.function(c, W40::ZERO).0))
    }

    /// Run one microinstruction: compute F and Y, and clock the registers.
    pub fn step(&mut self, c: &Controls, d: W40) -> Outputs {
        let (f, carry_out, carry_02) = self.function(c, d);
        let a = self.reg[usize::from(c.a & 15)].get();
        let q = self.q.get();
        let dest = c.dest & 7;
        let y = if dest == 0 { a } else { f };
        // DEST bit 2 (the 2 weight of the field) selects the direction of
        // the shift mixers: 0 left, 1 right (DPE5 E403, E384). The 2901
        // destination is the field with that bit inverted.
        let left = dest & 2 == 0;
        let shift_f = dest >= 4;
        let shift_q = dest == 4 || dest == 6;
        let style = c.shstyle & 7;
        let dp_sign = bit(y, 0);
        let (mut new_b, mut new_q) = (None, None);
        let (fl_02, qr_37);
        if left {
            // DPE1 E706, E705, E704 drive FR 01, FR 37, QR 37.
            let f02 = bit(f, I02);
            let q02 = bit(q, I02);
            let into_f01 = match style {
                0 | 5 | 6 => f02,
                _ => dp_sign,
            };
            let into_f37 = match style {
                0 => c.multi_shift,
                1 => false,
                2 => true,
                3 => f02,
                _ => q02,
            };
            let into_q37 = match style {
                0 | 1 | 4 | 5 => false,
                2 => true,
                3 => q02,
                6 => c.divide_shift,
                _ => f02,
            };
            qr_37 = Some(into_q37);
            // The slice of bits 02–05 drives FL 02 only while it shifts up.
            fl_02 = shift_f.then_some(f02);
            if shift_f {
                let mut v = (f << 1) & ALL;
                v = with_bit(v, I01, into_f01);
                v = with_bit(v, I37, into_f37);
                new_b = Some(v);
            }
            if shift_q {
                // DPE1 E702: Q 02 goes into Q 01.
                let mut v = (q << 1) & ALL;
                v = with_bit(v, I01, q02);
                v = with_bit(v, I37, into_q37);
                new_q = Some(v);
            }
        } else {
            // DPE1 E703, E707 drive FL 02, QL 02; E702 drives F SIGN, Q SIGN.
            let f01 = bit(f, I01);
            let f37 = bit(f, I37);
            let into_f02 = match style {
                0 | 4 => f01,
                1 | 5 => false,
                2 | 6 => true,
                3 => f37,
                _ => bit(q, I37),
            };
            let into_q02 = match style {
                0 => bit(q, I01),
                1 => false,
                2 => true,
                3 => bit(q, I37),
                _ => f37,
            };
            fl_02 = Some(into_f02);
            // The rightmost slice drives QR 37 only while Q shifts down.
            qr_37 = shift_q.then_some(bit(q, I37));
            if shift_f {
                let mut v = f >> 1;
                v = with_bit(v, 0, bit(f, 0)); // ADL SIGN into F SIGN
                v = with_bit(v, I02, into_f02);
                new_b = Some(v);
            }
            if shift_q {
                let mut v = q >> 1;
                v = with_bit(v, 0, false);
                v = with_bit(v, I02, into_q02);
                new_q = Some(v);
            }
        }
        if !shift_f {
            new_b = match dest {
                0 | 1 => Some(f),
                _ => None,
            };
        }
        if dest == 2 {
            new_q = Some(f);
        }
        let mask = (if c.clock_left { LEFT } else { 0 }) | (if c.clock_right { RIGHT } else { 0 });
        if let Some(v) = new_b {
            let r = &mut self.reg[usize::from(c.b & 15)];
            *r = W40::wrap((r.get() & !mask) | (v & mask));
        }
        if let Some(v) = new_q {
            self.q = W40::wrap((q & !mask) | (v & mask));
        }
        Outputs {
            f: W40::wrap(f),
            y: W40::wrap(y),
            carry_out,
            carry_02,
            fl_02,
            qr_37,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl(func: u8, src: u8, dest: u8) -> Controls {
        Controls {
            func,
            src_left: src,
            src_right: src,
            dest,
            a: 1,
            b: 2,
            clock_left: true,
            clock_right: true,
            ..Controls::default()
        }
    }

    #[test]
    fn the_d_input_centres_a_word() {
        let v = d_input(W36::wrap(0o400000_000001));
        assert_eq!(v.get(), 0b11 << 38 | 0o400000_000001 << 2);
        assert_eq!(word(v), W36::wrap(0o400000_000001));
    }

    #[test]
    fn subtract_with_and_without_carry_in() {
        // AD/B-A-.25 (11): SUBR with R=A, S=B. Without carry in, the result
        // is 1 less in bit 37, a quarter of the word LSB.
        let mut u = Alu::new(W40::ZERO);
        u.reg[1] = d_input(W36::wrap(3));
        u.reg[2] = d_input(W36::wrap(10));
        let c = ctl(1, 1, 3);
        let o = u.step(&c, W40::ZERO);
        assert_eq!(o.f.get(), (7 << 2) - 1);
        assert_eq!(o.carry_out, Some(true));
        let o = u.step(
            &Controls {
                carry_in: true,
                ..c
            },
            W40::ZERO,
        );
        assert_eq!(word(o.f), W36::wrap(7));
    }

    #[test]
    fn half_clocks_and_the_bit_18_inhibit() {
        let mut u = Alu::new(W40::ZERO);
        u.reg[1] = d_input(W36::wrap(0o000000_777777));
        // B gets A + 1/4·4 (= A plus 1 at bit 35), right half only.
        let c = Controls {
            carry_in: true,
            inhibit_carry_18: true,
            ..ctl(0, 4, 1)
        };
        let o = u.step(&c, W40::ZERO);
        // The carry out of bit 37 is not the word carry: add 4 at bit 37.
        assert_eq!(word(o.f), W36::wrap(0o000000_777777));
        let c = Controls {
            func: 0,
            src_left: 5,
            src_right: 5,
            inhibit_carry_18: true,
            ..ctl(0, 5, 1)
        };
        let o = u.step(&c, d_input(W36::wrap(1)));
        assert_eq!(word(o.f), W36::wrap(0));
        assert!(o.adr_zero());
        let c = Controls {
            clock_left: false,
            ..ctl(0, 7, 1)
        };
        u.reg[2] = d_input(W36::wrap(0o123456_654321));
        u.step(&c, d_input(W36::wrap(0o777777_000000)));
        assert_eq!(word(u.reg[2]), W36::wrap(0o123456_000000));
    }

    #[test]
    fn a_logic_function_has_no_carry() {
        let mut u = Alu::new(W40::ZERO);
        let o = u.step(&ctl(3, 1, 3), W40::ZERO);
        assert_eq!(o.carry_out, None);
        assert_eq!(o.carry_02, None);
    }
}

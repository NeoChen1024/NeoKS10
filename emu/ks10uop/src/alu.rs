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

/// The outputs of one microinstruction. A shift line is `None` where
/// nothing drives it: the 2901 shift pins are inputs in the destinations
/// that do not shift, and the mixers of DPE1 drive one direction only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outputs {
    /// The function output F.
    pub f: W40,
    /// The chip output Y, which is DP.
    pub y: W40,
    /// CARRY OUT: out of bit −2 (DPE2 E709).
    pub carry_out: bool,
    /// CARRY 02: out of bit 02 into bit 01 (DPE2 E710).
    pub carry_02: bool,
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
    ///
    /// Each slice gives G and P by the table "Logic functions for G, P,
    /// Cn+4, and OVR" **[manual]** (The Am2900 Family Data Book, 1978,
    /// Am2901A, Figure 8). The 2902 chips of DPE2 combine them: the carry
    /// out of a slice is G + P·(its carry in), the carry into bit 37 is
    /// CARRY IN, and the carry from the slice of bits 18–21 into the slice
    /// of bits 14–17 goes through P2 of E711, which `SPEC/INHCRY18` turns
    /// off **[manual]** (M8620-0-DPE2 rev A1).
    fn function(&self, c: &Controls, d: W40) -> (u64, bool, bool) {
        let a = self.reg[usize::from(c.a & 15)].get();
        let b = self.reg[usize::from(c.b & 15)].get();
        let (rl, sl) = self.operands(c.src_left, a, b, d.get());
        let (rr, sr) = self.operands(c.src_right, a, b, d.get());
        let r = (rl & LEFT) | (rr & RIGHT);
        let s = (sl & LEFT) | (sr & RIGHT);
        let func = c.func & 7;
        // The R and S of the G and P definitions (Figure 8: substitute
        // the complement of R for S−R, R̄∧S and R⊻S, of S for R−S).
        let (gr, gs) = match func {
            1 | 5 | 6 => (!r & ALL, s),
            2 => (r, !s & ALL),
            _ => (r, s),
        };
        let mut f = 0u64;
        let mut carry = c.carry_in;
        let (mut carry_02, mut carry_out) = (false, false);
        // Slice 9 holds bits 34–37 (shift 0–3), slice 0 bits −2 to 01.
        for slice in (0..10u32).rev() {
            let sh = 4 * (9 - slice);
            let (ri, si) = ((gr >> sh) & 15, (gs >> sh) & 15);
            let (p, g) = (ri | si, ri & si);
            let pb = |i: u32| (p >> i) & 1 != 0;
            let gb = |i: u32| (g >> i) & 1 != 0;
            let lookahead = gb(3)
                || (pb(3) && gb(2))
                || (pb(3) && pb(2) && gb(1))
                || (pb(3) && pb(2) && pb(1) && gb(0));
            let (big_g, big_p) = match func {
                0..=2 => (lookahead, p == 15),
                3 => (p != 15, true),
                4 | 5 => (g != 0, true),
                // R⊽S (and R⊻S with R̄): the last term has P0, not G0.
                _ => {
                    let x = gb(3)
                        || (pb(3) && gb(2))
                        || (pb(3) && pb(2) && gb(1))
                        || (pb(3) && pb(2) && pb(1) && pb(0));
                    (!x, g == 0)
                }
            };
            let (x, y) = ((r >> sh) & 15, (s >> sh) & 15);
            let fi = match func {
                0 => x + y + u64::from(carry),
                1 => y + (!x & 15) + u64::from(carry),
                2 => x + (!y & 15) + u64::from(carry),
                3 => x | y,
                4 => x & y,
                5 => !x & y,
                6 => x ^ y,
                _ => !(x ^ y),
            } & 15;
            f |= fi << sh;
            let out = big_g || (big_p && carry);
            if slice == 1 {
                carry_02 = out;
            }
            if slice == 0 {
                carry_out = out;
            }
            carry = if slice == 5 {
                out && !c.inhibit_carry_18
            } else {
                out
            };
        }
        (f, carry_out, carry_02)
    }

    /// The output Y before the registers change, if it does not depend on
    /// D: `DEST/A` selects the A register, no source uses D, or the
    /// function gives the same Y for every D.
    pub fn y_without_d(&self, c: &Controls) -> Option<W40> {
        if c.dest & 7 == 0 {
            return Some(self.reg[usize::from(c.a & 15)]);
        }
        let zero = self.function(c, W40::ZERO).0;
        if uses_d(c.src_left) || uses_d(c.src_right) {
            // D is one operand of each function. A function of D and a
            // fixed operand gives the same result for D = 0 and D = all
            // ones only if it does not depend on D: OR with all ones, AND
            // with 0, and NOT R AND S with S = 0. The sums, differences and
            // exclusive ORs differ for the two values.
            let ones = self.function(c, W40::ONES).0;
            return (zero == ones).then_some(W40::wrap(zero));
        }
        Some(W40::wrap(zero))
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
            // The slice of bits 02–05 drives FL 02 (its RAM3 pin) only in
            // RAMQU and RAMU (Am2901A, Figure 4).
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
            // The rightmost slice drives QR 37 (its Q0 pin) in RAMQD and
            // RAMD (Am2901A, Figure 4): the field codes 6 and 7.
            qr_37 = shift_f.then_some(bit(q, I37));
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
    fn y_does_not_depend_on_d_if_the_function_is_constant() {
        let alu = Alu::new(W40::wrap(0o12345));
        // AND of D with zero (source DZ): Y is 0 for every D.
        let mut c = ctl(4, 7, 1);
        assert_eq!(alu.y_without_d(&c), Some(W40::ZERO));
        // OR of D with zero: Y is D.
        c.func = 3;
        assert_eq!(alu.y_without_d(&c), None);
        // D plus A (source DA).
        let c = ctl(0, 5, 1);
        assert_eq!(alu.y_without_d(&c), None);
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
        assert!(o.carry_out);
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
    fn the_carries_of_logic_functions_follow_the_data_sheet() {
        // OR: P = 1, G = NOT(P3P2P1P0). With R = 0 and S = 0 every slice
        // generates, so CARRY OUT is 1 whatever the carry in.
        let mut u = Alu::new(W40::ZERO);
        let o = u.step(&ctl(3, 1, 3), W40::ZERO);
        assert!(o.carry_out && o.carry_02);
        // AND: P = 1, G = OR of the bit generates. With all ones, G = 1.
        u.reg[1] = W40::ONES;
        u.reg[2] = W40::ONES;
        let o = u.step(&ctl(4, 1, 3), W40::ZERO);
        assert!(o.carry_out);
        // AND with no common bits and no carry in: no carry.
        u.reg[2] = W40::ZERO;
        let o = u.step(&ctl(4, 1, 3), W40::ZERO);
        assert!(!o.carry_out);
    }

    #[test]
    fn arithmetic_carries_agree_with_the_sum() {
        let mut u = Alu::new(W40::ZERO);
        for (x, y) in [(0o777777_777777u64, 1u64), (0o377777_777777, 1), (5, 7)] {
            u.reg[1] = d_input(W36::wrap(x));
            u.reg[2] = d_input(W36::wrap(y));
            let o = u.step(&ctl(0, 1, 3), W40::ZERO);
            let sum = u.reg[1].get() + u.reg[2].get();
            assert_eq!(o.f.get(), sum & ALL);
            assert_eq!(o.carry_out, sum >> 40 != 0);
        }
    }
}

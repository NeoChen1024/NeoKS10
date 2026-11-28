//! Double word instructions: DMOVE, DMOVN, DMOVEM, DMOVNM (120, 121, 124,
//! 125), DADD, DSUB, DMUL, DDIV (114 to 117), and ADJSP (105).
//!
//! The format (PRM 2.2): a double word has the sign in bit 0 of the high
//! word and 70 magnitude bits in bits 1 to 35 of both words; a quadruple
//! word has 140. In a result, bit 0 of each lower word is a copy of the
//! sign; in an operand it is not used. The behaviour follows the ITS
//! microcode **[microcode]** (simple.42: DMOVN, DBLNGA, DADD, DSUB, CPYSGN,
//! DMUL, DDIV, ADJSP).

use pdp10::{Half, W36};

use crate::flags::Flags;
use crate::machine::{Fault, Inst, Machine, Next};

type R = Result<Next, Fault>;

const MASK36: u64 = 0o777777_777777;
const MAG: u64 = 0o377777_777777;
const MAG70: u128 = (1 << 70) - 1;

fn ac(i: Inst, k: usize) -> usize {
    (i.ac + k) & 0o17
}

fn next(e: Half) -> Half {
    Half::wrap(e.get() + 1)
}

/// A double word as a 71-bit two's complement number.
fn value(hi: W36, lo: W36) -> i128 {
    (i128::from(hi.signed()) << 35) | i128::from(lo.get() & MAG)
}

/// A 71-bit number as a double word, with the sign copied into the low
/// word.
fn words(x: i128) -> (W36, W36) {
    let hi = ((x >> 35) as u64) & MASK36;
    let lo = ((x as u64) & MAG) | (hi & (1 << 35));
    (W36::wrap(hi), W36::wrap(lo))
}

/// A 36-bit addition with a carry in: the sum and the flags Carry 0 and
/// Carry 1 of the result (PRM 2.2; "AD FLAGS").
fn add36(a: u64, b: u64, cin: u64) -> (u64, bool, bool) {
    let cry1 = (a & MAG) + (b & MAG) + cin > MAG;
    let s = a + b + cin;
    (s & MASK36, s > MASK36, cry1)
}

fn set_arith(m: &mut Machine, cry0: bool, cry1: bool) {
    m.flags.arith(cry0, cry1);
}

pub fn execute(m: &mut Machine, i: Inst, e: Half) -> R {
    match i.op {
        0o105 => adjsp(m, i, e),
        0o114 | 0o115 => {
            let (bh, bl) = (m.read(e)?, m.read(next(e))?);
            let (ah, al) = (m.ac[i.ac], m.ac[ac(i, 1)]);
            let (hi, lo) = if i.op == 0o114 {
                // DADD: the carry from the low magnitudes goes into the
                // high word (SKIP CRY1).
                let (lo, _, c) = add36(al.get(), bl.get(), 0);
                let (hi, c0, c1) = add36(ah.get(), bh.get(), u64::from(c));
                set_arith(m, c0, c1);
                (hi, lo)
            } else {
                // DSUB: AC - E as AC + NOT E + 1, low word first.
                let (lo, _, c) = add36(al.get(), !bl.get() & MASK36, 1);
                let (hi, c0, c1) = add36(ah.get(), !bh.get() & MASK36, u64::from(c));
                set_arith(m, c0, c1);
                (hi, lo)
            };
            // CPYSGN: the low word gets the sign of the high word.
            let lo = (lo & MAG) | (hi & (1 << 35));
            m.ac[i.ac] = W36::wrap(hi);
            m.ac[ac(i, 1)] = W36::wrap(lo);
            Ok(Next::Step)
        }
        0o116 => dmul(m, i, e),
        0o117 => ddiv(m, i, e),
        0o120 => {
            let (h, l) = (m.read(e)?, m.read(next(e))?);
            m.ac[i.ac] = h;
            m.ac[ac(i, 1)] = l;
            Ok(Next::Step)
        }
        0o121 => {
            let (h, l) = (m.read(e)?, m.read(next(e))?);
            let (h, l) = negate(m, h, l);
            m.ac[i.ac] = h;
            m.ac[ac(i, 1)] = l;
            Ok(Next::Step)
        }
        0o124 | 0o125 => {
            let (mut h, mut l) = (m.ac[i.ac], m.ac[ac(i, 1)]);
            if i.op == 0o125 {
                (h, l) = negate(m, h, l);
            }
            // DMOVN1: E+1 first, then E.
            m.write(next(e), l)?;
            m.write(e, h)?;
            Ok(Next::Step)
        }
        _ => Err(Fault::Unimplemented),
    }
}

/// DMOVN, DMOVNM: the low word without its bit 0 is negated; if the result
/// is 0, the high word is negated (with the flags of 0 - high), else
/// complemented. Bit 0 of the low word becomes 0 (PRM, DMOVN: the flags
/// change only for a doubleword of 0 or -2^70).
fn negate(m: &mut Machine, h: W36, l: W36) -> (W36, W36) {
    let lo = (l.get() & MAG).wrapping_neg() & MASK36;
    let hi = if lo == 0 {
        let (s, c0, c1) = add36(0, !h.get() & MASK36, 1);
        set_arith(m, c0, c1);
        s
    } else {
        !h.get() & MASK36
    };
    (W36::wrap(hi), W36::wrap(lo & MAG))
}

/// ADJSP: AC + E,,E without a carry between the halves. Trap 2 if the
/// sign of AC changes the wrong way: from not negative to negative, or the
/// reverse (ADJSP1, ADJSP2; "SET PDL OV" sets only Trap 2).
fn adjsp(m: &mut Machine, i: Inst, e: Half) -> R {
    let a = m.ac[i.ac];
    let lh = Half::wrap(a.lh().get() + e.get());
    let rh = Half::wrap(a.rh().get() + e.get());
    let w = W36::from_halves(lh, rh);
    if a.bit(0) != w.bit(0) {
        m.flags.set(Flags::TRAP2);
    }
    m.ac[i.ac] = w;
    Ok(Next::Step)
}

/// A 140-bit magnitude: the high and the low 70 bits.
type Quad = (u128, u128);

fn mul70(a: u128, b: u128) -> Quad {
    let (a1, a0) = (a >> 35, a & MAG as u128);
    let (b1, b0) = (b >> 35, b & MAG as u128);
    let mid = a1 * b0 + a0 * b1;
    let lo = a0 * b0 + ((mid & MAG as u128) << 35);
    let hi = a1 * b1 + (mid >> 35) + (lo >> 70);
    (hi, lo & MAG70)
}

/// The two's complement of a 141-bit number (sign and 140 bits).
fn neg_quad((hi, lo): Quad) -> Quad {
    let lo2 = (!lo).wrapping_add(1) & MAG70;
    let carry = u128::from(lo == 0);
    let hi2 = (!hi).wrapping_add(carry) & MAG70;
    (hi2, lo2)
}

/// A 141-bit number as 4 words: the sign in bit 0 of each.
fn quad_words(sign: bool, (hi, lo): Quad) -> [W36; 4] {
    let s = u64::from(sign) << 35;
    let d = |x: u128| W36::wrap(s | (x as u64 & MAG));
    [d(hi >> 35), d(hi), d(lo >> 35), d(lo)]
}

fn dmul(m: &mut Machine, i: Inst, e: Half) -> R {
    let (bh, bl) = (m.read(e)?, m.read(next(e))?);
    let (ah, al) = (m.ac[i.ac], m.ac[ac(i, 1)]);
    let (a, b) = (value(ah, al), value(bh, bl));
    let p = mul70(a.unsigned_abs(), b.unsigned_abs());
    let negative = (a < 0) != (b < 0) && p != (0, 0);
    let (sign, q) = if p.0 >> 70 != 0 {
        // Only (-2^70)^2: the magnitude 2^140 does not fit. Overflow and
        // Trap 1 (PRM, DMUL).
        m.flags.set(Flags::AROV | Flags::TRAP1);
        (true, (0, 0))
    } else if negative {
        (true, neg_quad(p))
    } else {
        (false, p)
    };
    for (k, w) in quad_words(sign, q).into_iter().enumerate() {
        m.ac[ac(i, k)] = w;
    }
    // DMTRAP: Trap 1 if Overflow is set, also from an earlier instruction
    // (simple.42, DMTRAP; design decision 2026-11-28, case
    // dmul-trap-old-overflow of spec/impldef.toml).
    if m.flags.has(Flags::AROV) {
        m.flags.set(Flags::TRAP1);
    }
    Ok(Next::Step)
}

fn ddiv(m: &mut Machine, i: Inst, e: Half) -> R {
    let d = value(m.read(e)?, m.read(next(e))?);
    let w: Vec<W36> = (0..4).map(|k| m.ac[ac(i, k)]).collect();
    let sign = w[0].bit(0);
    let digit = |k: usize| u128::from(w[k].get() & MAG);
    // The dividend as sign and 140 bits of two's complement.
    let hi = (u128::from(w[0].get() & MAG) << 35) | digit(1);
    let lo = (digit(2) << 35) | digit(3);
    let mag = if sign { neg_quad((hi, lo)) } else { (hi, lo) };
    // The magnitude of -2^140 is 2^140: its high part is 2^70.
    let mag_hi = if sign && mag == (0, 0) {
        1 << 70
    } else {
        mag.0
    };
    let dv = d.unsigned_abs();
    if dv == 0 || mag_hi >= dv {
        m.flags.set(Flags::AROV | Flags::TRAP1 | Flags::NODIV);
        return Ok(Next::Step);
    }
    let (mut r, mut q) = (mag_hi, 0u128);
    for k in (0..70).rev() {
        r = (r << 1) | ((mag.1 >> k) & 1);
        if r >= dv {
            r -= dv;
            q |= 1 << k;
        }
    }
    let qs = if sign != (d < 0) {
        -(q as i128)
    } else {
        q as i128
    };
    let rs = if sign { -(r as i128) } else { r as i128 };
    let (q0, q1) = words(qs);
    let (r0, r1) = words(rs);
    for (k, x) in [q0, q1, r0, r1].into_iter().enumerate() {
        m.ac[ac(i, k)] = x;
    }
    Ok(Next::Step)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(acs: &[u64], mem: &[u64]) -> Machine {
        let mut m = Machine::new();
        for (k, &w) in acs.iter().enumerate() {
            m.ac[1 + k] = W36::wrap(w);
        }
        for (k, &w) in mem.iter().enumerate() {
            m.mem[0o100 + k] = W36::wrap(w);
        }
        m
    }

    fn run(m: &mut Machine, op: u64) {
        let i = Inst::new(W36::wrap((op << 27) | (1 << 23) | 0o100));
        execute(m, i, Half::wrap(0o100)).unwrap();
    }

    fn acs(m: &Machine, n: usize) -> Vec<u64> {
        (1..=n).map(|k| m.ac[k].get()).collect()
    }

    const N: u64 = MASK36;

    // Expected values from PRM 2.2 and the DMOVN text (PRM, DMOVN: "If the
    // memory doubleword is fixed point -2^70, set the Trap 1, Overflow and
    // Carry 1 flags ... If the memory doubleword is zero, set Carry 0 and
    // Carry 1"), computed by hand.
    #[test]
    fn dmovn_flags() {
        let mut m = machine(&[], &[0, 1]);
        run(&mut m, 0o121);
        assert_eq!(acs(&m, 2), [N, MAG]); // -1: low word bit 0 is 0
        assert_eq!(m.flags.0, 0);
        let mut m = machine(&[], &[0, 0]);
        run(&mut m, 0o121);
        assert_eq!(acs(&m, 2), [0, 0]);
        assert!(m.flags.has(Flags::CRY0) && m.flags.has(Flags::CRY1));
        let mut m = machine(&[], &[1 << 35, 0]);
        run(&mut m, 0o121);
        assert_eq!(acs(&m, 2), [1 << 35, 0]);
        assert!(m.flags.has(Flags::AROV) && m.flags.has(Flags::CRY1));
        assert!(!m.flags.has(Flags::CRY0));
    }

    // PRM, DADD: "If the sum is >= 2^70, set Trap 1, Overflow and Carry 1;
    // the result stored has a minus sign".
    #[test]
    fn dadd_dsub() {
        let mut m = machine(&[0, MAG], &[0, 1]);
        run(&mut m, 0o114);
        assert_eq!(acs(&m, 2), [1, 0]);
        let mut m = machine(&[MAG, MAG], &[0, 1]);
        run(&mut m, 0o114);
        assert_eq!(acs(&m, 2), [1 << 35, 1 << 35]);
        assert!(m.flags.has(Flags::AROV) && m.flags.has(Flags::CRY1));
        // 0 - 1 = -1: both words all ones.
        let mut m = machine(&[0, 0], &[0, 1]);
        run(&mut m, 0o115);
        assert_eq!(acs(&m, 2), [N, N]);
    }

    #[test]
    fn dmul_cases() {
        // (2^35) * (2^35) = 2^70: words 0, 1, 0, 0.
        let mut m = machine(&[1, 0], &[1, 0]);
        run(&mut m, 0o116);
        assert_eq!(acs(&m, 4), [0, 1, 0, 0]);
        // -1 * 3 = -3: all ones except the low word.
        let mut m = machine(&[N, N], &[0, 3]);
        run(&mut m, 0o116);
        assert_eq!(acs(&m, 4), [N, N, N, N - 2]);
        // (-2^70)^2 overflows: Overflow and Trap 1, and no carry flag
        // (PRM, DMUL).
        let mut m = machine(&[1 << 35, 0], &[1 << 35, 0]);
        run(&mut m, 0o116);
        assert_eq!(m.flags.0, Flags::AROV | Flags::TRAP1);
        // Other products set no flag.
        let mut m = machine(&[N, 2], &[0o377777_777777, 0o123456_654321]);
        run(&mut m, 0o116);
        assert_eq!(m.flags.0, 0);
        // DMUL ends with DMTRAP: Trap 1 if Overflow is set, also from an
        // earlier instruction (simple.42, DMTRAP; docs/discrepancies.md,
        // "Trap 1 and Trap 2: each flag word loads them").
        let mut m = machine(&[0, 2], &[0, 3]);
        m.flags = Flags(Flags::AROV);
        run(&mut m, 0o116);
        assert_eq!(m.flags.0, Flags::AROV | Flags::TRAP1);
    }

    #[test]
    fn ddiv_cases() {
        // 2^70 / 2^35 = 2^35 remainder 0.
        let mut m = machine(&[0, 1, 0, 0], &[1, 0]);
        run(&mut m, 0o117);
        assert_eq!(acs(&m, 4), [1, 0, 0, 0]);
        // -7 / 2 = -3 remainder -1.
        let mut m = machine(&[N, N, N, N - 6], &[0, 2]);
        run(&mut m, 0o117);
        assert_eq!(acs(&m, 4), [N, N - 2, N, N]);
        // The high double word not below the divisor: No Divide.
        let mut m = machine(&[0, 2, 0, 0], &[0, 2]);
        run(&mut m, 0o117);
        assert!(m.flags.has(Flags::NODIV));
        assert_eq!(acs(&m, 4), [0, 2, 0, 0]);
    }

    #[test]
    fn adjsp_trap2() {
        let mut m = machine(&[0o777776_001000], &[]);
        let i = Inst::new(W36::wrap(0o105040_000003));
        execute(&mut m, i, Half::wrap(3)).unwrap();
        assert_eq!(m.ac[1].get(), 0o000001_001003);
        assert!(m.flags.has(Flags::TRAP2));
    }
}

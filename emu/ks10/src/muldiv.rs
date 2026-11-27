//! IMUL, MUL, IDIV, DIV (220 to 237; PRM 2.2, "Fixed Point
//! Arithmetic").
//!
//! The results and flags follow the ITS microcode **[microcode]**
//! (simple.42, "ARITHMETIC -- MUL, IMUL" and "ARITHMETIC -- DIV, IDIV").
//! A double word has its sign in bit 0 of the high word and 70 magnitude
//! bits; bit 0 of the low word is a copy of the sign in a result and is
//! not used in an operand.

use pdp10::{Half, W36};

use crate::flags::Flags;
use crate::machine::{Fault, Inst, Machine, Next};

type R = Result<Next, Fault>;

const MASK36: i128 = (1 << 36) - 1;
const MAG: i128 = (1 << 35) - 1;

fn next_ac(ac: usize) -> usize {
    (ac + 1) & 0o17
}

/// The operand: C(E), or 0,,E for the immediate mode (bits 7 and 8 = 1).
fn operand(m: &mut Machine, i: Inst, e: Half) -> Result<W36, Fault> {
    if i.op & 3 == 1 {
        Ok(W36::from_halves(Half::ZERO, e))
    } else {
        Ok(m.read(e)?)
    }
}

/// A 71-bit two's complement number as a double word: the high 36 bits,
/// and the sign with the low 35 bits.
fn split(x: i128) -> (W36, W36) {
    let hi = (x >> 35) & MASK36;
    let sign = (x >> 70) & 1;
    let lo = (sign << 35) | (x & MAG);
    (W36::wrap(hi as u64), W36::wrap(lo as u64))
}

/// Store by the mode: 0, 1 to AC (and AC+1 for a double result), 2 to E
/// (the high or only word), 3 to both.
fn store(m: &mut Machine, i: Inst, e: Half, w: W36, w2: Option<W36>) -> R {
    let mode = i.op & 3;
    if mode == 2 || mode == 3 {
        m.write(e, w)?;
    }
    if mode != 2 {
        m.ac[i.ac] = w;
        if let Some(w2) = w2 {
            m.ac[next_ac(i.ac)] = w2;
        }
    }
    Ok(Next::Step)
}

pub fn execute(m: &mut Machine, i: Inst, e: Half) -> R {
    let b = operand(m, i, e)?;
    let a = m.ac[i.ac];
    match i.op & 0o14 {
        0o00 => {
            // IMUL: the sign of the true product and its low 35 bits;
            // Overflow if the product does not fit (IMUL2, IMUL3).
            let p = i128::from(a.signed()) * i128::from(b.signed());
            let fits = (-(1 << 35)..1 << 35).contains(&p);
            if !fits {
                m.flags.set(Flags::AROV | Flags::TRAP1);
            }
            let sign = i128::from(p < 0);
            let w = W36::wrap(((sign << 35) | (p & MAG)) as u64);
            store(m, i, e, w, None)
        }
        0o04 => {
            // MUL: the 70-bit product; only the square of -2^35 overflows.
            let p = i128::from(a.signed()) * i128::from(b.signed());
            if p == 1 << 70 {
                m.flags.set(Flags::AROV | Flags::TRAP1);
            }
            let (hi, lo) = split(p);
            store(m, i, e, hi, Some(lo))
        }
        op => {
            let (d, check) = if op == 0o10 {
                // IDIV: AC with its sign extended.
                (i128::from(a.signed()), false)
            } else {
                let lo = i128::from(m.ac[next_ac(i.ac)].get()) & MAG;
                let ah = a.get();
                // DIV tests the size of the quotient only if the high word
                // is not all sign bits.
                let signs = ah == 0 || ah == 0o777777_777777;
                ((i128::from(a.signed()) << 35) | lo, !signs)
            };
            if b == W36::ZERO || (check && too_big(d, b)) {
                m.flags.set(Flags::AROV | Flags::TRAP1 | Flags::NODIV);
                return Ok(Next::Step);
            }
            let bs = i128::from(b.signed());
            let (q, r) = (d / bs, d % bs);
            let q = W36::wrap((q & MASK36) as u64);
            let r = W36::wrap((r & MASK36) as u64);
            store(m, i, e, q, Some(r))
        }
    }
}

/// The no-divide test of DIV (DIVB, DIVC, NODIV) in the 36-bit arithmetic
/// of the microcode: the high word of |dividend| minus |divisor| (or plus
/// a negative divisor) is not negative.
fn too_big(d: i128, b: W36) -> bool {
    let h = ((d.abs() >> 35) & MASK36) as u64;
    let t = if b.bit(0) {
        h.wrapping_add(b.get())
    } else {
        h.wrapping_sub(b.get())
    };
    t & (1 << 35) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(op: u64, a: u64, a1: u64, b: u64) -> (u64, u64, Flags) {
        let mut m = Machine::new();
        m.ac[1] = W36::wrap(a);
        m.ac[2] = W36::wrap(a1);
        m.mem[0o100] = W36::wrap(b);
        let i = Inst::new(W36::wrap((op << 27) | (1 << 23) | 0o100));
        execute(&mut m, i, Half::wrap(0o100)).unwrap();
        (m.ac[1].get(), m.ac[2].get(), m.flags)
    }

    const N: u64 = 0o777777_777777; // -1

    // Expected values from the PRM 2.2 definitions, computed by hand.
    #[test]
    fn imul_mul() {
        assert_eq!(run(0o220, 3, 0, N - 1).0, N - 5); // 3 * -2 = -6
        // 2^34 * 4 = 2^36: overflow, positive sign, low 35 bits 0.
        let (w, _, f) = run(0o220, 1 << 34, 0, 4);
        assert_eq!(w, 0);
        assert!(f.has(Flags::AROV) && f.has(Flags::TRAP1));
        // -2^35 * 1 fits.
        let (w, _, f) = run(0o220, 1 << 35, 0, 1);
        assert_eq!((w, f.has(Flags::AROV)), (1 << 35, false));
        // MUL 2^35-1 squared = 2^70 - 2^36 + 1.
        let (h, l, _) = run(0o224, MAG as u64, 0, MAG as u64);
        assert_eq!((h, l), (0o377777_777776, 1));
        // MUL -1 * 1: all ones high, sign copy in the low word.
        assert_eq!(run(0o224, N, 0, 1), (N, N, Flags(0)));
        // MUL -2^35 squared overflows: 400000,,0 in both words.
        let (h, l, f) = run(0o224, 1 << 35, 0, 1 << 35);
        assert_eq!((h, l), (1 << 35, 1 << 35));
        assert!(f.has(Flags::AROV));
    }

    #[test]
    fn idiv_div() {
        // -7 / 2 = -3 remainder -1 (the sign of the dividend).
        assert_eq!(run(0o230, N - 6, 0, 2), (N - 2, N, Flags(0)));
        // PRM 2-14: on the KS10, -2^35 / -1 gives -2^35 without an error.
        assert_eq!(run(0o230, 1 << 35, 0, N), (1 << 35, 0, Flags(0)));
        // Division by 0: No Divide, AC does not change.
        let (q, _, f) = run(0o230, 5, 0, 0);
        assert_eq!(q, 5);
        assert!(f.has(Flags::NODIV) && f.has(Flags::AROV) && f.has(Flags::TRAP1));
        // DIV 1,,0 (as a double word: 2^35 * 1 + 0) / 2 = 2^34.
        assert_eq!(run(0o234, 1, 0, 2), (1 << 34, 0, Flags(0)));
        // DIV with high word 1 / 1: quotient 2^35 does not fit.
        assert!(run(0o234, 1, 0, 1).2.has(Flags::NODIV));
        // The sign bit of AC+1 is not used.
        assert_eq!(run(0o234, 0, (1 << 35) | 10, 3), (3, 1, Flags(0)));
    }
}

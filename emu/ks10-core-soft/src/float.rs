//! Floating point: FAD, FSB, FMP, FDV with their modes (140 to 177),
//! FSC (132), FIX (122), FIXR (126), FLTR (127), and DFAD, DFSB, DFMP,
//! DFDV (110 to 113).
//!
//! Sources, in this order:
//!
//! - **[microcode]** flt.5 (FAD to DFDV, SNORM, SNNORM, SNNOT, DNORM,
//!   DNNORM): the operand formats, the alignment of FAD in AR!Q, the
//!   no-divide tests, and the form of a negative result.
//! - **[manual]** PRM 1982, 2.3 "Floating Point Arithmetic": rounding,
//!   the exponent overflow and underflow flags, FIX and FIXR, FLTR, FSC,
//!   and the note that a negative FDV quotient is a ones complement if the
//!   remainder is not zero.
//! - **[emulator]** KLH10 6d733f2 (`kn10ops.c`, `ffadr`, `op10xfix`):
//!   FIXR rounds by adding 1/2 (+1.5 gives 2, -1.5 gives -1); rounding of
//!   a negative result away from zero at exactly 1/2.
//!
//! An operand is taken apart as the microcode does it ("SIGN SMEAR"): the
//! exponent is bits 1 to 8, complemented if bit 0 is set, and the fraction
//! is the word with bits 1 to 8 replaced by copies of bit 0, a two's
//! complement number with 27 fraction bits (62 in double precision).
//!
//! The result is formed from its magnitude: normalize it to 27 (or 62)
//! bits, round if the instruction rounds (add 1/2 LSB to the magnitude),
//! put the exponent in, and negate the word if the result is negative. In
//! a single precision instruction without rounding, a negative result is
//! the ones complement of the positive word if any bit was dropped
//! (SNNOT), so the result is the next lower number (it is truncated
//! toward minus infinity).

use pdp10::{Half, W36};

use crate::flags::Flags;
use crate::machine::{Fault, Inst, Machine, Next};
use crate::shift;

type R = Result<Next, Fault>;

const MASK36: u64 = 0o777777_777777;
const MAG: u64 = 0o377777_777777;

/// Exponent and two's complement fraction of a single precision word.
fn unpack(w: W36) -> (i32, i128) {
    let x = w.get();
    let neg = w.bit(0);
    let e = ((x >> 27) & 0o377) as i32;
    let e = if neg { e ^ 0o377 } else { e };
    let f = (x & 0o777777777) as i128;
    let f = if neg { f - (1 << 27) } else { f };
    (e, f)
}

/// Exponent and two's complement fraction (62 bits) of a double word.
fn unpack2(hi: W36, lo: W36) -> (i32, i128) {
    let (e, f) = unpack(hi);
    (e, (f << 35) | i128::from(lo.get() & MAG))
}

/// A result before normalization: sign, magnitude, the number of fraction
/// bits in the magnitude, the exponent (excess 128), and whether nonzero
/// bits were lost before (only for the ones complement rule).
struct Raw {
    neg: bool,
    mag: u128,
    fbits: i32,
    exp: i32,
    sticky: bool,
}

impl Raw {
    fn from_signed(s: i128, fbits: i32, exp: i32) -> Raw {
        Raw {
            neg: s < 0,
            mag: s.unsigned_abs(),
            fbits,
            exp,
            sticky: false,
        }
    }
}

/// Normalize to `prec` bits and round if asked. Returns the magnitude, the
/// exponent and whether bits were dropped, or `None` for a zero result.
fn normalize(r: &Raw, prec: u32, round: bool) -> Option<(u128, i32, bool)> {
    if r.mag == 0 {
        return None;
    }
    let bl = 128 - r.mag.leading_zeros() as i32;
    let mut e = r.exp + bl - r.fbits;
    let shift = bl - prec as i32;
    let (mut t, rbit, dropped) = if shift > 0 {
        let s = shift as u32;
        let t = r.mag >> s;
        let rbit = (r.mag >> (s - 1)) & 1 == 1;
        let dropped = r.mag & ((1u128 << s) - 1) != 0;
        (t, rbit, dropped)
    } else {
        (r.mag << (-shift) as u32, false, false)
    };
    if round && rbit {
        t += 1;
        if t == 1 << prec {
            t >>= 1;
            e += 1;
        }
    }
    Some((t, e, dropped || r.sticky))
}

/// The exponent test of FL-EXIT (EXP TEST): above 255 sets Overflow,
/// Floating Overflow and Trap 1; below 0 also Floating Underflow. The
/// stored exponent is the low 8 bits.
fn exp_test(m: &mut Machine, e: i32) -> u64 {
    // EXP TEST loads Trap 1 with no hold term, and keeps Trap 2 (DPE9 E78;
    // [`Flags::change`]).
    m.flags.clear(Flags::TRAP1);
    if !(0..=0o377).contains(&e) {
        m.flags.set(Flags::AROV | Flags::FOV | Flags::TRAP1);
        if e < 0 {
            m.flags.set(Flags::FXU);
        }
    }
    (e & 0o377) as u64
}

/// The single precision result word.
fn single(m: &mut Machine, r: &Raw, round: bool) -> W36 {
    let Some((t, e, dropped)) = normalize(r, 27, round) else {
        return W36::ZERO;
    };
    let pos = (exp_test(m, e) << 27) | t as u64;
    let w = if !r.neg {
        pos
    } else if !round && dropped {
        !pos & MASK36
    } else {
        pos.wrapping_neg() & MASK36
    };
    W36::wrap(w)
}

/// The double precision result (always rounded; DNNORM, DNORM2: the low
/// word has bit 0 clear).
fn double(m: &mut Machine, r: &Raw) -> (W36, W36) {
    let Some((t, e, _)) = normalize(r, 62, true) else {
        return (W36::ZERO, W36::ZERO);
    };
    let hi = (exp_test(m, e) << 27) | (t >> 35) as u64;
    let lo = t as u64 & MAG;
    if !r.neg {
        return (W36::wrap(hi), W36::wrap(lo));
    }
    let nlo = lo.wrapping_neg() & MAG;
    let nhi = if nlo == 0 {
        hi.wrapping_neg() & MASK36
    } else {
        !hi & MASK36
    };
    (W36::wrap(nhi), W36::wrap(nlo))
}

/// The non-AC operand: C(E), or E,,0 in the immediate mode (FL-I: the
/// rounding instructions with bits 7 and 8 = 01).
fn operand(m: &mut Machine, i: Inst, e: Half) -> Result<W36, Fault> {
    if i.op & 0o7 == 0o5 {
        Ok(W36::from_halves(e, Half::ZERO))
    } else {
        Ok(m.read(e)?)
    }
}

fn store(m: &mut Machine, i: Inst, e: Half, w: W36) -> R {
    match i.op & 3 {
        2 => m.write(e, w)?,
        3 => {
            m.write(e, w)?;
            m.ac[i.ac] = w;
        }
        _ => m.ac[i.ac] = w,
    }
    Ok(Next::Step)
}

fn no_divide(m: &mut Machine) -> R {
    m.flags
        .change(Flags::AROV | Flags::FOV | Flags::NODIV | Flags::TRAP1);
    Ok(Next::Step)
}

pub fn execute(m: &mut Machine, i: Inst, e: Half) -> R {
    match i.op {
        0o110..=0o113 => dfloat(m, i, e),
        0o122 | 0o126 => fix(m, i, e),
        0o127 => {
            // FLTR: the integer, normalized and rounded.
            let x = m.read(e)?;
            let r = Raw::from_signed(i128::from(x.signed()), 0, 128);
            m.ac[i.ac] = single(m, &r, true);
            Ok(Next::Step)
        }
        0o132 => {
            // FSC: add the scale (the shift count of E) to the exponent
            // and normalize; no rounding (Q is 0).
            let (ex, f) = unpack(m.ac[i.ac]);
            let r = Raw::from_signed(f, 27, ex + shift::count(e));
            m.ac[i.ac] = single(m, &r, false);
            Ok(Next::Step)
        }
        0o140..=0o177 => {
            let round = i.op & 4 != 0;
            let b = operand(m, i, e)?;
            let a = m.ac[i.ac];
            let w = match i.op & 0o70 {
                0o40 | 0o50 => fad(m, a, b, i.op & 0o70 == 0o50, round),
                0o60 => fmp(m, a, b, round),
                _ => match fdv(m, a, b, round) {
                    Some(w) => w,
                    None => return no_divide(m),
                },
            };
            store(m, i, e, w)
        }
        _ => Err(Fault::Unimplemented),
    }
}

/// FAD, FSB (FAS1 to FAS4): the operand with the smaller exponent is
/// shifted right in AR!Q, 27 + 35 fraction bits, as a two's complement
/// number (ASHC: the bits that fall out are lost, so a negative operand
/// keeps all ones). There is no limit on the shift.
fn fad(m: &mut Machine, a: W36, b: W36, sub: bool, round: bool) -> W36 {
    let (ea, fa) = unpack(a);
    let (eb, mut fb) = unpack(b);
    if sub {
        fb = -fb;
    }
    let (eh, fh, el, fl) = if ea >= eb {
        (ea, fa, eb, fb)
    } else {
        (eb, fb, ea, fa)
    };
    let d = (eh - el).min(100) as u32;
    let s = (fh << 35) + ((fl << 35) >> d);
    single(m, &Raw::from_signed(s, 62, eh), round)
}

/// FMP: the exact product of the fractions (54 bits).
fn fmp(m: &mut Machine, a: W36, b: W36, round: bool) -> W36 {
    let (ea, fa) = unpack(a);
    let (eb, fb) = unpack(b);
    single(m, &Raw::from_signed(fa * fb, 54, ea + eb - 128), round)
}

/// FDV: `None` for no divide (FDV, FDV4, FDV5: a zero divisor word, or
/// |AC fraction| >= 2 |divisor fraction|). The quotient has 27 bits, 28
/// for FDVR (PRM, FDVR); it is computed from the magnitudes. A dividend
/// with a fraction at least the divisor fraction is first halved (FDV6,
/// FDV7: "IS ANSWER .LE. 1?").
fn fdv(m: &mut Machine, a: W36, b: W36, round: bool) -> Option<W36> {
    if b == W36::ZERO {
        return None;
    }
    let (ea, fa) = unpack(a);
    let (eb, fb) = unpack(b);
    let (na, nb) = (fa.unsigned_abs(), fb.unsigned_abs());
    if na >= 2 * nb {
        return None;
    }
    let mut exp = ea - eb + 128;
    let bits = if round { 28 } else { 27 };
    // The quotient fraction is below 1 after the halving.
    let den = if na >= nb {
        exp += 1;
        2 * nb
    } else {
        nb
    };
    let fbits = bits;
    let num = na << bits;
    let q = num / den;
    let rem = num % den != 0;
    let neg = (fa < 0) != (fb < 0);
    let r = if round {
        // Round with the extra bit, then normalize (PRM, FDVR).
        let q = (q + 1) >> 1;
        Raw {
            neg,
            mag: q,
            fbits: fbits - 1,
            exp,
            sticky: false,
        }
    } else {
        Raw {
            neg,
            mag: q,
            fbits,
            exp,
            sticky: rem,
        }
    };
    Some(single(m, &r, false))
}

/// FIX, FIXR (FIX, FIXR, FIXX, FIXT): the exponent test, then the
/// fraction shifted so that the binary point is right of bit 35. FIX
/// truncates toward 0; FIXR adds 1/2 and truncates toward minus infinity.
fn fix(m: &mut Machine, i: Inst, e: Half) -> R {
    let (ex, f) = unpack(m.read(e)?);
    if ex > 128 + 35 {
        m.flags.change(Flags::AROV | Flags::TRAP1);
        return Ok(Next::Step);
    }
    let n = ex - 128 - 27;
    let v = if n >= 0 {
        f << n
    } else {
        let k = (-n).min(100) as u32;
        if i.op == 0o126 {
            (f + (1i128 << (k - 1))) >> k
        } else if f < 0 {
            -((-f) >> k)
        } else {
            f >> k
        }
    };
    m.ac[i.ac] = W36::wrap((v as u64) & MASK36);
    Ok(Next::Step)
}

/// DFAD, DFSB, DFMP, DFDV. The sums are formed with 60 guard bits below
/// the 62 fraction bits, which keeps every bit that the rounding of the
/// microcode (2 guard bits and a sticky bit, DFADJ) can see.
fn dfloat(m: &mut Machine, i: Inst, e: Half) -> R {
    let next = Half::wrap(e.get() + 1);
    let (eb, mut fb) = unpack2(m.read(e)?, m.read(next)?);
    let (ea, fa) = unpack2(m.ac[i.ac], m.ac[(i.ac + 1) & 0o17]);
    let r = match i.op {
        0o110 | 0o111 => {
            if i.op == 0o111 {
                fb = -fb;
            }
            let (eh, fh, el, fl) = if ea >= eb {
                (ea, fa, eb, fb)
            } else {
                (eb, fb, ea, fa)
            };
            const G: u32 = 60;
            let d = (eh - el).min(126) as u32;
            let s = (fh << G) + ((fl << G) >> d);
            Raw::from_signed(s, 62 + G as i32, eh)
        }
        0o112 => Raw::from_signed(fa * fb, 124, ea + eb - 128),
        _ => {
            // DFDV: no divide as FDV; a quotient of 63 bits, the last for
            // rounding (PRM, DFDV).
            let (na, nb) = (fa.unsigned_abs(), fb.unsigned_abs());
            if nb == 0 || na >= 2 * nb {
                return no_divide(m);
            }
            let mut exp = ea - eb + 128;
            let den = if na >= nb {
                exp += 1;
                2 * nb
            } else {
                nb
            };
            let fbits = 63;
            let q = div_shift(na, den, 63);
            Raw {
                neg: (fa < 0) != (fb < 0),
                mag: q,
                fbits,
                exp,
                sticky: false,
            }
        }
    };
    let (h, l) = double(m, &r);
    m.ac[i.ac] = h;
    m.ac[(i.ac + 1) & 0o17] = l;
    Ok(Next::Step)
}

/// floor(a * 2^k / b) for a < b, by long division (a * 2^63 does not fit
/// in 128 bits for 62-bit fractions).
fn div_shift(a: u128, b: u128, k: u32) -> u128 {
    let (mut q, mut r) = (a / b, a % b);
    for _ in 0..k {
        r <<= 1;
        q <<= 1;
        if r >= b {
            r -= b;
            q |= 1;
        }
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(op: u64, a: u64, b: u64) -> (u64, Flags) {
        let mut m = Machine::new();
        m.ac[1] = W36::wrap(a);
        m.mem[0o100] = W36::wrap(b);
        let i = Inst::new(W36::wrap((op << 27) | (1 << 23) | 0o100));
        execute(&mut m, i, Half::wrap(0o100)).unwrap();
        (m.ac[1].get(), m.flags)
    }

    // Values of the PRM 1.4 format, computed by hand: 1.0 = 201400,,0,
    // 2.0 = 202400,,0, 0.5 = 200400,,0, -1.0 = 576400,,0, 3.0 = 202600,,0.
    const ONE: u64 = 0o201400_000000;
    const TWO: u64 = 0o202400_000000;
    const HALF: u64 = 0o200400_000000;
    const M_ONE: u64 = 0o576400_000000;

    #[test]
    fn add_sub() {
        assert_eq!(run(0o140, ONE, ONE).0, TWO);
        assert_eq!(run(0o140, TWO, ONE).0, 0o202600_000000);
        assert_eq!(run(0o150, ONE, ONE).0, 0);
        assert_eq!(run(0o140, ONE, M_ONE).0, 0);
        assert_eq!(run(0o150, ONE, TWO).0, M_ONE);
        assert_eq!(run(0o140, HALF, HALF).0, ONE);
    }

    #[test]
    fn mul_div() {
        assert_eq!(run(0o160, TWO, HALF).0, ONE);
        assert_eq!(run(0o160, M_ONE, TWO).0, 0o575400_000000); // -2.0
        assert_eq!(run(0o170, ONE, TWO).0, HALF);
        assert_eq!(run(0o170, TWO, ONE).0, TWO);
        // 1/3 = 0.0101..., 27 bits: 177525252525 truncated.
        assert_eq!(run(0o170, ONE, 0o202600_000000).0, 0o177525_252525);
        // Rounded: the 28th bit is 0 (0.0101..), so no change.
        assert_eq!(run(0o174, ONE, 0o202600_000000).0, 0o177525_252525);
        // 1/7 = 0.001001..: the 28th bit is 1, so FDVR rounds up.
        let seven = 0o203700_000000;
        assert_eq!(run(0o170, ONE, seven).0, 0o176444_444444);
        assert_eq!(run(0o174, ONE, seven).0, 0o176444_444445);
        // Divide by zero.
        let (w, f) = run(0o170, ONE, 0);
        assert_eq!(w, ONE);
        assert!(f.has(Flags::NODIV) && f.has(Flags::FOV) && f.has(Flags::AROV));
    }

    // PRM 2.3: an exponent above 127 sets Overflow, Floating Overflow and
    // Trap 1 and stores the exponent 256 less.
    // DSKCB, tests CB11201 and CB11202: a quotient just above 1 is
    // rounded with the 28th bit of the halved quotient.
    #[test]
    fn fdvr_quotient_above_one() {
        assert_eq!(
            run(0o174, 0o200777_777777, 0o200777_777776).0,
            0o201400_000001
        );
        assert_eq!(
            run(0o174, 0o200777_777777, 0o200777_777774).0,
            0o201400_000002
        );
    }

    #[test]
    fn overflow() {
        let big = 0o377400_000000; // 0.5 * 2^127
        let (w, f) = run(0o140, big, big);
        assert_eq!(w, 0o000400_000000);
        assert!(f.has(Flags::FOV) && f.has(Flags::AROV) && f.has(Flags::TRAP1));
        assert!(!f.has(Flags::FXU));
        let tiny = 0o000400_000000; // 0.5 * 2^-128
        let (w, f) = run(0o160, tiny, tiny);
        // 0.5 * 2^-257: the exponent -129 is stored as 127.
        assert_eq!(w, 0o177400_000000);
        assert!(f.has(Flags::FXU));
    }

    // FIX, FIXR (PRM 2-28; KLH10 op10xfix: +1.5 -> 2, -1.5 -> -1).
    #[test]
    fn fix_fixr() {
        let one_half = 0o201600_000000; // 1.5
        let m_one_half = 0o576200_000000; // -1.5
        assert_eq!(run(0o122, 0, one_half).0, 1);
        assert_eq!(run(0o122, 0, m_one_half).0, MASK36); // -1
        assert_eq!(run(0o126, 0, one_half).0, 2);
        assert_eq!(run(0o126, 0, m_one_half).0, MASK36); // -1
        // 2^36 does not fit: Overflow, AC unchanged.
        let (w, f) = run(0o122, 7, 0o245400_000000);
        assert_eq!(w, 7);
        assert!(f.has(Flags::AROV));
    }

    #[test]
    fn fltr_fsc() {
        assert_eq!(run(0o127, 0, 1).0, ONE);
        assert_eq!(run(0o127, 0, MASK36).0, M_ONE);
        // 2^27 + 1 needs 28 bits: rounds to 2^27 + 2 (the next bit is 1).
        assert_eq!(run(0o127, 0, (1 << 27) | 1).0, 0o234400_000001);
        // FSC 1,233 floats an integer in bits 9-35 (PRM, FSC).
        let mut m = Machine::new();
        m.ac[1] = W36::wrap(5);
        let i = Inst::new(W36::wrap(0o132040_000233));
        execute(&mut m, i, Half::wrap(0o233)).unwrap();
        assert_eq!(m.ac[1].get(), 0o203500_000000);
    }

    #[test]
    fn negative_truncation() {
        // -1/3 without rounding: the ones complement of 1/3 (PRM, FDV note).
        let (w, _) = run(0o170, M_ONE, 0o202600_000000);
        assert_eq!(w, !0o177525_252525 & MASK36);
        // FDVR: the twos complement of the rounded magnitude.
        let (w, _) = run(0o174, M_ONE, 0o202600_000000);
        assert_eq!(w, 0o177525_252525u64.wrapping_neg() & MASK36);
    }

    #[test]
    fn double_ops() {
        let mut m = Machine::new();
        // 1.0 + 1.0 in double precision.
        m.ac[1] = W36::wrap(ONE);
        m.ac[2] = W36::ZERO;
        m.mem[0o100] = W36::wrap(ONE);
        m.mem[0o101] = W36::ZERO;
        let i = Inst::new(W36::wrap(0o110040_000100));
        execute(&mut m, i, Half::wrap(0o100)).unwrap();
        assert_eq!((m.ac[1].get(), m.ac[2].get()), (TWO, 0));
        // DFSB 2.0 - 2.0 = 0.
        let i = Inst::new(W36::wrap(0o111040_000100));
        m.mem[0o100] = W36::wrap(TWO);
        execute(&mut m, i, Half::wrap(0o100)).unwrap();
        assert_eq!((m.ac[1].get(), m.ac[2].get()), (0, 0));
        // DFDV 1.0 / 3.0: 0.0101... in 62 bits, rounded (the 63rd bit is 1).
        m.ac[1] = W36::wrap(ONE);
        m.mem[0o100] = W36::wrap(0o202600_000000);
        let i = Inst::new(W36::wrap(0o113040_000100));
        execute(&mut m, i, Half::wrap(0o100)).unwrap();
        assert_eq!(
            (m.ac[1].get(), m.ac[2].get()),
            (0o177525_252525, 0o125252_525253)
        );
        // DFMP -1.0 * 2.0 = -2.0.
        m.ac[1] = W36::wrap(M_ONE);
        m.ac[2] = W36::ZERO;
        m.mem[0o100] = W36::wrap(TWO);
        let i = Inst::new(W36::wrap(0o112040_000100));
        execute(&mut m, i, Half::wrap(0o100)).unwrap();
        assert_eq!((m.ac[1].get(), m.ac[2].get()), (0o575400_000000, 0));
    }
}

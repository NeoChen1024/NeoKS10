//! Shift and rotate (PRM 2.5), JFFO (PRM 2.6), CIRC (ITS) and BLT
//! (PRM 2.1, "Block Transfers").

use pdp10::{Half, W36};

use crate::flags::Flags;
use crate::machine::{Fault, Inst, Machine, Next};

type R = Result<Next, Fault>;

const MASK36: u128 = (1 << 36) - 1;
const MASK35: u128 = (1 << 35) - 1;

/// The shift count: E bit 18 is the sign and bits 28 to 35 the low bits of
/// a 9-bit two's complement number (PRM 2.5, page 2-39): -256 to 255.
pub fn count(e: Half) -> i32 {
    let low = (e.get() & 0o377) as i32;
    if e.bit(0) { low - 256 } else { low }
}

fn next_ac(ac: usize) -> usize {
    (ac + 1) & 0o17
}

pub fn execute(m: &mut Machine, i: Inst, e: Half) -> R {
    let n = count(e);
    let a = u128::from(m.ac[i.ac].get());
    let b = u128::from(m.ac[next_ac(i.ac)].get());
    match i.op {
        0o240 => {
            // ASH
            let (w, ovf) = ash(a, 36, n);
            if ovf {
                m.flags.set(Flags::AROV | Flags::TRAP1);
            }
            m.ac[i.ac] = W36::wrap(w as u64);
        }
        0o241 => m.ac[i.ac] = W36::wrap(rotate(a, 36, n) as u64),
        0o242 => m.ac[i.ac] = W36::wrap(lshift(a, 36, n) as u64),
        0o243 => {
            // JFFO: AC+1 <- the number of leading zeros of AC; jump if
            // AC is not zero (AC+1 is 0 if AC is 0).
            let x = m.ac[i.ac].get();
            if x == 0 {
                m.ac[next_ac(i.ac)] = W36::ZERO;
                return Ok(Next::Step);
            }
            m.ac[next_ac(i.ac)] = W36::wrap(u64::from(x.leading_zeros() - 28));
            return Ok(Next::Jump(e));
        }
        0o244 => {
            // ASHC: the 70 magnitude bits of AC bits 1-35 and AC+1 bits
            // 1-35. If E is not 0, AC+1 bit 0 becomes the sign of AC.
            if n == 0 {
                return Ok(Next::Step);
            }
            let sign = a >> 35;
            let mag = ((a & MASK35) << 35) | (b & MASK35);
            let (w, ovf) = ash((sign << 70) | mag, 71, n);
            if ovf {
                m.flags.set(Flags::AROV | Flags::TRAP1);
            }
            let hi = (sign << 35) | ((w >> 35) & MASK35);
            let lo = (sign << 35) | (w & MASK35);
            m.ac[i.ac] = W36::wrap(hi as u64);
            m.ac[next_ac(i.ac)] = W36::wrap(lo as u64);
        }
        0o245 | 0o246 => {
            // ROTC, LSHC: AC,AC+1 as 72 bits.
            let d = (a << 36) | b;
            let w = if i.op == 0o245 {
                rotate(d, 72, n)
            } else {
                lshift(d, 72, n)
            };
            m.ac[i.ac] = W36::wrap((w >> 36) as u64);
            m.ac[next_ac(i.ac)] = W36::wrap((w & MASK36) as u64);
        }
        0o247 => {
            // CIRC: "ROTC with AC+1 going the wrong way" [its] (MIDAS
            // midas.458, ISYMTB comment). A left step moves AC left with AC+1
            // bit 35 into AC bit 35, and AC+1 right with AC bit 0 into AC+1
            // bit 0 (the same in sims 9556eca0, kx10_cpu.c, case 0247). As a
            // rotation: AC with AC+1 in reverse bit order, rotated as ROTC.
            let d = (a << 36) | reverse36(b);
            let w = rotate(d, 72, n);
            m.ac[i.ac] = W36::wrap((w >> 36) as u64);
            m.ac[next_ac(i.ac)] = W36::wrap(reverse36(w & MASK36) as u64);
        }
        _ => return Err(Fault::Unimplemented),
    }
    Ok(Next::Step)
}

fn reverse36(x: u128) -> u128 {
    u128::from((x as u64).reverse_bits() >> 28)
}

/// Logical shift of a `bits`-bit value: left for n > 0, right for n < 0;
/// zeros come in.
fn lshift(x: u128, bits: u32, n: i32) -> u128 {
    let mask = if bits == 128 {
        u128::MAX
    } else {
        (1 << bits) - 1
    };
    let k = n.unsigned_abs();
    if k >= bits {
        0
    } else if n >= 0 {
        (x << k) & mask
    } else {
        x >> k
    }
}

/// Rotation of a `bits`-bit value.
fn rotate(x: u128, bits: u32, n: i32) -> u128 {
    let mask = (1u128 << bits) - 1;
    let k = n.rem_euclid(bits as i32) as u32;
    if k == 0 {
        x
    } else {
        ((x << k) | (x >> (bits - k))) & mask
    }
}

/// Arithmetic shift of a `bits`-bit two's complement value (the sign is the
/// top bit and does not move). Returns the result and whether a bit of
/// significance was lost in a left shift (PRM 2.5, ASH).
fn ash(x: u128, bits: u32, n: i32) -> (u128, bool) {
    let sign = (x >> (bits - 1)) & 1;
    let mag_bits = bits - 1;
    let mag_mask = (1u128 << mag_bits) - 1;
    let mag = x & mag_mask;
    let k = n.unsigned_abs();
    if n >= 0 {
        // The bits shifted out of the magnitude must all equal the sign.
        let lost_bits = k.min(mag_bits);
        let lost = if lost_bits == 0 {
            0
        } else {
            mag >> (mag_bits - lost_bits)
        };
        let all = if sign == 1 {
            (1u128 << lost_bits) - 1
        } else {
            0
        };
        let shifted = if k >= mag_bits {
            0
        } else {
            (mag << k) & mag_mask
        };
        ((sign << mag_bits) | shifted, lost != all)
    } else {
        // Right: the sign comes in at bit 1.
        let fill = if sign == 1 { mag_mask } else { 0 };
        let shifted = if k >= mag_bits {
            fill
        } else {
            (mag >> k) | (fill & !(mag_mask >> k))
        };
        ((sign << mag_bits) | shifted, false)
    }
}

/// BLT (PRM 2.1, "Block Transfers"): move words from AC left to AC right
/// until a word is moved to E.
///
/// The order is the one of the ITS microcode **[microcode]** (simple.42,
/// BLT and SETBLT): first AC gets its final value, AC + n,,n with
/// n = E - (AC right) + 1 (mod 2^18, no carry between the halves); then the
/// words move, each read after the earlier writes. So if AC is in the
/// source or destination block, the words see the final AC. After each
/// word, the move stops if the destination was not below E.
pub fn blt(m: &mut Machine, i: Inst, e: Half) -> R {
    let p = m.ac[i.ac];
    let (mut src, mut dst) = (p.lh(), p.rh());
    let n = (e.get() + (1 << 18) - dst.get() + 1) & Half::MASK;
    m.ac[i.ac] = W36::from_halves(Half::wrap(src.get() + n), Half::wrap(dst.get() + n));
    loop {
        let w = m.read(src)?;
        m.write(dst, w)?;
        if dst.get() >= e.get() {
            break;
        }
        src = Half::wrap(src.get() + 1);
        dst = Half::wrap(dst.get() + 1);
    }
    Ok(Next::Step)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values from the definitions of PRM 2.5, computed by hand.
    #[test]
    fn counts() {
        assert_eq!(count(Half::wrap(3)), 3);
        assert_eq!(count(Half::wrap(0o777775)), -3);
        assert_eq!(count(Half::wrap(0o400000)), -256);
        // Bits 19 to 27 are not used.
        assert_eq!(count(Half::wrap(0o001003)), 3);
    }

    #[test]
    fn ash_cases() {
        // 1 shifted left 34 places: 200000000000, no overflow.
        assert_eq!(ash(1, 36, 34), (0o200000000000, false));
        // 1 shifted left 35 places: the 1 is lost, overflow.
        assert_eq!(ash(1, 36, 35), (0, true));
        // -1 shifted left 1: 777777777776, no overflow.
        assert_eq!(ash(0o777777777777, 36, 1), (0o777777777776, false));
        // 400000000000 shifted left 1: a 0 is lost from a negative number.
        assert_eq!(ash(0o400000000000, 36, 1), (0o400000000000, true));
        // Right shifts bring in the sign: -2 >> 1 = -1; -1 >> 100 = -1.
        assert_eq!(ash(0o777777777776, 36, -1), (0o777777777777, false));
        assert_eq!(ash(0o777777777777, 36, -100), (0o777777777777, false));
        assert_eq!(ash(0o000000000100, 36, -3), (0o10, false));
    }

    #[test]
    fn rotations_and_logical() {
        assert_eq!(rotate(0o400000000001, 36, 1), 0o000000000003);
        assert_eq!(rotate(0o400000000001, 36, -1), 0o600000000000);
        assert_eq!(rotate(0o123, 36, 36), 0o123);
        assert_eq!(lshift(0o400000000001, 36, 1), 0o000000000002);
        assert_eq!(lshift(0o400000000001, 36, -35), 1);
        assert_eq!(lshift(0o777, 36, 40), 0);
    }

    #[test]
    fn circ_step() {
        // CIRC 1: AC 400000000000, AC+1 000000000001 -> AC gets AC+1 bit 35
        // (1) at bit 35, AC+1 gets AC bit 0 (1) at bit 0.
        let mut m = Machine::new();
        m.ac[1] = W36::wrap(0o400000000000);
        m.ac[2] = W36::wrap(0o000000000001);
        let i = Inst::new(W36::wrap(0o247040_000001));
        execute(&mut m, i, Half::wrap(1)).unwrap();
        assert_eq!(
            (m.ac[1].get(), m.ac[2].get()),
            (0o000000000001, 0o400000000000)
        );
    }

    #[test]
    fn blt_moves() {
        let mut m = Machine::new();
        for k in 0..4 {
            m.mem[0o1000 + k] = W36::wrap(0o100 + k as u64);
        }
        m.ac[1] = W36::from_halves(Half::wrap(0o1000), Half::wrap(0o2000));
        let i = Inst::new(W36::wrap(0o251040_002003));
        blt(&mut m, i, Half::wrap(0o2003)).unwrap();
        assert_eq!(m.mem[0o2003].get(), 0o103);
        assert_eq!(m.mem[0o2004].get(), 0);
        assert_eq!(
            m.ac[1],
            W36::from_halves(Half::wrap(0o1004), Half::wrap(0o2004))
        );
    }

    // SUBSM (DSQDA) checks the processor with MOVE 1,X; BLT 1,1; CAMN 1,X;
    // HALT CPIERR: on a KS10 the BLT changes AC1. With AC1 = 1,,1 the
    // final value 2,,2 is stored first and then moved onto itself
    // (simple.42, BLT stores AC before the first read).
    #[test]
    fn blt_onto_its_ac() {
        let mut m = Machine::new();
        m.ac[1] = W36::from_halves(Half::wrap(1), Half::wrap(1));
        let i = Inst::new(W36::wrap(0o251040_000001));
        blt(&mut m, i, Half::wrap(1)).unwrap();
        assert_eq!(m.ac[1], W36::from_halves(Half::wrap(2), Half::wrap(2)));
        // Destination above E: one word moves.
        m.mem[0o3000] = W36::wrap(7);
        m.ac[2] = W36::from_halves(Half::wrap(0o3000), Half::wrap(0o3010));
        let i = Inst::new(W36::wrap(0o251100_003005));
        blt(&mut m, i, Half::wrap(0o3005)).unwrap();
        assert_eq!(m.mem[0o3010].get(), 7);
        assert_eq!(m.mem[0o3011].get(), 0);
    }
}

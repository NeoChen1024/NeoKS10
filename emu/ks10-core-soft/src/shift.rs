//! Shift and rotate (PRM 2.5), JFFO (PRM 2.6), CIRC (ITS) and BLT
//! (PRM 2.1, "Block Transfers").

use pdp10::{Half, W36};

use crate::flags::Flags;
use crate::machine::{Fault, Inst, Machine, Next, Ref};

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
            let r = ash(a, 36, n);
            r.set_flags(&mut m.flags);
            m.ac[i.ac] = W36::wrap(r.value as u64);
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
            let r = ash((sign << 70) | mag, 71, n);
            r.set_flags(&mut m.flags);
            let w = r.value;
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

/// The result of an arithmetic shift.
#[derive(Debug, PartialEq, Eq)]
struct Ash {
    value: u128,
    /// A step lost a bit that differs from the sign.
    overflow: bool,
    /// The last step lost such a bit.
    last: bool,
}

impl Ash {
    /// A left shift that loses a bit of significance sets Overflow (PRM
    /// 2.5, ASH). Trap 1 is the overflow of the last step: each step of
    /// the microcode loads it (M8620-0-DPE9 rev B, E85, E78; simple.42,
    /// ASHL, ASHCL), so an earlier overflow does not set it (design
    /// decision 2026-11-28, case ash-trap-last-step of spec/impldef.toml).
    /// A Trap 1 from before stays (case trap-flags-clear).
    fn set_flags(&self, f: &mut Flags) {
        if self.overflow {
            f.set(Flags::AROV);
        }
        if self.last {
            f.set(Flags::TRAP1);
        }
    }
}

/// Arithmetic shift of a `bits`-bit two's complement value (the sign is the
/// top bit and does not move) by `n` places, left if `n` is positive.
fn ash(x: u128, bits: u32, n: i32) -> Ash {
    let sign = (x >> (bits - 1)) & 1;
    let mag_bits = bits - 1;
    let mag_mask = (1u128 << mag_bits) - 1;
    let mag = x & mag_mask;
    let k = n.unsigned_abs();
    if n > 0 {
        // The bits shifted out of the magnitude must all equal the sign
        // (simple.42, ASHL: one place for each step of SC, ASH AROV).
        let lost_bits = k.min(mag_bits);
        let lost = mag >> (mag_bits - lost_bits);
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
        // After mag_bits places, the zeros that came in at the right leave
        // bit 1 too: they differ from the sign of a negative number.
        let zeros_lost = sign == 1 && k > mag_bits;
        // The bit that leaves bit 1 in the last step: bit k of the
        // magnitude from the top, or a 0 that came in.
        let last_bit = if k > mag_bits {
            0
        } else {
            (mag >> (mag_bits - k)) & 1
        };
        Ash {
            value: (sign << mag_bits) | shifted,
            overflow: lost != all || zeros_lost,
            last: last_bit != sign,
        }
    } else {
        // Right: the sign comes in at bit 1.
        let fill = if sign == 1 { mag_mask } else { 0 };
        let shifted = if k >= mag_bits {
            fill
        } else {
            (mag >> k) | (fill & !(mag_mask >> k))
        };
        Ash {
            value: (sign << mag_bits) | shifted,
            overflow: false,
            last: false,
        }
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
/// word, the move stops if the destination was not below E. A page failure
/// leaves src,,dst of the word that failed in AC (BLT-CLEANUP), so that the
/// BLT continues there.
pub fn blt(m: &mut Machine, i: Inst, e: Half) -> R {
    blt_with(m, i, e, |w| w)
}

/// BLTBU (716), BLTUB (717) **[microcode]** (simple.42, "UBABLT"): BLT
/// that converts each word from byte format (bytes 0 to 3 in bits 0 to 31)
/// to Unibus format (bits 2-9 byte 1, 10-17 byte 0, 20-27 byte 3, 28-35
/// byte 2), or back. The other bits become 0.
pub fn bltub(m: &mut Machine, i: Inst, e: Half) -> R {
    if i.op == 0o716 {
        blt_with(m, i, e, |w| {
            let b = |k: u32| w.field(8 * k, 8 * k + 7);
            W36::wrap((b(1) << 26) | (b(0) << 18) | (b(3) << 8) | b(2))
        })
    } else {
        blt_with(m, i, e, |w| {
            let (b1, b0) = (w.field(2, 9), w.field(10, 17));
            let (b3, b2) = (w.field(20, 27), w.field(28, 35));
            W36::wrap((b0 << 28) | (b1 << 20) | (b2 << 12) | (b3 << 4))
        })
    }
}

fn blt_with(m: &mut Machine, i: Inst, e: Half, f: impl Fn(W36) -> W36) -> R {
    let p = m.ac[i.ac];
    let (mut src, mut dst) = (p.lh(), p.rh());
    let n = (e.get() + (1 << 18) - dst.get() + 1) & Half::MASK;
    m.ac[i.ac] = W36::from_halves(Half::wrap(src.get() + n), Half::wrap(dst.get() + n));
    loop {
        let r = m
            .vread(src, Ref::D2)
            .and_then(|w| m.vwrite(dst, f(w), Ref::D1));
        if let Err(x) = r {
            m.ac[i.ac] = W36::from_halves(src, dst);
            return Err(x);
        }
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

    fn ash2(x: u128, bits: u32, n: i32) -> (u128, bool) {
        let r = ash(x, bits, n);
        (r.value, r.overflow)
    }

    #[test]
    fn ash_cases() {
        // 1 shifted left 34 places: 200000000000, no overflow.
        assert_eq!(ash2(1, 36, 34), (0o200000000000, false));
        // 1 shifted left 35 places: the 1 is lost, overflow.
        assert_eq!(ash2(1, 36, 35), (0, true));
        // -1 shifted left 1: 777777777776, no overflow.
        assert_eq!(ash2(0o777777777777, 36, 1), (0o777777777776, false));
        // 400000000000 shifted left 1: a 0 is lost from a negative number.
        assert_eq!(ash2(0o400000000000, 36, 1), (0o400000000000, true));
        // -1 shifted left 35 places is -2^35 and fits. Left 36 places: the
        // zeros that came in at bit 35 reach bit 1 and leave it, so a 0 is
        // lost from a negative number (simple.42, ASHL: one place for each
        // step, ASH AROV). KLH10 sets Overflow too.
        assert_eq!(ash2(0o777777777777, 36, 35), (0o400000000000, false));
        assert_eq!(ash2(0o777777777777, 36, 53), (0o400000000000, true));
        // ASHC: -1 (both words) left 71 places.
        assert_eq!(ash2((1 << 71) - 1, 71, 71), (1 << 70, true));
        // Right shifts bring in the sign: -2 >> 1 = -1; -1 >> 100 = -1.
        assert_eq!(ash2(0o777777777776, 36, -1), (0o777777777777, false));
        assert_eq!(ash2(0o777777777777, 36, -100), (0o777777777777, false));
        assert_eq!(ash2(0o000000000100, 36, -3), (0o10, false));
    }

    // The microcode (M8620-0-DPE9 rev B, E85, E78; simple.42, ASHL): each
    // step loads Trap 1 with its own overflow, so Trap 1 is the overflow of
    // the last step; Overflow holds (E68). docs/discrepancies.md, "Trap 1
    // and Trap 2: each flag word loads them": ASH 1,2 of 200000,,1 gives
    // OV 1 and TRAP 1 0. A Trap 1 from before stays (case
    // trap-flags-clear of spec/impldef.toml).
    #[test]
    fn trap_1_is_the_overflow_of_the_last_step() {
        let flags = |x: u128, bits: u32, n: i32, before: u32| {
            let mut f = Flags(before);
            ash(x, bits, n).set_flags(&mut f);
            f
        };
        assert_eq!(flags(0o200000_000001, 36, 2, 0), Flags(Flags::AROV));
        assert_eq!(
            flags(0o200000_000001, 36, 1, 0),
            Flags(Flags::AROV | Flags::TRAP1)
        );
        // 500000,,0: the first step loses bit 1 (0), the second keeps bit 2.
        assert_eq!(flags(0o500000_000000, 36, 2, 0), Flags(Flags::AROV));
        // -1 left 36 places: the last step shifts out a 0 of the zeros
        // that came in, so the last step overflows.
        assert_eq!(
            flags(0o777777_777777, 36, 36, 0),
            Flags(Flags::AROV | Flags::TRAP1)
        );
        // ASHC: 200000,,0 and 0, two places: only the first step overflows.
        assert_eq!(flags(1 << 69, 71, 2, 0), Flags(Flags::AROV));
        assert_eq!(
            flags(0o000000_000001, 36, 2, Flags::TRAP1),
            Flags(Flags::TRAP1)
        );
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
    // simple.42, UBABLT: byte format bytes 0-3 in bits 0-31; Unibus format
    // byte 1, byte 0 in bits 2-17, byte 3, byte 2 in bits 20-35.
    #[test]
    fn unibus_format() {
        let mut m = Machine::new();
        let b = [0o101u64, 0o102, 0o103, 0o104];
        m.mem[0o1000] = W36::wrap((b[0] << 28) | (b[1] << 20) | (b[2] << 12) | (b[3] << 4));
        m.ac[1] = W36::from_halves(Half::wrap(0o1000), Half::wrap(0o2000));
        bltub(
            &mut m,
            Inst::new(W36::wrap(0o716040_002000)),
            Half::wrap(0o2000),
        )
        .unwrap();
        let u = (b[1] << 26) | (b[0] << 18) | (b[3] << 8) | b[2];
        assert_eq!(m.mem[0o2000].get(), u);
        m.ac[1] = W36::from_halves(Half::wrap(0o2000), Half::wrap(0o3000));
        bltub(
            &mut m,
            Inst::new(W36::wrap(0o717040_003000)),
            Half::wrap(0o3000),
        )
        .unwrap();
        assert_eq!(m.mem[0o3000], m.mem[0o1000]);
    }

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

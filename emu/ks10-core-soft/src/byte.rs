//! The byte instructions: IBP and ADJBP (133), ILDB, LDB, IDPB, DPB (134
//! to 137) (PRM 2.12, "Byte Manipulation").
//!
//! The order of the steps and the use of First Part Done follow the ITS
//! microcode **[microcode]** (simple.42, "BYTE GROUP"): ILDB and IDPB
//! store the incremented pointer, then set FPD while they compute the byte
//! address, and clear it at the end. If FPD is set when they start (an
//! interrupt came during the first try), they do not increment the pointer
//! again (IBPS). IBP also does not increment with FPD set, and leaves FPD
//! as it is.

use pdp10::{Half, W36};

use crate::flags::Flags;
use crate::machine::{Fault, Inst, Machine, Next, Ref};

type R = Result<Next, Fault>;

/// P (bits 0 to 5) and S (bits 6 to 11) of a byte pointer.
fn ps(p: W36) -> (u32, u32) {
    (p.field(0, 5) as u32, p.field(6, 11) as u32)
}

fn with_p(ptr: W36, p: u32) -> W36 {
    W36::wrap((ptr.get() & 0o007777_777777) | (u64::from(p & 0o77) << 30))
}

/// The pointer after one increment (IBPS, NXTWRD): P - S if that is not
/// negative; else P = 36 - S (in 6 bits) and Y + 1 in the right half only.
pub fn increment(ptr: W36) -> W36 {
    let (p, s) = ps(ptr);
    if p >= s {
        with_p(ptr, p - s)
    } else {
        let q = with_p(ptr, 36u32.wrapping_sub(s));
        W36::from_halves(q.lh(), Half::wrap(q.rh().get() + 1))
    }
}

/// The byte of `word` at P and S (LDB1): `word` shifted right P places,
/// masked to S bits. P of 36 or more gives 0.
pub fn load(word: W36, p: u32, s: u32) -> W36 {
    let w = u128::from(word.get());
    W36::wrap(((w >> p) & mask(s)) as u64)
}

/// `word` with the byte at P and S replaced by the low bits of `byte`
/// (DPB1): bits that fall outside the word are lost.
pub fn deposit(word: W36, byte: W36, p: u32, s: u32) -> W36 {
    let m = (mask(s) << p) & 0o777777_777777;
    let b = (u128::from(byte.get()) << p) & m;
    W36::wrap(((u128::from(word.get()) & !m) | b) as u64)
}

fn mask(s: u32) -> u128 {
    (1u128 << s.min(36)) - 1
}

pub fn execute(m: &mut Machine, i: Inst, e: Half) -> R {
    if i.op == 0o133 && i.ac != 0 {
        return adjbp(m, i, e);
    }
    let incr = matches!(i.op, 0o133 | 0o134 | 0o136);
    let mut ptr = m.read(e)?;
    if incr && !m.flags.has(Flags::FPD) {
        ptr = increment(ptr);
        m.write(e, ptr)?;
    }
    if i.op == 0o133 {
        return Ok(Next::Step);
    }
    m.flags.set(Flags::FPD);
    let Some((be, _)) = m.ea_ref(ptr, Ref::E2)? else {
        // An indirect chain that does not end: do the instruction again
        // with FPD set, as after an interrupt.
        return Ok(Next::Jump(m.pc));
    };
    let (p, s) = ps(ptr);
    // The byte data: PXCT BYTE DATA (D2), read and written in one cycle.
    let word = m.vread(be, Ref::D2)?;
    if i.op & 2 == 0 {
        // ILDB, LDB
        m.ac[i.ac] = load(word, p, s);
    } else {
        m.vwrite(be, deposit(word, m.ac[i.ac], p, s), Ref::D2)?;
    }
    m.flags.clear(Flags::FPD);
    Ok(Next::Step)
}

/// ADJBP (simple.42, ADJBP to ADJBP6): AC <- the pointer at E moved by
/// C(AC) bytes.
fn adjbp(m: &mut Machine, i: Inst, e: Half) -> R {
    let ptr = m.read(e)?;
    let (p, s) = ps(ptr);
    if s == 0 {
        m.ac[i.ac] = ptr;
        return Ok(Next::Step);
    }
    if p > 36 {
        // The microcode divides 36 - P as an unsigned 36-bit number; this
        // case is not modelled yet.
        return Err(Fault::Unimplemented);
    }
    let (p, s) = (i64::from(p), i64::from(s));
    let r2 = (36 - p) % s;
    let bpw = p / s + (36 - p) / s;
    if bpw == 0 {
        // NO DIVIDE: the AC does not change.
        m.flags.set(Flags::AROV | Flags::TRAP1 | Flags::NODIV);
        return Ok(Next::Step);
    }
    let adj = m.ac[i.ac].signed();
    let (mut q, mut r) = (adj / bpw, adj % bpw);
    // Make the remainder 1 to bpw (ADJBP3, ADJBP4).
    if r <= 0 {
        q -= 1;
        r += bpw;
    }
    let y = Half::wrap(ptr.rh().get().wrapping_add(q as u64));
    let new_p = (36 - (r * s + r2)) as u32;
    m.ac[i.ac] = with_p(W36::from_halves(ptr.lh(), y), new_p);
    Ok(Next::Step)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values from the PRM 2.12 definitions, computed by hand.
    #[test]
    fn increment_cases() {
        // 7-bit bytes: P 36 -> 29; P 1 -> next word, P 29.
        let p = W36::wrap(0o440700_001000);
        assert_eq!(increment(p).get(), 0o350700_001000);
        let p = W36::wrap(0o010700_001000);
        assert_eq!(increment(p).get(), 0o350700_001001);
        // Y + 1 does not carry into the left half.
        let p = W36::wrap(0o000600_777777);
        assert_eq!(increment(p).get(), 0o360600_000000);
    }

    #[test]
    fn load_deposit() {
        let w = W36::wrap(0o123456_701234);
        assert_eq!(load(w, 0, 6).get(), 0o34);
        assert_eq!(load(w, 30, 6).get(), 0o12);
        assert_eq!(load(w, 36, 6).get(), 0);
        assert_eq!(load(w, 33, 6).get(), 0o1);
        // S = 36 is the whole word; a larger S gives no more bits.
        let n = W36::wrap(0o723456_701234);
        assert_eq!(load(n, 0, 36), n);
        assert_eq!(load(n, 0, 63), n);
        assert_eq!(deposit(n, W36::ZERO, 0, 36), W36::ZERO);
        assert_eq!(deposit(w, W36::wrap(0o77), 12, 6).get(), 0o123456_771234);
        // P + S > 36: only the bits inside the word change.
        assert_eq!(
            deposit(W36::ZERO, W36::wrap(0o77), 33, 6).get(),
            0o700000_000000
        );
        assert_eq!(deposit(w, W36::wrap(0o77), 36, 6), w);
    }

    // ADJBP, values from the formulas in the simple.42 comments: 7-bit
    // bytes, P = 36 (before the first byte), 5 bytes per word.
    #[test]
    fn adjbp_cases() {
        let mut m = Machine::new();
        m.mem[0o100] = W36::wrap(0o440700_001000);
        let i = Inst::new(W36::wrap(0o133040_000100));
        for (adj, want) in [
            (1i64, 0o350700_001000u64),
            (5, 0o010700_001000),
            (6, 0o350700_001001),
            (0, 0o010700_000777),
            (-1, 0o100700_000777),
        ] {
            m.ac[1] = W36::wrap(adj as u64 & 0o777777_777777);
            adjbp(&mut m, i, Half::wrap(0o100)).unwrap();
            assert_eq!(m.ac[1].get(), want, "adj {adj}");
        }
    }
}

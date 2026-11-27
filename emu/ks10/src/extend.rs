//! EXTEND (123): the string instructions CMPSx, MOVSO, MOVST, MOVSLJ,
//! MOVSRJ, CVTDBO, CVTDBT, CVTBDO, CVTBDT and EDIT (PRM 1982, 2.12 to
//! 2.14).
//!
//! The behaviour follows the ITS microcode **[microcode]** (extend.4)
//! where it is more exact than the PRM:
//!
//! - EXTEND: the word at E0 must have an opcode of 1 to 17 and AC 0, else
//!   the instruction is a monitor UUO (EXTEND, BITCHK). E1 is the
//!   effective address of that word; the offset instructions extend its
//!   sign from bit 18 (EXTEXT).
//! - MOVSx and CMPSx do a UUO if bits 0 to 8 of a length word are not 0;
//!   EDIT if bit 3 or bits 6 to 17 of AC are not 0 (BITCHK).
//! - A source byte that ends a move (an offset byte that is too large, or
//!   a translation that terminates) counts as read (MVABT, DBABT).
//! - MOVSRJ leaves 0 in AC bits 9-35 (MOVST3); docs/discrepancies.md.
//! - CMPSx compares the bytes and the fill words as unsigned 36-bit
//!   numbers (CMS7).
//! - A translation table entry has the function in bits 0 to 2 and the
//!   substitute in bits 3 to 17 of its half word (TRNSS1: mask 77777).
//!
//! The instructions run to the end in one step; this version takes no
//! interrupt or page failure inside them.

use pdp10::{Half, W36};

use crate::byte;
use crate::machine::{Fault, Inst, Machine, Next, Ref};

type R = Result<Next, Fault>;

const MASK36: u64 = 0o777777_777777;
const LEN: u64 = 0o000777_777777;
const S: u64 = 1 << 35;
const N: u64 = 1 << 34;
const M: u64 = 1 << 33;

fn acn(i: Inst, k: usize) -> usize {
    (i.ac + k) & 0o17
}

fn addr(base: Half, k: u64) -> Half {
    Half::wrap(base.get() + k)
}

/// The byte EA of a pointer.
fn byte_ea(m: &mut Machine, ptr: W36) -> Result<Half, Fault> {
    match m.ea_ref(ptr, Ref::E2)? {
        Some((e, _)) => Ok(e),
        None => Err(Fault::Unimplemented),
    }
}

/// ILDB through the pointer in AC k.
fn ildb(m: &mut Machine, k: usize) -> Result<W36, Fault> {
    let ptr = byte::increment(m.ac[k]);
    m.ac[k] = ptr;
    let e = byte_ea(m, ptr)?;
    let (p, s) = (ptr.field(0, 5) as u32, ptr.field(6, 11) as u32);
    Ok(byte::load(m.vread(e, Ref::D2)?, p, s))
}

/// IDPB through the pointer in AC k.
fn idpb(m: &mut Machine, k: usize, b: W36) -> Result<(), Fault> {
    let ptr = byte::increment(m.ac[k]);
    m.ac[k] = ptr;
    let e = byte_ea(m, ptr)?;
    let (p, s) = (ptr.field(0, 5) as u32, ptr.field(6, 11) as u32);
    let w = m.vread(e, Ref::D2)?;
    m.vwrite(e, byte::deposit(w, b, p, s), Ref::D2)?;
    Ok(())
}

/// A translation table entry for byte `b`: the half word at E1 + b/2, the
/// right half for an odd byte (XLATE, TRNAR). Returns the function code
/// and the substitute.
fn xlate(m: &mut Machine, e1: W36, b: W36) -> Result<(u64, u64), Fault> {
    let a = Half::wrap(e1.get().wrapping_add(b.get() >> 1));
    let w = m.read(a)?;
    let h = if b.get() & 1 == 1 { w.rh() } else { w.lh() }.get();
    Ok((h >> 15, h & 0o77777))
}

/// What a translation function does (TRNFNC): the new flags, and whether
/// to stop, to take the substitute, or to start significance.
enum Tr {
    Abort,
    /// Significance is set: use the substitute.
    Take,
    /// No significance yet (codes 0, 2, 3 with S clear).
    Skip,
    /// Significance starts with this byte (codes 4, 6, 7 with S clear).
    Start,
}

fn translate(code: u64, flags: &mut u64) -> Tr {
    match code {
        1 => return Tr::Abort,
        2 => *flags &= !M,
        3 => *flags |= M,
        4 => *flags |= N,
        5 => {
            *flags |= N;
            return Tr::Abort;
        }
        6 => *flags = (*flags & !M) | N,
        7 => *flags |= N | M,
        _ => {}
    }
    if *flags & S != 0 {
        Tr::Take
    } else if code >= 4 {
        Tr::Start
    } else {
        Tr::Skip
    }
}

pub fn execute(m: &mut Machine, i: Inst, e0: Half) -> R {
    // A page failure inside the instruction: the ACs go back to their
    // state at the start, so that the instruction starts again (the
    // microcode instead keeps the work done, extend.4, "PAGE FAIL
    // CLEANUP"; both give the same result when it runs again).
    let saved = m.ac;
    let r = run(m, i, e0);
    if let Err(Fault::PageFail(_)) = r {
        m.ac = saved;
    }
    r
}

fn run(m: &mut Machine, i: Inst, e0: Half) -> R {
    let w = m.read(e0)?;
    let op = w.field(0, 8);
    if op == 0 || op > 0o17 || w.field(9, 12) != 0 {
        return Err(Fault::Uuo);
    }
    let Some((e1, _)) = m.ea_ref(w, Ref::E2)? else {
        return Err(Fault::Unimplemented);
    };
    // Offset modes extend the sign of E1 (EXTEXT, DROM B = 1).
    let offset = matches!(op, 0o010 | 0o012 | 0o014);
    let e1w = if offset && e1.bit(0) {
        W36::from_halves(Half::wrap(0o777777), e1)
    } else {
        W36::from_halves(Half::ZERO, e1)
    };
    match op {
        0o001..=0o003 | 0o005..=0o007 => cmps(m, i, e0, op),
        0o004 => edit(m, i, e0, e1w),
        0o010 | 0o011 => cvtdb(m, i, e1w, op == 0o010),
        0o012 | 0o013 => cvtbd(m, i, e0, e1w, op == 0o012),
        _ => movs(m, i, e0, e1w, op),
    }
}

/// CMPSL, CMPSE, CMPSLE, CMPSGE, CMPSN, CMPSG (CMS to CMS8).
fn cmps(m: &mut Machine, i: Inst, e0: Half, op: u64) -> R {
    let (la, lb) = (m.ac[i.ac].get(), m.ac[acn(i, 3)].get());
    if (la | lb) & !LEN != 0 {
        return Err(Fault::Uuo);
    }
    // One fill word: for string 1 if string 2 is longer, else for 2.
    let fill = if lb > la {
        m.read(addr(e0, 1))?
    } else {
        m.read(addr(e0, 2))?
    };
    let (mut ls, mut ld) = (la, lb);
    let mut cmp = std::cmp::Ordering::Equal;
    while ls > 0 || ld > 0 {
        let a = if ls > 0 {
            ls -= 1;
            let b = ildb(m, acn(i, 1))?;
            m.ac[i.ac] = W36::wrap(ls);
            b
        } else {
            fill
        };
        let b = if ld > 0 {
            ld -= 1;
            let b = ildb(m, acn(i, 4))?;
            m.ac[acn(i, 3)] = W36::wrap(ld);
            b
        } else {
            fill
        };
        cmp = a.get().cmp(&b.get());
        if cmp.is_ne() {
            break;
        }
    }
    use std::cmp::Ordering::*;
    let skip = match op {
        0o001 => cmp == Less,
        0o002 => cmp == Equal,
        0o003 => cmp != Greater,
        0o005 => cmp != Less,
        0o006 => cmp != Equal,
        _ => cmp == Greater,
    };
    Ok(if skip { Next::Skip } else { Next::Step })
}

/// MOVSO, MOVST, MOVSLJ, MOVSRJ (MVS to MOVST4).
fn movs(m: &mut Machine, i: Inst, e0: Half, e1: W36, op: u64) -> R {
    let fill = m.read(addr(e0, 1))?;
    let dl = m.ac[acn(i, 3)].get();
    if dl & !LEN != 0 {
        return Err(Fault::Uuo);
    }
    let src = m.ac[i.ac].get();
    let mut flags = src & !LEN;
    let (mut sl, mut dl) = (src & LEN, dl);
    let set_lengths = |m: &mut Machine, flags: u64, sl: u64, dl: u64| {
        m.ac[i.ac] = W36::wrap(flags | sl);
        m.ac[acn(i, 3)] = W36::wrap(dl);
    };
    if op == 0o017 {
        // MOVSRJ: skip the extra source bytes or fill on the left.
        while sl > dl {
            m.ac[acn(i, 1)] = byte::increment(m.ac[acn(i, 1)]);
            sl -= 1;
        }
        while dl > sl {
            idpb(m, acn(i, 4), fill)?;
            dl -= 1;
        }
        // Now the lengths are equal.
        for _ in 0..sl {
            let b = ildb(m, acn(i, 1))?;
            idpb(m, acn(i, 4), b)?;
        }
        set_lengths(m, flags, 0, 0);
        return Ok(Next::Skip);
    }
    // The bits that must be 0 in an offset byte (MVSO): outside S of the
    // destination pointer.
    let ds = m.ac[acn(i, 4)].field(6, 11) as u32;
    let msk = if ds >= 36 {
        0
    } else {
        MASK36 & !((1u64 << ds) - 1)
    };
    while sl > 0 && dl > 0 {
        let b = ildb(m, acn(i, 1))?;
        sl -= 1;
        let out = match op {
            0o014 => {
                let v = b.get().wrapping_add(e1.get()) & MASK36;
                if v & msk != 0 {
                    set_lengths(m, flags, sl, dl);
                    return Ok(Next::Step);
                }
                Some(W36::wrap(v))
            }
            0o015 => {
                let (code, sub) = xlate(m, e1, b)?;
                match translate(code, &mut flags) {
                    Tr::Abort => {
                        set_lengths(m, flags, sl, dl);
                        return Ok(Next::Step);
                    }
                    Tr::Skip => None,
                    Tr::Start => {
                        flags |= S;
                        Some(W36::wrap(sub))
                    }
                    Tr::Take => Some(W36::wrap(sub)),
                }
            }
            _ => Some(b),
        };
        if let Some(x) = out {
            idpb(m, acn(i, 4), x)?;
            dl -= 1;
        }
    }
    if sl > 0 {
        set_lengths(m, flags, sl, 0);
        return Ok(Next::Step);
    }
    while dl > 0 {
        idpb(m, acn(i, 4), fill)?;
        dl -= 1;
    }
    set_lengths(m, flags, 0, 0);
    Ok(Next::Skip)
}

/// CVTDBO, CVTDBT (DBIN to DBNEG). The result is a 71-bit number in AC+3
/// (sign and high 35 bits) and AC+4 (low 35 bits; bit 0 is a sign copy at
/// the end).
fn cvtdb(m: &mut Machine, i: Inst, e1: W36, offset: bool) -> R {
    let a = m.ac[i.ac].get();
    let mut flags = a & !LEN;
    let mut len = a & LEN;
    let (b0, b1) = (acn(i, 3), acn(i, 4));
    m.ac[b1] = W36::wrap(m.ac[b1].get() & !S);
    if flags & S == 0 {
        m.ac[b0] = W36::ZERO;
        m.ac[b1] = W36::ZERO;
    }
    if offset {
        flags |= S;
    }
    let mut ok = true;
    while len > 0 {
        let b = ildb(m, acn(i, 1))?;
        len -= 1;
        let digit = if offset {
            let v = b.get().wrapping_add(e1.get()) & MASK36;
            if v >= 10 {
                ok = false;
                break;
            }
            v
        } else {
            let (code, sub) = xlate(m, e1, b)?;
            match translate(code, &mut flags) {
                Tr::Abort => {
                    ok = false;
                    break;
                }
                Tr::Skip => continue,
                Tr::Start => {
                    flags |= S;
                    sub
                }
                Tr::Take => sub,
            }
        };
        if !offset && digit >= 10 {
            ok = false;
            break;
        }
        let v = (u128::from(m.ac[b0].get()) << 35) | u128::from(m.ac[b1].get() & !S);
        let v = (v * 10 + u128::from(digit)) & ((1 << 71) - 1);
        m.ac[b0] = W36::wrap((v >> 35) as u64);
        m.ac[b1] = W36::wrap(v as u64 & !S);
    }
    if ok && flags & M != 0 {
        // DBNEG: negate the double word.
        let lo = m.ac[b1].get() & !S;
        let nlo = lo.wrapping_neg() & MASK36 & !S;
        let hi = m.ac[b0].get();
        let nhi = if nlo == 0 {
            hi.wrapping_neg() & MASK36
        } else {
            !hi & MASK36
        };
        m.ac[b0] = W36::wrap(nhi);
        m.ac[b1] = W36::wrap(nlo);
    }
    // DBDONE: the sign copy in AC+4; with S clear the result is 0.
    let sign = m.ac[b0].get() & S;
    m.ac[b1] = W36::wrap((m.ac[b1].get() & !S) | sign);
    m.ac[i.ac] = W36::wrap(flags | len);
    if flags & S == 0 {
        m.ac[b0] = W36::ZERO;
        m.ac[b1] = W36::ZERO;
    }
    Ok(if ok { Next::Skip } else { Next::Step })
}

/// CVTBDO, CVTBDT (BDEC to BDSUB).
fn cvtbd(m: &mut Machine, i: Inst, e0: Half, e1: W36, offset: bool) -> R {
    let d = m.ac[acn(i, 3)].get();
    let mut flags = d & 0o777000_000000;
    let len = d & LEN;
    let hi = m.ac[i.ac];
    let lo = m.ac[acn(i, 1)];
    let v = (i128::from(hi.signed()) << 35) | i128::from(lo.get() & !S);
    if v < 0 {
        flags |= M;
    }
    let mag = v.unsigned_abs();
    let text = mag.to_string();
    let digits = text.len() as u64;
    if mag != 0 {
        flags |= N;
    }
    if digits > len {
        return Ok(Next::Step);
    }
    let mut rest = len;
    if flags & S != 0 {
        // L: fill on the left.
        let fill = m.read(addr(e0, 1))?;
        while rest > digits {
            idpb(m, acn(i, 4), fill)?;
            rest -= 1;
            m.ac[acn(i, 3)] = W36::wrap(flags | rest);
        }
    }
    let n = text.len();
    for (k, c) in text.bytes().enumerate() {
        let dg = u64::from(c - b'0');
        let x = if offset {
            dg.wrapping_add(e1.get()) & MASK36
        } else {
            let w = m.read(Half::wrap(e1.get() + dg))?;
            let last = k + 1 == n;
            if last && flags & M != 0 {
                w.lh().get()
            } else {
                w.rh().get()
            }
        };
        idpb(m, acn(i, 4), W36::wrap(x))?;
    }
    m.ac[i.ac] = W36::ZERO;
    m.ac[acn(i, 1)] = W36::ZERO;
    m.ac[acn(i, 3)] = W36::wrap(flags);
    Ok(Next::Skip)
}

/// EDIT (EDIT to EDNOP2).
fn edit(m: &mut Machine, i: Inst, e0: Half, e1: W36) -> R {
    let fill = m.read(addr(e0, 1))?;
    if m.ac[i.ac].get() & 0o047777_000000 != 0 {
        return Err(Fault::Uuo);
    }
    let (src, mark, dst) = (acn(i, 1), acn(i, 3), acn(i, 4));
    loop {
        let a = m.ac[i.ac].get();
        let mut flags = a & (S | N | M);
        let pbn = (a >> 30) & 3;
        let pw = m.read(Half::wrap(a))?.get();
        let cmd = (pw >> (27 - 9 * pbn)) & 0o777;
        // The pattern bytes to advance after this command.
        let mut step = 1;
        let mut stop = None;
        match cmd >> 6 {
            0 => match cmd {
                0 => stop = Some(Next::Skip),
                1 => {
                    let b = ildb(m, src)?;
                    let (code, sub) = xlate(m, e1, b)?;
                    match translate(code, &mut flags) {
                        Tr::Abort => stop = Some(Next::Step),
                        Tr::Take => idpb(m, dst, W36::wrap(sub))?,
                        Tr::Skip => {
                            if fill != W36::ZERO {
                                idpb(m, dst, fill)?;
                            }
                        }
                        Tr::Start => {
                            start_sig(m, e0, mark, dst, &mut flags)?;
                            idpb(m, dst, W36::wrap(sub))?;
                        }
                    }
                }
                2 => {
                    if flags & S == 0 {
                        start_sig(m, e0, mark, dst, &mut flags)?;
                    }
                }
                3 => flags = 0,
                4 => {
                    let ma = m.ac[mark].rh();
                    let old = m.read(ma)?;
                    m.write(ma, m.ac[dst])?;
                    m.ac[dst] = old;
                }
                _ => {}
            },
            1 => {
                if flags & S != 0 {
                    let c = m.read(addr(e0, (cmd & 0o77) + 1))?;
                    idpb(m, dst, c)?;
                } else if fill != W36::ZERO {
                    idpb(m, dst, fill)?;
                }
            }
            5 if flags & M != 0 => step += (cmd & 0o77) + 1,
            6 if flags & N != 0 => step += (cmd & 0o77) + 1,
            7 => step += (cmd & 0o77) + 1,
            _ => {}
        }
        // Advance the pattern pointer (EDNOP1, EDSTOP).
        let idx = pbn + step;
        let pa = Half::wrap(a + (idx >> 2));
        let w = (flags & (S | N | M)) | ((idx & 3) << 30) | pa.get();
        m.ac[i.ac] = W36::wrap(w);
        if let Some(n) = stop {
            return Ok(n);
        }
    }
}

/// Start significance (EDFLT): store the destination pointer at the mark
/// address, then the float character from E0+2 if that word is not 0, and
/// set S.
fn start_sig(
    m: &mut Machine,
    e0: Half,
    mark: usize,
    dst: usize,
    flags: &mut u64,
) -> Result<(), Fault> {
    m.write(m.ac[mark].rh(), m.ac[dst])?;
    let fl = m.read(addr(e0, 2))?;
    if fl != W36::ZERO {
        idpb(m, dst, fl)?;
    }
    *flags |= S;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 7-bit ASCII pointer before the first byte of word `a`.
    fn ptr7(a: u64) -> W36 {
        W36::wrap(0o440700_000000 | a)
    }

    fn put_text(m: &mut Machine, a: usize, t: &[u8]) {
        for (k, ch) in t.chunks(5).enumerate() {
            let mut w = 0u64;
            for j in 0..5 {
                w = (w << 7) | u64::from(*ch.get(j).unwrap_or(&0));
            }
            m.mem[a + k] = W36::wrap(w << 1);
        }
    }

    fn run(m: &mut Machine, e0w: u64) -> Next {
        m.mem[0o200] = W36::wrap(e0w);
        let i = Inst::new(W36::wrap(0o123100_000200)); // EXTEND 2,200
        execute(m, i, Half::wrap(0o200)).unwrap()
    }

    // PRM 2.12, MOVSLJ: "If any source bytes are left over ... go to the
    // next instruction. Otherwise place the fill character ... and skip".
    #[test]
    fn movslj() {
        let mut m = Machine::new();
        put_text(&mut m, 0o1000, b"ABC");
        m.mem[0o201] = W36::wrap(0o40); // fill: space
        m.ac[2] = W36::wrap(3);
        m.ac[3] = ptr7(0o1000);
        m.ac[5] = W36::wrap(5);
        m.ac[6] = ptr7(0o2000);
        assert!(matches!(run(&mut m, 0o016000_000000), Next::Skip));
        let mut want = Machine::new();
        put_text(&mut want, 0o2000, b"ABC  ");
        assert_eq!(m.mem[0o2000], want.mem[0o2000]);
        assert_eq!((m.ac[2].get(), m.ac[5].get()), (0, 0));
        // Source longer: no skip, 2 bytes left.
        let mut m = Machine::new();
        put_text(&mut m, 0o1000, b"ABCDE");
        m.ac[2] = W36::wrap(5);
        m.ac[3] = ptr7(0o1000);
        m.ac[5] = W36::wrap(3);
        m.ac[6] = ptr7(0o2000);
        assert!(matches!(run(&mut m, 0o016000_000000), Next::Step));
        assert_eq!((m.ac[2].get(), m.ac[5].get()), (2, 0));
    }

    #[test]
    fn cmps_and_uuo() {
        let mut m = Machine::new();
        put_text(&mut m, 0o1000, b"AB");
        put_text(&mut m, 0o2000, b"AC");
        m.ac[2] = W36::wrap(2);
        m.ac[3] = ptr7(0o1000);
        m.ac[5] = W36::wrap(2);
        m.ac[6] = ptr7(0o2000);
        // CMPSL: "AB" < "AC".
        assert!(matches!(run(&mut m, 0o001000_000000), Next::Skip));
        assert_eq!((m.ac[2].get(), m.ac[5].get()), (0, 0));
        // E0 with AC not 0: a UUO.
        m.mem[0o200] = W36::wrap(0o001040_000000);
        let i = Inst::new(W36::wrap(0o123100_000200));
        assert!(matches!(
            execute(&mut m, i, Half::wrap(0o200)),
            Err(Fault::Uuo)
        ));
    }

    // PRM 2.13: CVTBDO with offset 60 gives ASCII digits; CVTDBO with
    // offset -60 the reverse.
    #[test]
    fn decimal() {
        let mut m = Machine::new();
        m.ac[2] = W36::ZERO;
        m.ac[3] = W36::wrap(1234);
        m.ac[5] = W36::wrap(4);
        m.ac[6] = ptr7(0o2000);
        assert!(matches!(run(&mut m, 0o012000_000060), Next::Skip));
        let mut want = Machine::new();
        put_text(&mut want, 0o2000, b"1234");
        assert_eq!(m.mem[0o2000], want.mem[0o2000]);
        assert_eq!(m.ac[5].get(), N);
        // Too few positions: no change, no skip.
        m.ac[3] = W36::wrap(12345);
        m.ac[5] = W36::wrap(4);
        assert!(matches!(run(&mut m, 0o012000_000060), Next::Step));
        assert_eq!(m.ac[3].get(), 12345);
        // Back to binary: offset -60 (777720 sign extended).
        let mut m = Machine::new();
        put_text(&mut m, 0o1000, b"-42");
        // Start after the "-": the pointer is at the first byte.
        m.ac[3] = byte::increment(ptr7(0o1000));
        m.ac[2] = W36::wrap(M | 2);
        assert!(matches!(run(&mut m, 0o010000_777720), Next::Skip));
        // -42 as a double word.
        let v = (i128::from(m.ac[5].signed()) << 35) | i128::from(m.ac[6].get() & !S);
        assert_eq!(v, -42);
    }

    // extend.4, CMS7: the bytes are compared as unsigned numbers. 36-bit
    // bytes 400000,,0 and 1: the first is greater.
    #[test]
    fn cmps_unsigned() {
        let mut m = Machine::new();
        m.mem[0o1000] = W36::wrap(0o400000_000000);
        m.mem[0o2000] = W36::wrap(1);
        m.ac[2] = W36::wrap(1);
        m.ac[3] = W36::wrap(0o004400_000777); // POINT 36,777: the next byte is 1000
        m.ac[5] = W36::wrap(1);
        m.ac[6] = W36::wrap(0o004400_001777);
        assert!(matches!(run(&mut m, 0o007000_000000), Next::Skip)); // CMPSG
    }

    // PRM 2.13, CVTBDT: the last digit of a negative number comes from the
    // left half of the table entry.
    #[test]
    fn cvtbdt_last_digit() {
        let mut m = Machine::new();
        for d in 0..10u64 {
            m.mem[0o3000 + d as usize] =
                W36::from_halves(Half::wrap(0o100 + d), Half::wrap(0o60 + d));
        }
        // -12 as a double word.
        m.ac[2] = W36::wrap(0o777777_777777);
        m.ac[3] = W36::wrap(0o777777_777764);
        m.ac[5] = W36::wrap(2);
        m.ac[6] = ptr7(0o2000);
        assert!(matches!(run(&mut m, 0o013000_003000), Next::Skip));
        let mut want = Machine::new();
        put_text(&mut want, 0o2000, &[0o61, 0o102]);
        assert_eq!(m.mem[0o2000], want.mem[0o2000]);
        assert_eq!(m.ac[5].get(), N | M);
        // Positive: the right half for every digit.
        m.ac[2] = W36::ZERO;
        m.ac[3] = W36::wrap(12);
        m.ac[5] = W36::wrap(2);
        m.ac[6] = ptr7(0o2001);
        assert!(matches!(run(&mut m, 0o013000_003000), Next::Skip));
        put_text(&mut want, 0o2001, &[0o61, 0o62]);
        assert_eq!(m.mem[0o2001], want.mem[0o2001]);
    }
}

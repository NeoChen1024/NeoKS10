//! The instructions of the software processor.
//!
//! The families decode their variants from the opcode bits as the PRM
//! describes them (Processor Reference Manual, 1982, chapter 2). Which
//! opcodes the ITS microcode implements comes from spec/instructions.toml
//! (class `inst`, `luuo`, `uuo`); `Machine::execute` sends only implemented
//! opcodes here.

use pdp10::{Half, W36};

use crate::flags::Flags;
use crate::machine::{Fault, Inst, Machine, Next};
use crate::{apr, shift};

type R = Result<Next, Fault>;

const ONE: W36 = W36::from_halves(Half::ZERO, Half::wrap(1));

/// The operand of an immediate instruction: 0,,E.
fn imm(e: Half) -> W36 {
    W36::from_halves(Half::ZERO, e)
}

/// The skip and jump conditions of bits 6 to 8 (PRM 2.6, "Arithmetic
/// Testing"): never, L, E, LE, A, GE, N, G, of a signed comparison.
fn cond(c: u16, a: W36, b: W36) -> bool {
    let (a, b) = (a.signed(), b.signed());
    match c & 7 {
        0 => false,
        1 => a < b,
        2 => a == b,
        3 => a <= b,
        4 => true,
        5 => a >= b,
        6 => a != b,
        _ => a > b,
    }
}

pub fn execute(m: &mut Machine, i: Inst, e: Half, last: W36) -> R {
    match i.op {
        0o200..=0o217 => moves(m, i, e),
        0o240..=0o247 => shift::execute(m, i, e),
        0o251 => shift::blt(m, i, e),
        0o700 => apr::op700(m, i, e),
        0o250 => {
            // EXCH (PRM 2.1)
            let x = m.read(e)?;
            m.write(e, m.ac[i.ac])?;
            m.ac[i.ac] = x;
            Ok(Next::Step)
        }
        0o252 | 0o253 => aobj(m, i, e),
        0o254 => jrst(m, i, e, last),
        0o255 => jfcl(m, i, e),
        0o256 => Ok(Next::Execute(m.read(e)?)),
        0o260..=0o267 => calls(m, i, e),
        0o270..=0o277 => add_sub(m, i, e),
        0o300..=0o377 => arith_test(m, i, e),
        0o400..=0o477 => boolean(m, i, e),
        0o500..=0o577 => half(m, i, e),
        0o600..=0o677 => logic_test(m, i, e),
        _ => Err(Fault::Unimplemented),
    }
}

/// Store a result by the mode in bits 7 and 8: 0 and 1 to AC, 2 to E,
/// 3 to AC and E (PRM 2.1).
fn store4(m: &mut Machine, i: Inst, e: Half, w: W36) -> R {
    match i.op & 3 {
        0 | 1 => m.ac[i.ac] = w,
        2 => m.write(e, w)?,
        _ => {
            m.write(e, w)?;
            m.ac[i.ac] = w;
        }
    }
    Ok(Next::Step)
}

/// The operand by the mode: C(E), 0,,E for mode 1, AC for mode 2 (to
/// memory), C(E) for mode 3.
fn operand4(m: &Machine, i: Inst, e: Half) -> Result<W36, Fault> {
    Ok(match i.op & 3 {
        1 => imm(e),
        _ => m.read(e)?,
    })
}

/// MOVE, MOVS, MOVN, MOVM (PRM 2.1, "Move Instructions"). The modes are
/// basic, immediate, memory (from AC to E) and self (E to E, and to AC if
/// A is not 0).
fn moves(m: &mut Machine, i: Inst, e: Half) -> R {
    let src = match i.op & 3 {
        0 => m.read(e)?,
        1 => imm(e),
        2 => m.ac[i.ac],
        _ => m.read(e)?,
    };
    let w = match (i.op >> 2) & 3 {
        0 => src,
        1 => src.swap(),
        2 => {
            // MOVN: negate; the carries and overflow of 0 - src.
            let s = src.neg();
            m.flags.arith(s.cry0, s.cry1);
            s.word
        }
        _ => {
            // MOVM: magnitude; only the most negative number changes the flags.
            if src.is_negative() {
                let s = src.neg();
                m.flags.arith(s.cry0, s.cry1);
                s.word
            } else {
                src
            }
        }
    };
    match i.op & 3 {
        0 | 1 => m.ac[i.ac] = w,
        2 => m.write(e, w)?,
        _ => {
            m.write(e, w)?;
            if i.ac != 0 {
                m.ac[i.ac] = w;
            }
        }
    }
    Ok(Next::Step)
}

/// ADD and SUB (PRM 2.2), modes basic, immediate, memory, both.
fn add_sub(m: &mut Machine, i: Inst, e: Half) -> R {
    let b = operand4(m, i, e)?;
    let a = m.ac[i.ac];
    let s = if i.op & 4 == 0 { a.add(b) } else { a.sub(b) };
    m.flags.arith(s.cry0, s.cry1);
    store4(m, i, e, s.word)
}

/// The 16 Boolean functions of bits 3 to 6 (PRM 2.4), of A = AC and M.
fn boolean(m: &mut Machine, i: Inst, e: Half) -> R {
    let mm = operand4(m, i, e)?;
    let a = m.ac[i.ac];
    let w = match (i.op >> 2) & 0o17 {
        0o00 => W36::ZERO,
        0o01 => a.and(mm),
        0o02 => a.not().and(mm),
        0o03 => mm,
        0o04 => a.and(mm.not()),
        0o05 => a,
        0o06 => a.xor(mm),
        0o07 => a.or(mm),
        0o10 => a.not().and(mm.not()),
        0o11 => a.xor(mm).not(),
        0o12 => a.not(),
        0o13 => a.not().or(mm),
        0o14 => mm.not(),
        0o15 => a.or(mm.not()),
        0o16 => a.not().or(mm.not()),
        _ => W36::ONES,
    };
    store4(m, i, e, w)
}

/// Half word data transmission (PRM 2.8). Bit 3: the destination half is
/// right (1) or left (0); bit 6: the source half is the other one; bits 4
/// and 5: the other half of the destination is unchanged, zeros, ones, or
/// the extended sign of the moved half. Modes: basic (C(E) to AC),
/// immediate (0,,E to AC), memory (AC to E), self (C(E) to E, and AC if A
/// is not 0).
fn half(m: &mut Machine, i: Inst, e: Half) -> R {
    let to_right = i.op & 0o40 != 0;
    let swap = i.op & 0o4 != 0;
    let ext = (i.op >> 3) & 3;
    let mode = i.op & 3;
    let src = match mode {
        1 => imm(e),
        2 => m.ac[i.ac],
        _ => m.read(e)?,
    };
    let dest = match mode {
        0 | 1 => m.ac[i.ac],
        _ => m.read(e)?,
    };
    // The source half: left for HLx and HRx when not swapped is the half
    // of the same side as the destination.
    let from_left = to_right == swap;
    let h = if from_left { src.lh() } else { src.rh() };
    let other = match ext {
        0 => {
            if to_right {
                dest.lh()
            } else {
                dest.rh()
            }
        }
        1 => Half::ZERO,
        2 => Half::ONES,
        _ => {
            if h.is_negative() {
                Half::ONES
            } else {
                Half::ZERO
            }
        }
    };
    let w = if to_right {
        W36::from_halves(other, h)
    } else {
        W36::from_halves(h, other)
    };
    match mode {
        0 | 1 => m.ac[i.ac] = w,
        2 => m.write(e, w)?,
        _ => {
            m.write(e, w)?;
            if i.ac != 0 {
                m.ac[i.ac] = w;
            }
        }
    }
    Ok(Next::Step)
}

/// CAI, CAM, JUMP, SKIP, AOJ, AOS, SOJ, SOS (PRM 2.6).
fn arith_test(m: &mut Machine, i: Inst, e: Half) -> R {
    let c = i.op & 7;
    let skip = |b: bool| if b { Next::Skip } else { Next::Step };
    let jump = |b: bool| if b { Next::Jump(e) } else { Next::Step };
    match (i.op >> 3) & 7 {
        0 => Ok(skip(cond(c, m.ac[i.ac], imm(e)))),
        1 => Ok(skip(cond(c, m.ac[i.ac], m.read(e)?))),
        2 => Ok(jump(cond(c, m.ac[i.ac], W36::ZERO))),
        3 => {
            let x = m.read(e)?;
            if i.ac != 0 {
                m.ac[i.ac] = x;
            }
            Ok(skip(cond(c, x, W36::ZERO)))
        }
        g => {
            // 4 AOJ, 5 AOS, 6 SOJ, 7 SOS.
            let in_ac = g == 4 || g == 6;
            let x = if in_ac { m.ac[i.ac] } else { m.read(e)? };
            let s = if g < 6 { x.add(ONE) } else { x.sub(ONE) };
            m.flags.arith(s.cry0, s.cry1);
            if in_ac {
                m.ac[i.ac] = s.word;
                Ok(jump(cond(c, s.word, W36::ZERO)))
            } else {
                m.write(e, s.word)?;
                if i.ac != 0 {
                    m.ac[i.ac] = s.word;
                }
                Ok(skip(cond(c, s.word, W36::ZERO)))
            }
        }
    }
}

/// Logical testing and modification (PRM 2.7). Bit 8: the mask is
/// swapped (L, S); bit 5: the mask is C(E) (D, S), else E (R, L); bits 6
/// and 7: skip never, if all masked bits are 0 (E), always, if not all are
/// 0 (N); bits 3 and 4: the masked bits are unchanged, zeroed, complemented
/// or set (N, Z, C, O). The test uses AC before the modification.
fn logic_test(m: &mut Machine, i: Inst, e: Half) -> R {
    let mut mask = if i.op & 0o10 != 0 { m.read(e)? } else { imm(e) };
    if i.op & 1 != 0 {
        mask = mask.swap();
    }
    let a = m.ac[i.ac];
    let zero = a.and(mask) == W36::ZERO;
    let skip = match (i.op >> 1) & 3 {
        0 => false,
        1 => zero,
        2 => true,
        _ => !zero,
    };
    match (i.op >> 4) & 3 {
        0 => {}
        1 => m.ac[i.ac] = a.and(mask.not()),
        2 => m.ac[i.ac] = a.xor(mask),
        _ => m.ac[i.ac] = a.or(mask),
    }
    Ok(if skip { Next::Skip } else { Next::Step })
}

/// Add 1 to each half (`d` = 1) or subtract 1 from each half (`d` = -1),
/// with no carry between the halves (PRM 2.10, footnote 25: the KA10 alone
/// carries between them).
fn halves_step(w: W36, d: i64) -> W36 {
    let l = Half::from_signed(w.lh().signed() + d);
    let r = Half::from_signed(w.rh().signed() + d);
    W36::from_halves(l, r)
}

/// AOBJP, AOBJN (PRM 2.6): add 1 to each half of AC, then jump if AC is
/// positive (AOBJP) or negative (AOBJN).
fn aobj(m: &mut Machine, i: Inst, e: Half) -> R {
    let w = halves_step(m.ac[i.ac], 1);
    m.ac[i.ac] = w;
    let neg = w.is_negative();
    Ok(if neg == (i.op == 0o253) {
        Next::Jump(e)
    } else {
        Next::Step
    })
}

/// The flags that a subroutine jump clears (PRM 2.9, JSP).
const CALL_CLEARS: u32 = Flags::FPD | Flags::AFI | Flags::TRAP1 | Flags::TRAP2;

/// PUSHJ, PUSH, POP, POPJ, JSR, JSP, JSA, JRA (PRM 2.9, 2.10).
fn calls(m: &mut Machine, i: Inst, e: Half) -> R {
    let ret = Half::wrap(m.pc.get() + 1);
    match i.op {
        0o260 | 0o261 => {
            // PUSHJ, PUSH
            let p = halves_step(m.ac[i.ac], 1);
            let value = if i.op == 0o260 {
                m.flags.pc_word(ret)
            } else {
                m.read(e)?
            };
            m.ac[i.ac] = p;
            m.write(p.rh(), value)?;
            if p.lh() == Half::ZERO {
                m.flags.set(Flags::TRAP2);
            }
            if i.op == 0o260 {
                m.flags
                    .clear(Flags::FPD | Flags::AFI | Flags::TRAP1 | Flags::TRAP2);
                if p.lh() == Half::ZERO {
                    m.flags.set(Flags::TRAP2);
                }
                return Ok(Next::Jump(e));
            }
            Ok(Next::Step)
        }
        0o262 | 0o263 => {
            // POP, POPJ
            let old = m.ac[i.ac];
            let value = m.read(old.rh())?;
            let p = halves_step(old, -1);
            m.ac[i.ac] = p;
            if i.op == 0o262 {
                m.write(e, value)?;
            }
            if p.lh() == Half::ONES {
                m.flags.set(Flags::TRAP2);
            }
            Ok(if i.op == 0o263 {
                Next::Jump(value.rh())
            } else {
                Next::Step
            })
        }
        0o264 => {
            // JSR: the PC word to E, jump to E+1.
            m.write(e, m.flags.pc_word(ret))?;
            m.flags.clear(CALL_CLEARS);
            Ok(Next::Jump(Half::wrap(e.get() + 1)))
        }
        0o265 => {
            // JSP: the PC word to AC, jump to E.
            m.ac[i.ac] = m.flags.pc_word(ret);
            m.flags.clear(CALL_CLEARS);
            Ok(Next::Jump(e))
        }
        0o266 => {
            // JSA: AC to E, E,,PC+1 to AC, jump to E+1.
            m.write(e, m.ac[i.ac])?;
            m.ac[i.ac] = W36::from_halves(e, ret);
            Ok(Next::Jump(Half::wrap(e.get() + 1)))
        }
        _ => {
            // JRA: AC from the location of AC left, jump to E.
            let a = m.ac[i.ac].lh();
            m.ac[i.ac] = m.read(a)?;
            Ok(Next::Jump(e))
        }
    }
}

/// JRST (PRM 2.9, "The JRST Instruction"); the variants by AC from the
/// dispatch of the ITS microcode (spec/instructions.toml, acdisp 254).
fn jrst(m: &mut Machine, i: Inst, e: Half, last: W36) -> R {
    match i.ac {
        0 | 1 => Ok(Next::Jump(e)),
        2 => {
            // JRSTF: restore the flags from bits 0 to 12 of the last word
            // of the address calculation (C(X) if indexed). In user mode
            // the User flag stays set, and User In-out cannot be set.
            let mut f = (last.lh().get() as u32) & Flags::ALL;
            if m.flags.is_user() {
                f |= Flags::USER;
                if !m.flags.has(Flags::USERIO) {
                    f &= !Flags::USERIO;
                }
            }
            m.flags = Flags(f);
            Ok(Next::Jump(e))
        }
        4 => {
            if m.flags.is_user() {
                return Err(Fault::Unimplemented); // a monitor UUO (later)
            }
            // [microcode] simple.42, HALT: PC <- E; inout.50, HALTED: halt
            // code 1 to physical location 0, the PC to location 1. The halt
            // status block of the microcode registers is not written.
            m.pc = e;
            m.deposit(0, ONE)?;
            m.deposit(1, W36::from_halves(Half::ZERO, e))?;
            Ok(Next::Stop(crate::machine::Stop::Halt { pc: e }))
        }
        _ => Err(Fault::Unimplemented),
    }
}

/// JFCL (PRM 2.9): AC bits 9 to 12 select Overflow, Carry 0, Carry 1 and
/// Floating Overflow. If a selected flag is set, clear the selected flags
/// and jump.
fn jfcl(m: &mut Machine, i: Inst, e: Half) -> R {
    let mut sel = 0;
    for (bit, f) in [
        (8, Flags::AROV),
        (4, Flags::CRY0),
        (2, Flags::CRY1),
        (1, Flags::FOV),
    ] {
        if i.ac & bit != 0 {
            sel |= f;
        }
    }
    if m.flags.0 & sel != 0 {
        m.flags.clear(sel);
        Ok(Next::Jump(e))
    } else {
        Ok(Next::Step)
    }
}

/// A local UUO (opcodes 001 to 037; PRM 2.16): opcode, A and E to location
/// 40 (bits 13 to 17 zero), then the instruction in location 41.
pub fn luuo(m: &mut Machine, i: Inst, e: Half) -> R {
    let w = W36::from_halves(Half::wrap(i.word.lh().get() & 0o777740), e);
    m.write(Half::wrap(0o40), w)?;
    Ok(Next::Execute(m.read(Half::wrap(0o41))?))
}

#[cfg(test)]
mod tests {
    use pdp10::instructions::OPS;

    // The decode of the families in this module, written as a mnemonic,
    // must give the name that MIDAS gives (spec/instructions.toml, from
    // ISYMTB): an independent source of the bit assignments.
    fn mode(op: u16, names: [&str; 4]) -> &str {
        names[usize::from(op & 3)]
    }

    #[test]
    fn half_word_names() {
        for op in 0o500..=0o577u16 {
            let to_right = op & 0o40 != 0;
            let swap = op & 0o4 != 0;
            let from_left = to_right == swap;
            let ext = ["", "Z", "O", "E"][usize::from((op >> 3) & 3)];
            let name = format!(
                "H{}{}{}{}",
                if from_left { "L" } else { "R" },
                if to_right { "R" } else { "L" },
                ext,
                mode(op, ["", "I", "M", "S"])
            );
            assert_eq!(OPS[usize::from(op)].name, name, "{op:o}");
        }
    }

    #[test]
    fn logic_test_names() {
        for op in 0o600..=0o677u16 {
            let mask = match (op & 0o10 != 0, op & 1 != 0) {
                (false, false) => "R",
                (false, true) => "L",
                (true, false) => "D",
                (true, true) => "S",
            };
            let modi = ["N", "Z", "C", "O"][usize::from((op >> 4) & 3)];
            let skip = ["", "E", "A", "N"][usize::from((op >> 1) & 3)];
            assert_eq!(
                OPS[usize::from(op)].name,
                format!("T{mask}{modi}{skip}"),
                "{op:o}"
            );
        }
    }

    #[test]
    fn arith_test_names() {
        let conds = ["", "L", "E", "LE", "A", "GE", "N", "G"];
        let groups = ["CAI", "CAM", "JUMP", "SKIP", "AOJ", "AOS", "SOJ", "SOS"];
        for op in 0o300..=0o377u16 {
            let name = format!(
                "{}{}",
                groups[usize::from((op >> 3) & 7)],
                conds[usize::from(op & 7)]
            );
            assert_eq!(OPS[usize::from(op)].name, name, "{op:o}");
        }
    }

    #[test]
    fn boolean_and_move_names() {
        let f = [
            "SETZ", "AND", "ANDCA", "SETM", "ANDCM", "SETA", "XOR", "IOR", "ANDCB", "EQV", "SETCA",
            "ORCA", "SETCM", "ORCM", "ORCB", "SETO",
        ];
        for op in 0o400..=0o477u16 {
            let name = format!(
                "{}{}",
                f[usize::from((op >> 2) & 0o17)],
                mode(op, ["", "I", "M", "B"])
            );
            assert_eq!(OPS[usize::from(op)].name, name, "{op:o}");
        }
        for op in 0o200..=0o217u16 {
            let base = ["MOVE", "MOVS", "MOVN", "MOVM"][usize::from((op >> 2) & 3)];
            let name = format!("{base}{}", mode(op, ["", "I", "M", "S"]));
            assert_eq!(OPS[usize::from(op)].name, name, "{op:o}");
        }
        for op in 0o270..=0o277u16 {
            let base = if op & 4 == 0 { "ADD" } else { "SUB" };
            assert_eq!(
                OPS[usize::from(op)].name,
                format!("{base}{}", mode(op, ["", "I", "M", "B"]))
            );
        }
    }
}

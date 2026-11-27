//! One instruction on the model, compared with the software processor
//! `ks10-core-soft` (docs/microcode-model.md, "Single instructions").
//!
//! The model runs the power-up sequence into the halt loop. The test then
//! puts the instruction at 1000, the operands at 2000 and 2001 and the
//! ACs into block 0 of the RAM file, and sets the PC register of the 2901
//! to 1000. With CONTINUE set and RUN clear, the microcode does one
//! instruction and halts: it stores the halt code 2 in location 0 and PC
//! in location 1 (inout, HALTLP, CONT, NICOND code 5, HALTED, PWRON).
//!
//! `ks10-core-soft` runs the same instruction from the same state. The test compares
//! the ACs, PC, the program flags and the words at 2000 and 2001.
//!
//! `ks10-core-soft` does not copy the microcode: the two can differ by a
//! decision. Each such difference is an entry of [`KNOWN`], with the
//! section of docs/emulator.md ("Differences from the microcode") that
//! gives the reason. A new difference is a finding: record it there and
//! here, or correct the model or `ks10-core-soft`. It is not a value to
//! adjust (AGENTS.md, "Tests").

mod common;

use common::{FILLS, label, machine, run_until, symbol};
use ks10_core_soft::flags::Flags;
use ks10_core_uop::alu;
use ks10_core_uop::machine::Machine;
use pdp10::{Half, W36};

const CODE: usize = 0o1000;
const E: usize = 0o2000;

/// A case: the instruction word (its E is 2000, 2001 or an AC), the ACs,
/// the words at 2000 and 2001, and the program flags before the
/// instruction (in the form of `ks10-core-soft`).
#[derive(Clone)]
struct Case {
    name: String,
    inst: u64,
    ac: [u64; 16],
    mem: [u64; 2],
    flags: u32,
}

/// The state after the instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    pc: u64,
    flags: u32,
    ac: [u64; 16],
    mem: [u64; 2],
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PC {:o} flags {:o} mem {:o} {:o} AC",
            self.pc, self.flags, self.mem[0], self.mem[1]
        )?;
        for a in self.ac {
            write!(f, " {a:o}")?;
        }
        Ok(())
    }
}

fn reference(c: &Case) -> Result<State, String> {
    let mut m = ks10_core_soft::Machine::new();
    for (k, &a) in c.ac.iter().enumerate() {
        m.ac[k] = W36::wrap(a);
    }
    m.mem[CODE] = W36::wrap(c.inst);
    m.mem[E] = W36::wrap(c.mem[0]);
    m.mem[E + 1] = W36::wrap(c.mem[1]);
    m.flags = ks10_core_soft::flags::Flags(c.flags);
    m.pc = Half::wrap(CODE as u64);
    m.step().map_err(|s| format!("{s:?}"))?;
    Ok(State {
        pc: m.pc.get(),
        flags: m.flags.0,
        ac: m.ac.map(|w| w.get()),
        mem: [m.mem[E].get(), m.mem[E + 1].get()],
    })
}

/// The model in the halt loop after power-up.
fn halted(fill: alu::W40) -> Machine {
    let haltlp = label("HALTLP");
    let mut m = machine(fill);
    run_until(&mut m, 100_000, |m| m.upc == haltlp && m.csl.halt_loop).unwrap();
    m
}

fn model(fill: alu::W40, c: &Case) -> Result<State, String> {
    let haltlp = label("HALTLP");
    // A machine has its own bus with the devices, so each case runs the
    // power-up sequence (about 640 microcycles).
    let mut m = halted(fill);
    for (k, &a) in c.ac.iter().enumerate() {
        m.ram_file[k] = W36::wrap(a);
    }
    m.memory[CODE] = W36::wrap(c.inst);
    m.memory[E] = W36::wrap(c.mem[0]);
    m.memory[E + 1] = W36::wrap(c.mem[1]);
    // Design decision 2026-11-27: the test sets PC in the 2901 register.
    // The console starts a program with EXECUTE of a JRST (cmds.m80,
    // STCMD, EXINT), which reads the console register with an I/O cycle.
    m.alu.reg[symbol("A", "PC")] = alu::d_input(W36::wrap(CODE as u64));
    // One tick of the 1 ms timer with 1 MSEC EN clear clears its flag.
    m.millisecond();
    m.flags = common::pc_flags(c.flags);
    // The "SI" command: CONTINUE without RUN (cmds.m80, SICMD: "I/O WRT
    // 212/1").
    m.csl.cont = true;
    m.csl.run = false;
    // Leave the halt loop, then run to its next entry.
    run_until(&mut m, 50, |m| !m.csl.halt_loop)?;
    run_until(&mut m, 20_000, |m| m.upc == haltlp && m.csl.halt_loop)?;
    if m.memory[0] != W36::wrap(2) {
        return Err(format!("halt code {:o}", m.memory[0].get()));
    }
    // The halt status block at 500 (inout, DUMP; simple, "ITS makes it
    // 500"): PC, ONE, MASK, XWD1 and T1 with the halt code.
    let hsb = |k: usize| m.memory[0o500 + k].get();
    let want = [
        (0o1, m.memory[1].get()),
        (0o7, 1),
        (0o12, 0o777777_777777),
        (0o15, 0o000001_000001),
        (0o17, 2),
    ];
    for (k, v) in want {
        if hsb(k) != v {
            return Err(format!("halt status block +{k:o}: {:o}, not {v:o}", hsb(k)));
        }
    }
    Ok(State {
        pc: m.memory[1].get() & 0o777777,
        flags: common::ks10_flags(m.flags),
        ac: std::array::from_fn(|k| m.ram_file[k].get()),
        mem: [m.memory[E].get(), m.memory[E + 1].get()],
    })
}

/// An instruction word: opcode, AC and E.
fn i(op: u64, ac: u64, e: u64) -> u64 {
    (op << 27) | (ac << 23) | e
}

fn acs(v: &[(usize, u64)]) -> [u64; 16] {
    let mut a = [0; 16];
    for &(k, w) in v {
        a[k] = w;
    }
    a
}

fn cases() -> Vec<Case> {
    const MAX: u64 = 0o377777_777777;
    const MIN: u64 = 0o400000_000000;
    const M1: u64 = 0o777777_777777;
    let e = E as u64;
    vec![
        Case {
            name: "MOVE".into(),
            inst: i(0o200, 1, e),
            ac: acs(&[]),
            mem: [0o123456_654321, 0],
            flags: 0,
        },
        Case {
            name: "MOVEI".into(),
            inst: i(0o201, 2, 0o1234),
            ac: acs(&[]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "MOVEM".into(),
            inst: i(0o202, 3, e),
            ac: acs(&[(3, 0o707070_070707)]),
            mem: [5, 0],
            flags: 0,
        },
        Case {
            name: "MOVE from an AC".into(),
            inst: i(0o200, 1, 4),
            ac: acs(&[(4, 0o4444)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "MOVEM to an AC".into(),
            inst: i(0o202, 5, 6),
            ac: acs(&[(5, 0o5555)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "ADD".into(),
            inst: i(0o270, 1, e),
            ac: acs(&[(1, 5)]),
            mem: [7, 0],
            flags: 0,
        },
        Case {
            name: "ADD overflow".into(),
            inst: i(0o270, 1, e),
            ac: acs(&[(1, MAX)]),
            mem: [1, 0],
            flags: 0,
        },
        Case {
            name: "ADD carries".into(),
            inst: i(0o270, 1, e),
            ac: acs(&[(1, M1)]),
            mem: [M1, 0],
            flags: 0,
        },
        Case {
            name: "SUB".into(),
            inst: i(0o274, 1, e),
            ac: acs(&[(1, 5)]),
            mem: [7, 0],
            flags: 0,
        },
        Case {
            name: "SUB overflow".into(),
            inst: i(0o274, 1, e),
            ac: acs(&[(1, MIN)]),
            mem: [1, 0],
            flags: 0,
        },
        Case {
            name: "ADDB".into(),
            inst: i(0o273, 7, e),
            ac: acs(&[(7, 0o100)]),
            mem: [0o23, 0],
            flags: 0,
        },
        Case {
            name: "AND".into(),
            inst: i(0o404, 2, e),
            ac: acs(&[(2, 0o707070_707070)]),
            mem: [0o777000_000777, 0],
            flags: 0,
        },
        Case {
            name: "IOR".into(),
            inst: i(0o434, 2, e),
            ac: acs(&[(2, 0o700000_000007)]),
            mem: [0o000077_770000, 0],
            flags: 0,
        },
        Case {
            name: "SETZM".into(),
            inst: i(0o402, 0, e),
            ac: acs(&[]),
            mem: [M1, 0],
            flags: 0,
        },
        Case {
            name: "AOS".into(),
            inst: i(0o350, 0, e),
            ac: acs(&[]),
            mem: [41, 0],
            flags: 0,
        },
        Case {
            name: "AOS to an AC".into(),
            inst: i(0o350, 2, e),
            ac: acs(&[]),
            mem: [MAX, 0],
            flags: 0,
        },
        Case {
            name: "AOS of an AC".into(),
            inst: i(0o350, 0, 6),
            ac: acs(&[(6, 0o17)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "HRRZ".into(),
            inst: i(0o550, 1, e),
            ac: acs(&[(1, M1)]),
            mem: [0o123456_654321, 0],
            flags: 0,
        },
        Case {
            name: "HLL".into(),
            inst: i(0o500, 1, e),
            ac: acs(&[(1, 0o111111_222222)]),
            mem: [0o333333_444444, 0],
            flags: 0,
        },
        Case {
            name: "CAIE skip".into(),
            inst: i(0o302, 1, 5),
            ac: acs(&[(1, 5)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "CAIE no skip".into(),
            inst: i(0o302, 1, 5),
            ac: acs(&[(1, 6)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "SKIPL".into(),
            inst: i(0o331, 0, e),
            ac: acs(&[]),
            mem: [MIN, 0],
            flags: 0,
        },
        Case {
            name: "MOVE indexed".into(),
            inst: i(0o200, 1, e - 1) | (3 << 18),
            ac: acs(&[(3, 1)]),
            mem: [0o31, 0],
            flags: 0,
        },
        Case {
            name: "MOVE indirect".into(),
            inst: i(0o200, 1, e + 1) | (1 << 22),
            ac: acs(&[]),
            mem: [0o42, e],
            flags: 0,
        },
        Case {
            name: "IMUL".into(),
            inst: i(0o220, 1, e),
            ac: acs(&[(1, M1 - 6)]),
            mem: [9, 0],
            flags: 0,
        },
        Case {
            name: "MUL".into(),
            inst: i(0o224, 1, e),
            ac: acs(&[(1, MAX)]),
            mem: [M1 - 2, 0],
            flags: 0,
        },
        Case {
            name: "IDIV".into(),
            inst: i(0o230, 1, e),
            ac: acs(&[(1, M1 - 99)]),
            mem: [7, 0],
            flags: 0,
        },
        Case {
            name: "DIV".into(),
            inst: i(0o234, 1, e),
            ac: acs(&[(1, 3), (2, 0o12345)]),
            mem: [0o1000, 0],
            flags: 0,
        },
        Case {
            name: "ASH left".into(),
            inst: i(0o240, 1, 3),
            ac: acs(&[(1, M1 - 4)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "ASH right".into(),
            inst: i(0o240, 1, 0o777775),
            ac: acs(&[(1, M1 - 0o100)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "ASH overflow".into(),
            inst: i(0o240, 1, 2),
            ac: acs(&[(1, 0o200000_000001)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "ASH overflow at the last step".into(),
            inst: i(0o240, 1, 2),
            ac: acs(&[(1, 0o100000_000001)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "ASH overflow by one place".into(),
            inst: i(0o240, 1, 1),
            ac: acs(&[(1, 0o200000_000000)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "ASH by 0".into(),
            inst: i(0o240, 1, 0),
            ac: acs(&[(1, 0o200000_000001)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "ASHC left".into(),
            inst: i(0o244, 1, 4),
            ac: acs(&[(1, 0o000001_234567), (2, 0o400000_000123)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "LSH left".into(),
            inst: i(0o242, 1, 3),
            ac: acs(&[(1, 0o700000_000007)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "LSH by 40".into(),
            inst: i(0o242, 1, 0o50),
            ac: acs(&[(1, M1)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "LSH by -40".into(),
            inst: i(0o242, 1, 0o777730),
            ac: acs(&[(1, M1)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "ROT right".into(),
            inst: i(0o241, 1, 0o777775),
            ac: acs(&[(1, 0o700000_000001)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "ASHC".into(),
            inst: i(0o244, 1, 0o777776),
            ac: acs(&[(1, M1), (2, 0o12)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "LSH".into(),
            inst: i(0o242, 1, 0o777774),
            ac: acs(&[(1, M1)]),
            mem: [0, 0],
            flags: 0,
        },
        Case {
            name: "ROT".into(),
            inst: i(0o241, 1, 5),
            ac: acs(&[(1, 0o700000_000001)]),
            mem: [0, 0],
            flags: 0,
        },
    ]
}

/// Left shifts with ASH and ASHC from a fixed seed: the count and the
/// operands are random, and the top bits of the operand copy the sign for a
/// random number of places, so that the last lost bit varies. Each step of
/// the microcode tests the overflow (simple, `ASH36 LEFT`, `ASHCL`).
fn random_shifts() -> Vec<Case> {
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let word = |next: &mut dyn FnMut() -> u64| {
        let w = next() & 0o777777_777777;
        let keep = next() % 36;
        let sign = w >> 35;
        let top = !((1u64 << (35 - keep)) - 1) & 0o777777_777777;
        if sign == 1 { w | top } else { w & !top }
    };
    let mut v = Vec::new();
    for k in 0..48 {
        let ashc = k % 2 == 1;
        let n = 1 + next() % if ashc { 80 } else { 40 };
        let (a, b) = (word(&mut next), next() & 0o777777_777777);
        let op = if ashc { 0o244 } else { 0o240 };
        v.push(Case {
            name: format!(
                "{} {a:012o} {b:012o} by {n}",
                if ashc { "ASHC" } else { "ASH" }
            ),
            inst: i(op, 1, n),
            ac: acs(&[(1, a), (2, b)]),
            mem: [0, 0],
            flags: 0,
        });
    }
    v
}

/// Floating point instructions on random normalized numbers. A third of
/// the exponents are near 0 or 377, so that the results overflow and
/// underflow (`SPEC/EXPTST`: Overflow, Floating Overflow, Floating
/// Underflow, Trap 1).
fn random_floats() -> Vec<Case> {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let float = |next: &mut dyn FnMut() -> u64| {
        let exp = match next() % 3 {
            0 => next() % 12,
            1 => 0o377 - next() % 12,
            _ => next() % 0o400,
        };
        let w = exp << 27 | 1 << 26 | (next() & 0o377777777);
        if next() & 1 == 0 {
            w
        } else {
            w.wrapping_neg() & 0o777777_777777
        }
    };
    // Single: FAD, FADR, FSB, FSBR, FMP, FMPR, FDV, FDVR, FIX, FIXR,
    // FSC. Double: DFAD, DFSB, DFMP, DFDV.
    let single = [
        0o140, 0o144, 0o150, 0o154, 0o160, 0o164, 0o170, 0o174, 0o122, 0o126, 0o132,
    ];
    let double = [0o110, 0o111, 0o112, 0o113];
    let mut v = Vec::new();
    for k in 0..120 {
        let (a, b) = (float(&mut next), float(&mut next));
        let (lo_a, lo_b) = (next() & 0o377777_777777, next() & 0o377777_777777);
        let (op, ac, mem, inst) = if k % 3 == 2 {
            let op = double[(next() % 4) as usize];
            (
                (op),
                acs(&[(1, a), (2, lo_a)]),
                [b, lo_b],
                i(op, 1, E as u64),
            )
        } else {
            let op = single[(next() % 11) as usize];
            // FSC: E is the scale, -200 to 177.
            let e = if op == 0o132 {
                (next() % 0o400).wrapping_sub(0o200) & 0o777777
            } else {
                E as u64
            };
            (op, acs(&[(1, a)]), [b, 0], i(op, 1, e))
        };
        v.push(Case {
            name: format!(
                "{op:o} {:012o} {:012o} {:012o} {:012o}",
                ac[1], ac[2], mem[0], mem[1]
            ),
            inst,
            ac,
            mem,
            flags: 0,
        });
    }
    v
}

/// DMUL on random double words, with high words of 0 or -1 now and then:
/// the four calls of QMULT set Overflow, Carry 0 and Carry 1 from their
/// partial products (simple, QMULT). The first two cases are those of
/// docs/unknowns.md.
fn random_dmuls() -> Vec<Case> {
    let mut x: u64 = 0xd1b5_4a32_d192_ed03;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x & 0o777777_777777
    };
    let mut ops = vec![
        [0o777777_777777, 2, 0o377777_777777, 0o123456_654321],
        [0o400000_000000, 0, 0o400000_000000, 0],
    ];
    for _ in 0..40 {
        let mut w = [next(), next(), next(), next()];
        for h in [0, 2] {
            if next() % 3 == 0 {
                w[h] = if next() & 1 == 0 { 0 } else { 0o777777_777777 };
            }
        }
        ops.push(w);
    }
    ops.into_iter()
        .map(|w| Case {
            name: format!(
                "DMUL {:012o} {:012o} by {:012o} {:012o}",
                w[0], w[1], w[2], w[3]
            ),
            inst: i(0o116, 1, E as u64),
            ac: acs(&[(1, w[0]), (2, w[1])]),
            mem: [w[2], w[3]],
            flags: 0,
        })
        .collect()
}

/// A case of [`flag_cases`]: name, instruction, ACs, and the words at 2000
/// and 2001.
type Base = (&'static str, u64, Vec<(usize, u64)>, [u64; 2]);

/// Instructions that start with flags set. Each word with `SPEC/FLAGS`
/// loads Trap 1 and Trap 2 (DPE9 E60, E78): `AD FLAGS`, `SET AROV`, `SET
/// PDL OV`, `NO DIVIDE` and `JFCL FLAGS` (ks10, "MACROS -- PC FLAGS";
/// simple). The cases start with the two trap flags, and with all flags
/// of executive mode except FPD.
fn flag_cases() -> Vec<Case> {
    use ks10_core_soft::flags::Flags as F;
    const MAX: u64 = 0o377777_777777;
    const M1: u64 = 0o777777_777777;
    const MIN: u64 = 0o400000_000000;
    let e = E as u64;
    let traps = F::TRAP1 | F::TRAP2;
    let all = F::ALL & !(F::USER | F::USERIO | F::FPD | F::OIP);
    let base: Vec<Base> = vec![
        ("MOVE", i(0o200, 1, e), vec![], [5, 0]),
        ("MOVN", i(0o210, 1, e), vec![], [5, 0]),
        (
            "MOVM of a negative number",
            i(0o214, 1, e),
            vec![],
            [M1 - 4, 0],
        ),
        ("MOVM of a positive number", i(0o214, 1, e), vec![], [5, 0]),
        ("ADD", i(0o270, 1, e), vec![(1, 1)], [1, 0]),
        ("ADD with overflow", i(0o270, 1, e), vec![(1, MAX)], [1, 0]),
        ("SUB", i(0o274, 1, e), vec![(1, 7)], [2, 0]),
        ("AOS", i(0o350, 1, e), vec![], [6, 0]),
        ("AOJ", i(0o340, 1, e), vec![(1, 6)], [0, 0]),
        ("SOJ", i(0o360, 1, e), vec![(1, 6)], [0, 0]),
        ("SOS", i(0o370, 1, e), vec![], [6, 0]),
        ("IMUL", i(0o220, 1, e), vec![(1, 3)], [5, 0]),
        ("IMUL with overflow", i(0o220, 1, e), vec![(1, MAX)], [2, 0]),
        ("MUL", i(0o224, 1, e), vec![(1, 3)], [5, 0]),
        ("IDIV", i(0o230, 1, e), vec![(1, 7)], [2, 0]),
        ("IDIV by 0", i(0o230, 1, e), vec![(1, 7)], [0, 0]),
        (
            "DIV, quotient too big",
            i(0o234, 1, e),
            vec![(1, 1), (2, 0)],
            [1, 0],
        ),
        ("DADD", i(0o114, 1, e), vec![(1, 0), (2, 1)], [0, 1]),
        ("DMOVN, low word not 0", i(0o121, 1, e), vec![], [0, 5]),
        ("DMOVN of 5,,5", i(0o121, 1, e), vec![], [5, 5]),
        ("DMOVN, high word -1", i(0o121, 1, e), vec![], [M1, 5]),
        (
            "DMOVN, high word 400000,,0",
            i(0o121, 1, e),
            vec![],
            [MIN, 5],
        ),
        (
            "DMOVNM, low word not 0",
            i(0o125, 1, e),
            vec![(1, 7), (2, 5)],
            [0, 0],
        ),
        ("DMOVN, low word 0", i(0o121, 1, e), vec![], [5, 0]),
        ("JFCL 0,", i(0o255, 0, e), vec![], [0, 0]),
        ("JFCL 17,", i(0o255, 0o17, e), vec![], [0, 0]),
        ("PUSH", i(0o261, 1, e), vec![(1, 0o000001_002000)], [0, 0]),
        (
            "PUSH with overflow",
            i(0o261, 1, e),
            vec![(1, 0o777777_002000)],
            [0, 0],
        ),
        (
            "POP with overflow",
            i(0o262, 1, e + 1),
            vec![(1, 0o000000_002000)],
            [3, 0],
        ),
        ("ADJSP with overflow", i(0o105, 1, 1), vec![(1, M1)], [0, 0]),
        ("ASH by 1", i(0o240, 1, 1), vec![(1, 1)], [0, 0]),
        ("ASH by 0", i(0o240, 1, 0), vec![(1, 1)], [0, 0]),
        ("ROT", i(0o241, 1, 5), vec![(1, 1)], [0, 0]),
        ("SETZ", i(0o400, 1, 0), vec![(1, 1)], [0, 0]),
        ("AOBJN", i(0o253, 1, e), vec![(1, M1)], [0, 0]),
        ("DMOVE", i(0o120, 1, e), vec![], [1, 2]),
        ("PUSHJ", i(0o260, 1, e), vec![(1, 0o000001_002000)], [0, 0]),
        // Byte pointers at 2000: P,S,,2001 (bits 0-5, 6-11, 18-35).
        (
            "LDB",
            i(0o135, 1, e),
            vec![],
            [0o220600_002001, 0o123456_765432],
        ),
        (
            "DPB",
            i(0o137, 1, e),
            vec![(1, 0o77)],
            [0o220600_002001, 0o123456_765432],
        ),
        (
            "ILDB",
            i(0o134, 1, e),
            vec![],
            [0o220600_002001, 0o123456_765432],
        ),
        (
            "IDPB",
            i(0o136, 1, e),
            vec![(1, 0o77)],
            [0o220600_002001, 0o123456_765432],
        ),
        (
            "DPB of a 7-bit byte 5, bit 35 set",
            i(0o137, 1, e),
            vec![(1, 0o177)],
            [0o010700_002001, 0o000000_000001],
        ),
        (
            "ILDB of a 7-bit byte",
            i(0o134, 1, e),
            vec![],
            [0o440700_002001, 0o101_102_103_104],
        ),
        (
            "JFFO",
            i(0o243, 1, 0o1777),
            vec![(1, 0o000040_000000)],
            [0, 0],
        ),
        ("JSR", i(0o264, 0, e), vec![], [0, 0]),
        ("JSP", i(0o265, 1, e), vec![], [0, 0]),
        ("DMUL", i(0o116, 1, e), vec![(1, 0), (2, 3)], [0, 5]),
        // EXP TEST loads Trap 1, Overflow, Floating Overflow and Floating
        // Underflow (DPE9 E209): without an overflow it clears Trap 1.
        (
            "FAD",
            i(0o140, 1, e),
            vec![(1, 0o201400_000000)],
            [0o202400_000000, 0],
        ),
        (
            "FMP with an underflow",
            i(0o160, 1, e),
            vec![(1, 0o001400_000000)],
            [0o001400_000000, 0],
        ),
        (
            "FMP with an overflow",
            i(0o160, 1, e),
            vec![(1, 0o377400_000000)],
            [0o377400_000000, 0],
        ),
        (
            "DFAD",
            i(0o110, 1, e),
            vec![(1, 0o201400_000000), (2, 0)],
            [0o202400_000000, 0],
        ),
        (
            "DMUL of large numbers",
            i(0o116, 1, e),
            vec![(1, MAX), (2, MAX)],
            [MAX, MAX],
        ),
    ];
    let mut v = Vec::new();
    for (f, fname) in [(traps, "the traps"), (all, "all flags")] {
        for (name, inst, ac, mem) in &base {
            v.push(Case {
                name: format!("{name}, with {fname}"),
                inst: *inst,
                ac: acs(ac),
                mem: *mem,
                flags: f,
            });
        }
    }
    v
}

/// A known difference between the model and `ks10-core-soft`. For a case
/// where `applies` is true, `normalize` takes the different parts out of
/// both states before the comparison; the other parts must agree.
struct Known {
    /// The section of docs/emulator.md, "Differences from the microcode".
    why: &'static str,
    applies: fn(&Case) -> bool,
    normalize: fn(&mut State),
}

const TRAPS: u32 = Flags::TRAP1 | Flags::TRAP2;
const CARRIES: u32 = Flags::CRY0 | Flags::CRY1;

/// The opcode of a case.
fn opcode(c: &Case) -> u64 {
    c.inst >> 27
}

/// The known differences (docs/discrepancies.md has the sources of each).
const KNOWN: &[Known] = &[
    // The microcode loads Trap 1 and Trap 2 at each flag word, and ASH
    // gives Trap 1 the overflow of the last step; ks10-core-soft follows
    // the PRM: an instruction only sets a trap flag, on any overflow.
    Known {
        why: "Trap 1 and Trap 2",
        applies: |c| c.flags & TRAPS != 0 || matches!(opcode(c), 0o116 | 0o240 | 0o244),
        normalize: |s| s.flags &= !TRAPS,
    },
    // The complement of the high word sets Carry 0 and Carry 1 in the
    // microcode; ks10-core-soft follows the PRM.
    Known {
        why: "DMOVN carry flags",
        applies: |c| matches!(opcode(c), 0o121 | 0o125),
        normalize: |s| s.flags &= !CARRIES,
    },
    // Each QMULT of the microcode sets Carry 0 and Carry 1 from a partial
    // product; ks10-core-soft follows the PRM.
    Known {
        why: "DMUL carry flags",
        applies: |c| opcode(c) == 0o116,
        normalize: |s| s.flags &= !CARRIES,
    },
];

/// Do the two states agree, after the normalization of the known
/// differences that apply to the case? The second value names them.
fn agree(known: &[Known], c: &Case, got: &State, want: &State) -> (bool, Vec<&'static str>) {
    let (mut got, mut want) = (got.clone(), want.clone());
    let mut used = Vec::new();
    for k in known.iter().filter(|k| (k.applies)(c)) {
        (k.normalize)(&mut got);
        (k.normalize)(&mut want);
        used.push(k.why);
    }
    (got == want, used)
}

fn all_cases() -> Vec<Case> {
    let mut v = cases();
    v.extend(random_shifts());
    v.extend(random_floats());
    v.extend(random_dmuls());
    v.extend(flag_cases());
    v
}

#[test]
fn single_instructions_agree_with_the_software_processor() {
    let mut stops = Vec::new();
    let mut differences = Vec::new();
    let mut known = std::collections::BTreeMap::new();
    let mut known_cases = 0;
    for fill in FILLS {
        for c in all_cases() {
            let want = reference(&c).expect(&c.name);
            let got = model(fill, &c);
            match got {
                Ok(got) if got == want => {}
                Ok(got) if agree(KNOWN, &c, &got, &want).0 => {
                    known_cases += 1;
                    for why in agree(KNOWN, &c, &got, &want).1 {
                        *known.entry(why).or_insert(0) += 1;
                    }
                }
                Ok(got) => differences.push(format!(
                    "{} (fill {:o}):\n  model {got}\n  ks10  {want}",
                    c.name,
                    fill.get()
                )),
                Err(e) => stops.push(format!("{} (fill {:o}): {e}", c.name, fill.get())),
            }
        }
    }
    let n = 2 * all_cases().len();
    let k = known_cases;
    println!(
        "{} cases: {} agree, {k} known differences, {} differ, {} stop",
        n,
        n - k - differences.len() - stops.len(),
        differences.len(),
        stops.len()
    );
    for (why, count) in &known {
        println!("KNOWN, applied to {count}: {why}");
    }
    for s in &stops {
        println!("STOP {s}");
    }
    for d in &differences {
        println!("DIFF {d}");
    }
    assert!(differences.is_empty() && stops.is_empty());
}

/// A known difference takes out only its own part of the states.
#[test]
fn a_known_difference_hides_only_its_part() {
    let known = [Known {
        why: "test",
        applies: |c| c.name == "x",
        normalize: |s| s.flags = 0,
    }];
    let c = |name: &str| Case {
        name: name.into(),
        inst: 0,
        ac: [0; 16],
        mem: [0; 2],
        flags: 0,
    };
    let want = State {
        pc: 0o1001,
        flags: 0,
        ac: [0; 16],
        mem: [0; 2],
    };
    let flags = State {
        flags: 0o400000,
        ..want.clone()
    };
    let pc = State {
        pc: 0o1002,
        ..flags.clone()
    };
    assert_eq!(agree(&known, &c("x"), &flags, &want), (true, vec!["test"]));
    assert_eq!(agree(&known, &c("y"), &flags, &want), (false, vec![]));
    assert!(!agree(&known, &c("x"), &pc, &want).0);
}

/// The flag rules of the microcode that `ks10-core-soft` does not have
/// ([`KNOWN`]), on the model alone. The expected flags come from the
/// reading of DPE9 and the microcode in docs/discrepancies.md ("Trap 1 and
/// Trap 2: each flag word loads them", "DMOVN: the carry flags of the
/// complement", "DMUL: the carry flags of QMULT").
#[test]
fn the_flag_rules_of_the_microcode() {
    const N: u64 = 0o777777_777777;
    let e = E as u64;
    let case = |op: u64, ac: Vec<(usize, u64)>, mem: [u64; 2], flags: u32| {
        let mut c = Case {
            name: format!("{op:o}"),
            inst: op << 27 | 1 << 23 | e,
            ac: [0; 16],
            mem,
            flags,
        };
        for (k, v) in ac {
            c.ac[k] = v;
        }
        c
    };
    let cases = [
        // ADD without overflow: AD FLAGS clears both trap flags.
        (case(0o270, vec![(1, 1)], [1, 0], TRAPS), 0),
        // POP with overflow: SET PDL OV sets Trap 2 and clears Trap 1.
        (
            case(0o262, vec![(1, 0o000000_002000)], [3, 0], TRAPS),
            Flags::TRAP2,
        ),
        // ASH 1,2 of 200000,,000001: the first step overflows, the last
        // does not. Overflow stays, Trap 1 is the last step.
        (
            Case {
                inst: 0o240 << 27 | 1 << 23 | 2,
                ..case(0o240, vec![(1, 0o200000_000001)], [0, 0], 0)
            },
            Flags::AROV,
        ),
        // DMOVN with a low word not 0: the complement sets the carries.
        (case(0o121, vec![], [0, 5], 0), CARRIES),
        // DMUL: a negative partial product sets the carries.
        (
            case(
                0o116,
                vec![(1, N), (2, 2)],
                [0o377777_777777, 0o123456_654321],
                0,
            ),
            CARRIES,
        ),
        // DMUL of (-2^70)^2: Overflow, Carry 1 and Trap 1.
        (
            case(0o116, vec![(1, 1 << 35)], [1 << 35, 0], 0),
            Flags::AROV | Flags::CRY1 | Flags::TRAP1,
        ),
    ];
    for (c, want) in cases {
        let got = model(FILLS[0], &c).expect(&c.name);
        assert_eq!(got.flags, want, "{}", c.name);
    }
}

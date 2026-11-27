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
//! the ACs, PC, the program flags and the words at 2000 and 2001. A
//! difference is a finding for docs/discrepancies.md, not a value to
//! adjust (AGENTS.md, "Tests").

mod common;

use common::{FILLS, label, machine, run_until, symbol};
use ks10_core_uop::alu;
use ks10_core_uop::machine::{Coverage, Machine, PcFlags};
use pdp10::{Half, W36};

const CODE: usize = 0o1000;
const E: usize = 0o2000;

/// A case: the instruction word (its E is 2000, 2001 or an AC), the ACs
/// and the words at 2000 and 2001.
struct Case {
    name: String,
    inst: u64,
    ac: [u64; 16],
    mem: [u64; 2],
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

fn ks10_flags(f: PcFlags) -> u32 {
    use ks10_core_soft::flags::Flags as F;
    [
        (f.ov, F::AROV),
        (f.cry0, F::CRY0),
        (f.cry1, F::CRY1),
        (f.fov, F::FOV),
        (f.fpd, F::FPD),
        (f.user, F::USER),
        (f.pcu, F::USERIO),
        (f.trap2, F::TRAP2),
        (f.trap1, F::TRAP1),
        (f.fxu, F::FXU),
        (f.nodiv, F::NODIV),
    ]
    .iter()
    .filter(|(v, _)| *v)
    .fold(0, |a, (_, b)| a | b)
}

fn reference(c: &Case) -> Result<State, String> {
    let mut m = ks10_core_soft::Machine::new();
    for (k, &a) in c.ac.iter().enumerate() {
        m.ac[k] = W36::wrap(a);
    }
    m.mem[CODE] = W36::wrap(c.inst);
    m.mem[E] = W36::wrap(c.mem[0]);
    m.mem[E + 1] = W36::wrap(c.mem[1]);
    m.flags = ks10_core_soft::flags::Flags(0);
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
fn halted(text: &str, fill: alu::W40) -> Machine {
    let haltlp = label(text, "HALTLP");
    let mut m = machine(text, fill);
    run_until(&mut m, 100_000, |m| m.upc == haltlp && m.csl.halt_loop).unwrap();
    m
}

fn model(text: &str, fill: alu::W40, c: &Case, coverage: &mut Coverage) -> Result<State, String> {
    let haltlp = label(text, "HALTLP");
    // A machine has its own bus with the devices, so each case runs the
    // power-up sequence (about 640 microcycles).
    let mut m = halted(text, fill);
    for (k, &a) in c.ac.iter().enumerate() {
        m.ram_file[k] = W36::wrap(a);
    }
    m.memory[CODE] = W36::wrap(c.inst);
    m.memory[E] = W36::wrap(c.mem[0]);
    m.memory[E + 1] = W36::wrap(c.mem[1]);
    // Design decision 2026-11-27: the test sets PC in the 2901 register.
    // The console starts a program with EXECUTE of a JRST (cmds.m80,
    // STCMD, EXINT), which reads the console register with an I/O cycle.
    m.alu.reg[symbol(text, "PC")] = alu::d_input(W36::wrap(CODE as u64));
    // One tick of the 1 ms timer with 1 MSEC EN clear clears its flag.
    m.millisecond();
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
    coverage.merge(&m.coverage);
    Ok(State {
        pc: m.memory[1].get() & 0o777777,
        flags: ks10_flags(m.flags),
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
        },
        Case {
            name: "MOVEI".into(),
            inst: i(0o201, 2, 0o1234),
            ac: acs(&[]),
            mem: [0, 0],
        },
        Case {
            name: "MOVEM".into(),
            inst: i(0o202, 3, e),
            ac: acs(&[(3, 0o707070_070707)]),
            mem: [5, 0],
        },
        Case {
            name: "MOVE from an AC".into(),
            inst: i(0o200, 1, 4),
            ac: acs(&[(4, 0o4444)]),
            mem: [0, 0],
        },
        Case {
            name: "MOVEM to an AC".into(),
            inst: i(0o202, 5, 6),
            ac: acs(&[(5, 0o5555)]),
            mem: [0, 0],
        },
        Case {
            name: "ADD".into(),
            inst: i(0o270, 1, e),
            ac: acs(&[(1, 5)]),
            mem: [7, 0],
        },
        Case {
            name: "ADD overflow".into(),
            inst: i(0o270, 1, e),
            ac: acs(&[(1, MAX)]),
            mem: [1, 0],
        },
        Case {
            name: "ADD carries".into(),
            inst: i(0o270, 1, e),
            ac: acs(&[(1, M1)]),
            mem: [M1, 0],
        },
        Case {
            name: "SUB".into(),
            inst: i(0o274, 1, e),
            ac: acs(&[(1, 5)]),
            mem: [7, 0],
        },
        Case {
            name: "SUB overflow".into(),
            inst: i(0o274, 1, e),
            ac: acs(&[(1, MIN)]),
            mem: [1, 0],
        },
        Case {
            name: "ADDB".into(),
            inst: i(0o273, 7, e),
            ac: acs(&[(7, 0o100)]),
            mem: [0o23, 0],
        },
        Case {
            name: "AND".into(),
            inst: i(0o404, 2, e),
            ac: acs(&[(2, 0o707070_707070)]),
            mem: [0o777000_000777, 0],
        },
        Case {
            name: "IOR".into(),
            inst: i(0o434, 2, e),
            ac: acs(&[(2, 0o700000_000007)]),
            mem: [0o000077_770000, 0],
        },
        Case {
            name: "SETZM".into(),
            inst: i(0o402, 0, e),
            ac: acs(&[]),
            mem: [M1, 0],
        },
        Case {
            name: "AOS".into(),
            inst: i(0o350, 0, e),
            ac: acs(&[]),
            mem: [41, 0],
        },
        Case {
            name: "AOS to an AC".into(),
            inst: i(0o350, 2, e),
            ac: acs(&[]),
            mem: [MAX, 0],
        },
        Case {
            name: "AOS of an AC".into(),
            inst: i(0o350, 0, 6),
            ac: acs(&[(6, 0o17)]),
            mem: [0, 0],
        },
        Case {
            name: "HRRZ".into(),
            inst: i(0o550, 1, e),
            ac: acs(&[(1, M1)]),
            mem: [0o123456_654321, 0],
        },
        Case {
            name: "HLL".into(),
            inst: i(0o500, 1, e),
            ac: acs(&[(1, 0o111111_222222)]),
            mem: [0o333333_444444, 0],
        },
        Case {
            name: "CAIE skip".into(),
            inst: i(0o302, 1, 5),
            ac: acs(&[(1, 5)]),
            mem: [0, 0],
        },
        Case {
            name: "CAIE no skip".into(),
            inst: i(0o302, 1, 5),
            ac: acs(&[(1, 6)]),
            mem: [0, 0],
        },
        Case {
            name: "SKIPL".into(),
            inst: i(0o331, 0, e),
            ac: acs(&[]),
            mem: [MIN, 0],
        },
        Case {
            name: "MOVE indexed".into(),
            inst: i(0o200, 1, e - 1) | (3 << 18),
            ac: acs(&[(3, 1)]),
            mem: [0o31, 0],
        },
        Case {
            name: "MOVE indirect".into(),
            inst: i(0o200, 1, e + 1) | (1 << 22),
            ac: acs(&[]),
            mem: [0o42, e],
        },
        Case {
            name: "IMUL".into(),
            inst: i(0o220, 1, e),
            ac: acs(&[(1, M1 - 6)]),
            mem: [9, 0],
        },
        Case {
            name: "MUL".into(),
            inst: i(0o224, 1, e),
            ac: acs(&[(1, MAX)]),
            mem: [M1 - 2, 0],
        },
        Case {
            name: "IDIV".into(),
            inst: i(0o230, 1, e),
            ac: acs(&[(1, M1 - 99)]),
            mem: [7, 0],
        },
        Case {
            name: "DIV".into(),
            inst: i(0o234, 1, e),
            ac: acs(&[(1, 3), (2, 0o12345)]),
            mem: [0o1000, 0],
        },
        Case {
            name: "ASH left".into(),
            inst: i(0o240, 1, 3),
            ac: acs(&[(1, M1 - 4)]),
            mem: [0, 0],
        },
        Case {
            name: "ASH right".into(),
            inst: i(0o240, 1, 0o777775),
            ac: acs(&[(1, M1 - 0o100)]),
            mem: [0, 0],
        },
        Case {
            name: "ASH overflow".into(),
            inst: i(0o240, 1, 2),
            ac: acs(&[(1, 0o200000_000001)]),
            mem: [0, 0],
        },
        Case {
            name: "ASH overflow at the last step".into(),
            inst: i(0o240, 1, 2),
            ac: acs(&[(1, 0o100000_000001)]),
            mem: [0, 0],
        },
        Case {
            name: "ASH overflow by one place".into(),
            inst: i(0o240, 1, 1),
            ac: acs(&[(1, 0o200000_000000)]),
            mem: [0, 0],
        },
        Case {
            name: "ASH by 0".into(),
            inst: i(0o240, 1, 0),
            ac: acs(&[(1, 0o200000_000001)]),
            mem: [0, 0],
        },
        Case {
            name: "ASHC left".into(),
            inst: i(0o244, 1, 4),
            ac: acs(&[(1, 0o000001_234567), (2, 0o400000_000123)]),
            mem: [0, 0],
        },
        Case {
            name: "LSH left".into(),
            inst: i(0o242, 1, 3),
            ac: acs(&[(1, 0o700000_000007)]),
            mem: [0, 0],
        },
        Case {
            name: "LSH by 40".into(),
            inst: i(0o242, 1, 0o50),
            ac: acs(&[(1, M1)]),
            mem: [0, 0],
        },
        Case {
            name: "LSH by -40".into(),
            inst: i(0o242, 1, 0o777730),
            ac: acs(&[(1, M1)]),
            mem: [0, 0],
        },
        Case {
            name: "ROT right".into(),
            inst: i(0o241, 1, 0o777775),
            ac: acs(&[(1, 0o700000_000001)]),
            mem: [0, 0],
        },
        Case {
            name: "ASHC".into(),
            inst: i(0o244, 1, 0o777776),
            ac: acs(&[(1, M1), (2, 0o12)]),
            mem: [0, 0],
        },
        Case {
            name: "LSH".into(),
            inst: i(0o242, 1, 0o777774),
            ac: acs(&[(1, M1)]),
            mem: [0, 0],
        },
        Case {
            name: "ROT".into(),
            inst: i(0o241, 1, 5),
            ac: acs(&[(1, 0o700000_000001)]),
            mem: [0, 0],
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
        });
    }
    v
}

#[test]
fn single_instructions_agree_with_the_software_processor() {
    let Some(text) = common::listing_text("instructions") else {
        return;
    };
    let mut stops = Vec::new();
    let mut differences = Vec::new();
    let mut coverage = Coverage::default();
    for fill in FILLS {
        for c in cases().into_iter().chain(random_shifts()) {
            let want = reference(&c).expect(&c.name);
            let got = model(&text, fill, &c, &mut coverage);
            match got {
                Ok(got) if got == want => {}
                Ok(got) => differences.push(format!(
                    "{} (fill {:o}):\n  model {got}\n  ks10  {want}",
                    c.name,
                    fill.get()
                )),
                Err(e) => stops.push(format!("{} (fill {:o}): {e}", c.name, fill.get())),
            }
        }
    }
    let n = 2 * (cases().len() + random_shifts().len());
    println!(
        "{} cases: {} agree, {} differ, {} stop",
        n,
        n - differences.len() - stops.len(),
        differences.len(),
        stops.len()
    );
    println!("{coverage}");
    for s in &stops {
        println!("STOP {s}");
    }
    for d in &differences {
        println!("DIFF {d}");
    }
    assert!(differences.is_empty() && stops.is_empty());
}

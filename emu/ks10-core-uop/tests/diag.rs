//! The DEC KS10 diagnostics on the microcode-level model: recorded data
//! of the machine (AGENTS.md, "Sources of truth", rank 2). The programs,
//! the run file and the pass criteria are those of
//! `emu/ks10-core-soft/tests/diag.rs` (SMCPU.CMD; docs/emulator.md,
//! "Diagnostics"):
//!
//! - DSKAA to DSKAH run alone and pass if they do not halt and their pass
//!   count (location 30047, PASCNT) reaches the iterations.
//! - The others run under SUBSM and pass if they do not halt, print "END
//!   PASS n." and print no error message.
//!
//! The model starts a program with the console command ST 30000. A halt
//! is the halt loop of the microcode. A program that reaches a function
//! that the model does not have is PENDING: the test prints it and does
//! not count it as a pass or a failure. A halt, an error message or a
//! wrong pass count fails the test. The test needs `KS10_DIAG_DIR` (the
//! .SAV files, `make diag`).

mod common;

use std::path::{Path, PathBuf};

use common::{FILLS, label, machine, symbol};
use ks10_core_uop::alu;
use ks10_core_uop::machine::Machine;
use ks10_machine::boot::Processor;
use ks10_machine::console::Script;
use ks10_machine::sav::{self, Packing};

/// The most microcycles without an instruction: no instruction of the
/// diagnostics takes so long, and the halt loop counts none.
const STALL_CYCLES: u64 = 10_000_000;

fn dir() -> Option<PathBuf> {
    std::env::var_os("KS10_DIAG_DIR").map(PathBuf::from)
}

fn load(m: &mut Machine, d: &Path, name: &str) {
    let bytes =
        std::fs::read(d.join(format!("diagnostics..{name}.sav"))).expect("read the .SAV file");
    let s = sav::parse(&sav::words(&bytes, Packing::Ansi5)).expect("a SAV file");
    for (addr, ws) in &s.blocks {
        for (k, w) in ws.iter().enumerate() {
            m.memory[addr.get() as usize + k] = *w;
        }
    }
}

/// Why a run ended.
#[derive(Debug, PartialEq)]
enum End {
    /// The condition of the caller.
    Done,
    Limit,
    Halt {
        code: u64,
        pc: u64,
    },
    Stop(String),
}

/// Step the model until `done` is true, it halts, it stops, or `limit`
/// instructions. The console script runs between instructions.
fn run(
    m: &mut Machine,
    haltlp: u16,
    limit: u64,
    script: &mut Script,
    mut done: impl FnMut(&Machine) -> bool,
) -> End {
    let mut last = (u64::MAX, m.cycles);
    loop {
        if m.instructions != last.0 {
            last = (m.instructions, m.cycles);
            if done(m) {
                return End::Done;
            }
            if m.instructions >= limit {
                return End::Limit;
            }
            script.tick(m.instructions, &mut m.cty);
        } else if m.upc == haltlp && m.csl.halt_loop {
            return End::Halt {
                code: m.memory[0].get(),
                pc: m.memory[1].get() & 0o777777,
            };
        } else if m.cycles - last.1 > STALL_CYCLES {
            return End::Stop(format!("no instruction at {:04o}", m.upc));
        }
        if let Err(e) = m.step() {
            return End::Stop(e.to_string());
        }
    }
}

/// A model after the power-up sequence and the master reset, with the
/// programs loaded and started at 30000.
fn started(d: &Path, programs: &[&str]) -> Machine {
    let mut m = machine(FILLS[0]);
    m.master_reset();
    for p in programs {
        load(&mut m, d, p);
    }
    m.start_at(0o30000);
    m
}

/// How a program of SMCPU.CMD runs (`emu/ks10-core-soft/tests/diag.rs`,
/// `how`, for the reasons of the switches and of the programs not run).
enum How {
    Alone,
    Subsm(&'static str),
    /// DSKCG to the start of test 9, with the UBR fixture of SUBSM.
    Dskcg,
    Skip(&'static str),
}

fn how(name: &str) -> Option<How> {
    Some(match name {
        "DSKAA" | "DSKAB" | "DSKAC" | "DSKAD" | "DSKAE" | "DSKAF" | "DSKAG" | "DSKAH" => How::Alone,
        "DSKAI" | "DSKAJ" | "DSKAK" | "DSKAL" | "DSKAM" | "DSKCA" | "DSKCB" | "DSKCC" | "DSKCD"
        | "DSKCE" | "DSKCF" => How::Subsm("000100"),
        "DSKBA" => How::Subsm("000110"),
        "DSKCG" => How::Dskcg,
        "DSKEA" => How::Skip("the paging of the DEC microcode"),
        // DSKEB tells cache hits from misses by time: RDTIME before and
        // after a page of JFCLs (DSKEBM.MAC, PHYCHK, CACCHK). The time
        // base of the model counts instructions.
        "DSKEB" => {
            How::Skip("cache hits by time, and the time base of the model counts instructions")
        }
        "DSKEC" => How::Skip("the KL paging of the DEC microcode"),
        _ => return None,
    })
}

fn run_file(d: &Path) -> Vec<(String, String, u64)> {
    let text = std::fs::read(d.join("diagnostics..smcpu.cmd")).expect("read SMCPU.CMD");
    let text = String::from_utf8_lossy(&text);
    let mut list = Vec::new();
    for line in text.lines() {
        let line = line.trim_matches(|c: char| c == '\r' || c == '\0' || c == '\x1a');
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(f.len(), 4, "SMCPU.CMD line {line:?}");
        let iterations = u64::from_str_radix(f[3], 8).expect("octal iterations");
        list.push((f[0].to_string(), f[2].to_string(), iterations.max(1)));
    }
    list
}

fn one(d: &Path, name: &str, how: &How, rh: &str, iterations: u64) -> Result<String, String> {
    let haltlp = label("HALTLP");
    let lower = name.to_lowercase();
    match how {
        How::Skip(why) => Ok(format!("{name}: not run, it tests {why}")),
        How::Alone => {
            let mut m = started(d, &[&lower]);
            let pascnt = |m: &Machine| m.memory[0o30047];
            let end = run(&mut m, haltlp, 200_000_000, &mut Script::default(), |m| {
                pascnt(m).get() >= iterations && !pascnt(m).bit(0)
            });
            match end {
                End::Done => Ok(format!(
                    "{name}: PASCNT {:o} after {} instructions",
                    pascnt(&m).get(),
                    m.instructions
                )),
                e => Err(format!("{name}: {e:?}, PASCNT {:o}", pascnt(&m).get())),
            }
        }
        How::Dskcg => {
            // The fixture and the end of `emu/ks10-core-soft/tests/diag.rs`
            // (`How::Dskcg`): the ITS form of the UBR of SUBSM at 7103, and
            // the run ends at the first PAGSET (35223), after tests 1 to 8.
            let mut m = started(d, &["subsm", &lower]);
            m.memory[0o7103] = pdp10::W36::wrap(0o100000_000000);
            let pc = symbol("A", "PC");
            let mut script = subsm_script("000100", rh);
            let end = run(&mut m, haltlp, 50_000_000, &mut script, |m| {
                alu::word(m.alu.reg[pc]).get() & 0o777777 == 0o35223
            });
            let out = String::from_utf8_lossy(&m.cty.output).replace('\r', "");
            if end != End::Done {
                return Err(format!("{name}: {end:?}, not at test 9\n{out}"));
            }
            if out.contains("ERROR") {
                return Err(format!("{name}: an error message\n{out}"));
            }
            Ok(format!(
                "{name}: tests 1 to 8 without an error, {} instructions",
                m.instructions
            ))
        }
        How::Subsm(lh) => {
            let want = format!("END PASS {iterations}.");
            let mut m = started(d, &["subsm", &lower]);
            let mut script = subsm_script(lh, rh);
            // The text comes one character at a time: look at the end.
            let end = run(&mut m, haltlp, 1_000_000_000, &mut script, |m| {
                let o = &m.cty.output;
                o[o.len().saturating_sub(2 * want.len())..]
                    .windows(want.len())
                    .any(|w| w == want.as_bytes())
            });
            let out = String::from_utf8_lossy(&m.cty.output).replace('\r', "");
            if end != End::Done {
                return Err(format!("{name}: {end:?}\n{out}"));
            }
            if out.contains("ERROR") {
                return Err(format!("{name}: an error message\n{out}"));
            }
            Ok(format!(
                "{name}: \"{want}\" after {} instructions",
                m.instructions
            ))
        }
    }
}

/// The answers to the questions of SUBSM.
fn subsm_script(lh: &str, rh: &str) -> Script {
    let mut script = Script::default();
    script.answer("TTY SWITCH CONTROL", "Y\r");
    script.answer("LH SWITCHES", &format!("{lh}\r"));
    script.answer("RH SWITCHES", &format!("{rh}\r"));
    script
}

/// A stop of the model at a function that it does not have.
fn pending(name: &str, r: &Result<String, String>) -> Option<String> {
    let e = r.as_ref().err()?;
    e.contains("not modelled")
        .then(|| format!("{name}: PENDING, {}", e.lines().next().unwrap_or("")))
}

#[test]
fn smcpu_run_file() {
    let Some(d) = dir() else {
        println!("SKIPPED: diagnostics: KS10_DIAG_DIR is not set");
        return;
    };
    let jobs: Vec<(String, How, String, u64)> = run_file(&d)
        .into_iter()
        .map(|(n, rh, i)| {
            let h =
                how(&n).unwrap_or_else(|| panic!("{n}: a program of SMCPU.CMD without a decision"));
            (n, h, rh, i)
        })
        .collect();
    let results: Vec<Result<String, String>> = std::thread::scope(|s| {
        let hs: Vec<_> = jobs
            .iter()
            .map(|(n, h, rh, i)| {
                let d = &d;
                s.spawn(move || one(d, n, h, rh, *i))
            })
            .collect();
        hs.into_iter()
            .map(|h| h.join().expect("a thread"))
            .collect()
    });
    let mut failed = Vec::new();
    for ((n, ..), r) in jobs.iter().zip(results) {
        if let Some(p) = pending(n, &r) {
            println!("{p}");
            continue;
        }
        match r {
            Ok(t) => println!("{t}"),
            Err(e) => {
                println!("FAILED {e}");
                failed.push(e);
            }
        }
    }
    assert!(failed.is_empty(), "{} programs failed", failed.len());
}

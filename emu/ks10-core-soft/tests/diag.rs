//! The DEC KS10 diagnostics that run alone (docs/emulator.md,
//! "Diagnostics"). The programs are third-party files and are not in git:
//! the test runs only if KS10_DIAG_DIR names a directory with the .SAV
//! files (5-byte words, as t10backup extracts them, with the names
//! "diagnostics..dskaa.sav" and so on).
//!
//! The test runs the programs of the run file of the diagnostic monitor,
//! SMCPU.CMD, in its order, each for the iterations of its line (SMMON.TXT,
//! section 5.0: name, passes, right half of the switches, iterations in
//! octal). `make diag` extracts the files from the tape and runs it.
//!
//! - DSKAA to DSKAH run alone. Such a program passes if it does not halt
//!   and its pass count (location 30047, PASCNT; DSKAA.TXT, section 5.0)
//!   reaches the iterations.
//! - The others run under SUBSM (DSKAM.TXT, 3.2). Such a program passes if
//!   it does not halt, prints "END PASS n." for n = the iterations
//!   (decimal), and prints no error message.
//! - A program of the run file that is not in the table of this file
//!   fails the test, so that a new program gets a decision.

use ks10_core_soft::{Machine, Stop};
use ks10_machine::console::Script;
use ks10_machine::diag::{How, dir, how, load, run_file};
use pdp10::Half;

const DSKEB: &str = "the cache, which this emulator does not have";

/// Load SUBSM and a program, type the switch answers, and step until the
/// CTY output has `done` (if not empty), `until` is the PC, or `limit`
/// steps are done.
fn subsm_run(
    name: &str,
    (lh, rh): (&str, &str),
    limit: u64,
    done: &str,
    until: Option<u32>,
    fixture: &[(u32, u64)],
) -> Option<(Machine, Stop, String)> {
    let d = dir()?;
    let mut m = Machine::new();
    load(&mut m, &d, "subsm");
    load(&mut m, &d, name);
    for &(a, w) in fixture {
        m.deposit(a, pdp10::W36::wrap(w))
            .expect("address in memory");
    }
    m.pc = Half::wrap(0o30000);
    let mut script = Script::default();
    script.answer("TTY SWITCH CONTROL", "Y\r");
    script.answer("LH SWITCHES", &format!("{lh}\r"));
    script.answer("RH SWITCHES", &format!("{rh}\r"));
    let mut seen = 0;
    let stop = loop {
        if m.count >= limit || until.is_some_and(|u| m.pc.get() as u32 == u) {
            break Stop::Limit;
        }
        let n = m.console.output.len();
        if !done.is_empty() && n != seen {
            // Only the new text, with room for a match across the edge.
            let from = seen.saturating_sub(done.len());
            seen = n;
            if m.console.output[from..]
                .windows(done.len())
                .any(|w| w == done.as_bytes())
            {
                break Stop::Limit;
            }
        }
        script.tick(m.count, &mut m.console);
        if let Err(s) = m.step() {
            break s;
        }
    };
    let out = String::from_utf8_lossy(&m.console.output).replace('\r', "");
    Some((m, stop, out))
}

/// The result of one program, or why it failed.
fn one(name: &str, how: &How, rh: &str, iterations: u64) -> Result<String, String> {
    let lower = name.to_lowercase();
    match how {
        How::Skip(why) => Ok(format!("{name}: not run, it tests {why}")),
        How::Alone => {
            let d = dir().expect("KS10_DIAG_DIR");
            let mut m = Machine::new();
            load(&mut m, &d, &lower);
            m.pc = Half::wrap(0o30000);
            // PASCNT counts up from 0 when the program runs alone.
            while m.mem[0o30047].get() < iterations || m.mem[0o30047].bit(0) {
                if m.count > 200_000_000 {
                    return Err(format!(
                        "{name}: PASCNT {} after {} instructions",
                        m.mem[0o30047], m.count
                    ));
                }
                let stop = m.run(10_000);
                if stop != Stop::Limit {
                    return Err(format!("{name}: {stop:?} at PC {}", m.pc));
                }
            }
            Ok(format!(
                "{name}: PASCNT {} after {} instructions",
                m.mem[0o30047], m.count
            ))
        }
        How::Subsm(lh) => {
            let want = format!("END PASS {iterations}.");
            let (m, stop, out) = subsm_run(&lower, (lh, rh), 1_000_000_000, &want, None, &[])
                .expect("KS10_DIAG_DIR");
            if stop != Stop::Limit {
                return Err(format!("{name}: {stop:?} at PC {}\n{out}", m.pc));
            }
            if out.contains("ERROR") {
                return Err(format!("{name}: an error message\n{out}"));
            }
            if !out.contains(&want) {
                return Err(format!(
                    "{name}: no \"{want}\" after {} instructions\n{out}",
                    m.count
                ));
            }
            Ok(format!("{name}: \"{want}\" after {} instructions", m.count))
        }
        How::Dskcg => {
            // SUBSM clears UBR with DATAO PAG,[LLDUSB,,400000] (SUBSM.SEQ,
            // line 1959; the literal is at 7103). In the DEC format the
            // right half is a page number in bits 23 to 35, so 400000 is
            // page 0. The ITS microcode takes bits 16 to 35 as the address
            // (docs/discrepancies.md), so the test writes 100000,,0, the
            // same UBR in the ITS format. DSKCG puts its MUUO handler at UPT
            // 430 for a UPT at 0 (DSKCGM.MAC, MUUOHS). Test 9 (page fail
            // recovery) needs the KI style paging of the DEC microcode
            // (DSKCGM.MAC, PAGSET): the run ends at the first PAGSET (035223
            // in DSKCG.SEQ), after tests 1 to 8.
            let fixture = [(0o7103, 0o100000_000000)];
            let (m, stop, out) = subsm_run(
                "dskcg",
                ("000100", rh),
                50_000_000,
                "",
                Some(0o35223),
                &fixture,
            )
            .expect("KS10_DIAG_DIR");
            if stop != Stop::Limit || m.pc.get() != 0o35223 {
                return Err(format!(
                    "{name}: {stop:?} at PC {}, not at test 9\n{out}",
                    m.pc
                ));
            }
            if out.contains("ERROR") {
                return Err(format!("{name}: an error message\n{out}"));
            }
            Ok(format!(
                "{name}: tests 1 to 8 without an error, {} instructions",
                m.count
            ))
        }
    }
}

#[test]
fn smcpu_run_file() {
    let Some(d) = dir() else {
        eprintln!("KS10_DIAG_DIR is not set: diagnostics not run");
        return;
    };
    let list = run_file(&d);
    assert!(!list.is_empty(), "SMCPU.CMD has no programs");
    let jobs: Vec<(String, How, String, u64)> = list
        .into_iter()
        .map(|(n, rh, i)| {
            let h = how(&n, DSKEB)
                .unwrap_or_else(|| panic!("{n}: a program of SMCPU.CMD without a decision"));
            (n, h, rh, i)
        })
        .collect();
    // The programs are independent: one thread for each.
    let results: Vec<Result<String, String>> = std::thread::scope(|s| {
        let hs: Vec<_> = jobs
            .iter()
            .map(|(n, h, rh, i)| s.spawn(move || one(n, h, rh, *i)))
            .collect();
        hs.into_iter()
            .map(|h| h.join().expect("a thread"))
            .collect()
    });
    let mut failed = Vec::new();
    for r in results {
        match r {
            Ok(t) => eprintln!("{t}"),
            Err(e) => {
                eprintln!("FAIL {e}");
                failed.push(e.lines().next().unwrap_or("").to_string());
            }
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}

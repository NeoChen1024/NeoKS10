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

use std::path::PathBuf;

use ks10::console::Script;
use ks10::sav::{self, Packing};
use ks10::{Machine, Stop};
use pdp10::Half;

fn dir() -> Option<PathBuf> {
    std::env::var_os("KS10_DIAG_DIR").map(PathBuf::from)
}

fn load(m: &mut Machine, d: &std::path::Path, name: &str) {
    let bytes =
        std::fs::read(d.join(format!("diagnostics..{name}.sav"))).expect("read the .SAV file");
    let s = sav::parse(&sav::words(&bytes, Packing::Ansi5)).expect("a SAV file");
    for (addr, ws) in &s.blocks {
        for (k, w) in ws.iter().enumerate() {
            m.deposit(addr.get() as u32 + k as u32, *w)
                .expect("address in memory");
        }
    }
}

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

/// How a program of SMCPU.CMD runs here.
enum How {
    /// Alone: the PASCNT criterion.
    Alone,
    /// Under SUBSM with these left half switches.
    Subsm(&'static str),
    /// DSKCG: under SUBSM, up to the first instruction of test 9.
    Dskcg,
    /// Not for the ITS microcode, with the reason.
    Skip(&'static str),
}

/// The switches: LH 000100 is INHPAG, inhibit paging and traps
/// (DSKAM.TXT, 4.0; PARAM, INHPAG==000100). The program would else enable
/// the KI style pager of the DEC microcode, which the ITS microcode does
/// not have. DSKBA also needs OPRSEL (000010): without it, the program
/// limits memory to 32K, and with INHPAG SUBSM gives too little memory
/// (docs/emulator.md, "Diagnostics").
fn how(name: &str) -> Option<How> {
    Some(match name {
        "DSKAA" | "DSKAB" | "DSKAC" | "DSKAD" | "DSKAE" | "DSKAF" | "DSKAG" | "DSKAH" => How::Alone,
        "DSKAI" | "DSKAJ" | "DSKAK" | "DSKAL" | "DSKAM" | "DSKCA" | "DSKCB" | "DSKCC" | "DSKCD"
        | "DSKCE" | "DSKCF" => How::Subsm("000100"),
        "DSKBA" => How::Subsm("000110"),
        "DSKCG" => How::Dskcg,
        "DSKEA" => How::Skip("the paging of the DEC microcode"),
        "DSKEB" => How::Skip("the cache, which this emulator does not have"),
        "DSKEC" => How::Skip("the KL paging of the DEC microcode"),
        _ => return None,
    })
}

/// The lines of SMCPU.CMD: the name, the right half of the switches
/// (octal) and the iterations.
fn run_file(d: &std::path::Path) -> Vec<(String, String, u64)> {
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
        // SMMON.TXT, 5.0 E: zero is one iteration.
        list.push((f[0].to_string(), f[2].to_string(), iterations.max(1)));
    }
    list
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
            let h =
                how(&n).unwrap_or_else(|| panic!("{n}: a program of SMCPU.CMD without a decision"));
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

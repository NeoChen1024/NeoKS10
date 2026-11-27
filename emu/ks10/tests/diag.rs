//! The DEC KS10 diagnostics that run alone (docs/emulator.md,
//! "Diagnostics"). The programs are third-party files and are not in git:
//! the test runs only if KS10_DIAG_DIR names a directory with the .SAV
//! files (5-byte words, as t10backup extracts them, with the names
//! "diagnostics..dskaa.sav" and so on).
//!
//! A program passes if it does not halt in its first N instructions and its
//! pass count (location 30047, PASCNT; DSKAA.TXT, section 5.0) grows.
//! DSKAI to DSKAM run under the diagnostic monitor SUBSM (DSKAM.TXT, 3.2):
//! they also must print "END PASS 10." and no error message on the CTY.

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

fn run(name: &str, limit: u64) {
    let Some(d) = dir() else {
        eprintln!("KS10_DIAG_DIR is not set: {name} not run");
        return;
    };
    let mut m = Machine::new();
    load(&mut m, &d, name);
    m.pc = Half::wrap(0o30000);
    let stop = m.run(limit);
    assert_eq!(stop, Stop::Limit, "{name}: PC {}", m.pc);
    assert!(
        m.mem[0o30047].get() > 10,
        "{name}: PASCNT {}",
        m.mem[0o30047]
    );
}

#[test]
fn basic_instruction_diagnostics() {
    for name in [
        "dskaa", "dskab", "dskac", "dskad", "dskae", "dskaf", "dskag", "dskah",
    ] {
        run(name, 5_000_000);
    }
}

/// Run a diagnostic under SUBSM. The answers select the switches by hand
/// ("Y"): LH 000100 is INHPAG, inhibit paging and traps (DSKAM.TXT, 4.0;
/// PARAM, INHPAG==000100). The program would else enable the KI style
/// pager of the DEC microcode, which the ITS microcode does not have.
fn run_subsm(name: &str, limit: u64) {
    let Some(d) = dir() else {
        eprintln!("KS10_DIAG_DIR is not set: {name} not run");
        return;
    };
    let mut m = Machine::new();
    load(&mut m, &d, "subsm");
    load(&mut m, &d, name);
    m.pc = Half::wrap(0o30000);
    let mut script = Script::default();
    script.answer("TTY SWITCH CONTROL", "Y\r");
    script.answer("LH SWITCHES", "000100\r");
    script.answer("RH SWITCHES", "0\r");
    let stop = loop {
        if m.count >= limit {
            break Stop::Limit;
        }
        script.tick(m.count, &mut m.console);
        if let Err(s) = m.step() {
            break s;
        }
    };
    let out = String::from_utf8_lossy(&m.console.output).replace('\r', "");
    assert_eq!(stop, Stop::Limit, "{name}: PC {}\n{out}", m.pc);
    assert!(out.contains("END PASS 10."), "{name}:\n{out}");
    assert!(!out.contains("ERROR"), "{name}:\n{out}");
    assert!(
        m.mem[0o30047].get() > 10,
        "{name}: PASCNT {}",
        m.mem[0o30047]
    );
}

#[test]
fn basic_instruction_diagnostics_under_subsm() {
    for name in ["dskai", "dskaj", "dskak", "dskal", "dskam"] {
        run_subsm(name, 3_000_000);
    }
}

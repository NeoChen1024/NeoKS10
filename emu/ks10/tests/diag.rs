//! The DEC KS10 diagnostics that run alone (docs/emulator.md,
//! "Diagnostics"). The programs are third-party files and are not in git:
//! the test runs only if KS10_DIAG_DIR names a directory with the .SAV
//! files (5-byte words, as t10backup extracts them, with the names
//! "diagnostics..dskaa.sav" and so on).
//!
//! A program passes if it does not halt in its first N instructions and its
//! pass count (location 30047, PASCNT; DSKAA.TXT, section 5.0) grows.
//! The others run under the diagnostic monitor SUBSM (DSKAM.TXT, 3.2): they
//! also must print "END PASS 10." and no error message on the CTY.

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

/// Load SUBSM and a program, type the switch answers, and step until
/// `until` is the PC or `limit` steps are done.
fn subsm_run(
    name: &str,
    lh: &str,
    limit: u64,
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
    script.answer("RH SWITCHES", "0\r");
    let stop = loop {
        if m.count >= limit || until.is_some_and(|u| m.pc.get() as u32 == u) {
            break Stop::Limit;
        }
        script.tick(m.count, &mut m.console);
        if let Err(s) = m.step() {
            break s;
        }
    };
    let out = String::from_utf8_lossy(&m.console.output).replace('\r', "");
    Some((m, stop, out))
}

/// Run a diagnostic under SUBSM. The answers select the switches by hand
/// ("Y"): LH 000100 is INHPAG, inhibit paging and traps (DSKAM.TXT, 4.0;
/// PARAM, INHPAG==000100). The program would else enable the KI style
/// pager of the DEC microcode, which the ITS microcode does not have.
fn run_subsm(name: &str, lh: &str, limit: u64) {
    let Some((m, stop, out)) = subsm_run(name, lh, limit, None, &[]) else {
        eprintln!("KS10_DIAG_DIR is not set: {name} not run");
        return;
    };
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
        run_subsm(name, "000100", 3_000_000);
    }
}

/// The advanced instruction diagnostics: multiply and divide, double word,
/// floating point, and more (DSKCA.TXT to DSKCF.TXT, "ABSTRACT").
#[test]
fn advanced_instruction_diagnostics() {
    for name in ["dskca", "dskcb", "dskcc", "dskcd", "dskce"] {
        run_subsm(name, "000100", 3_000_000);
    }
    run_subsm("dskcf", "000100", 6_000_000);
}

/// DSKCG tests EXTEND. Two fixtures make it fit the ITS microcode:
///
/// - SUBSM clears UBR with DATAO PAG,[LLDUSB,,400000] (SUBSM.SEQ, line
///   1959; the literal is at 7103). In the DEC format the right half is a
///   page number in bits 23 to 35, so 400000 is page 0. The ITS microcode
///   takes bits 16 to 35 as the address (docs/discrepancies.md), so the
///   test writes 100000,,0, the same UBR in the ITS format. DSKCG puts its
///   MUUO handler at UPT 430 for a UPT at 0 (DSKCGM.MAC, MUUOHS).
/// - Test 9 (page fail recovery) needs the KI style paging of the DEC
///   microcode (DSKCGM.MAC, PAGSET). The run stops at the first PAGSET
///   (035223 in DSKCG.SEQ), after tests 1 to 8.
#[test]
fn extend_diagnostic() {
    let fixture = [(0o7103, 0o100000_000000)];
    let Some((m, stop, out)) = subsm_run("dskcg", "000100", 5_000_000, Some(0o35223), &fixture)
    else {
        eprintln!("KS10_DIAG_DIR is not set: dskcg not run");
        return;
    };
    assert_eq!(stop, Stop::Limit);
    assert_eq!(m.pc.get(), 0o35223, "dskcg did not reach test 9\n{out}");
    assert!(!out.contains("ERROR"), "dskcg:\n{out}");
}

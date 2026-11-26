//! The DEC KS10 diagnostics that run alone (docs/emulator.md,
//! "Diagnostics"). The programs are third-party files and are not in git:
//! the test runs only if KS10_DIAG_DIR names a directory with the .SAV
//! files (5-byte words, as t10backup extracts them, with the names
//! "diagnostics..dskaa.sav" and so on).
//!
//! A program passes if it does not halt in its first N instructions and its
//! pass count (location 30047, PASCNT; DSKAA.TXT, section 5.0) grows.

use std::path::PathBuf;

use ks10::sav::{self, Packing};
use ks10::{Machine, Stop};
use pdp10::Half;

fn dir() -> Option<PathBuf> {
    std::env::var_os("KS10_DIAG_DIR").map(PathBuf::from)
}

fn run(name: &str, limit: u64) {
    let Some(d) = dir() else {
        eprintln!("KS10_DIAG_DIR is not set: {name} not run");
        return;
    };
    let bytes =
        std::fs::read(d.join(format!("diagnostics..{name}.sav"))).expect("read the .SAV file");
    let s = sav::parse(&sav::words(&bytes, Packing::Ansi5)).expect("a SAV file");
    let mut m = Machine::new();
    for (addr, ws) in &s.blocks {
        for (k, w) in ws.iter().enumerate() {
            m.deposit(addr.get() as u32 + k as u32, *w)
                .expect("address in memory");
        }
    }
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
    for name in ["dskaa", "dskab", "dskac", "dskad", "dskae", "dskaf"] {
        run(name, 5_000_000);
    }
}

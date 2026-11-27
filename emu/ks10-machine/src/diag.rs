//! The DEC KS10 diagnostics (docs/emulator.md, "Diagnostics"): the
//! programs of the run file SMCPU.CMD and how each processor runs them.
//!
//! The programs are third-party files and are not in git. `make diag`
//! extracts them from the tape into a directory that `KS10_DIAG_DIR`
//! names: .SAV files of 5-byte words, as `t10backup` writes them, with the
//! names "diagnostics..dskaa.sav" and so on.

use std::path::{Path, PathBuf};

use crate::boot::Processor;
use crate::sav;

/// The directory of the .SAV files, if `KS10_DIAG_DIR` names one.
pub fn dir() -> Option<PathBuf> {
    std::env::var_os("KS10_DIAG_DIR").map(PathBuf::from)
}

/// Load the program `name` (lower case, without ".sav") into memory.
/// Panics if the file is missing or bad: a test calls it.
pub fn load(p: &mut impl Processor, d: &Path, name: &str) {
    let bytes =
        std::fs::read(d.join(format!("diagnostics..{name}.sav"))).expect("read the .SAV file");
    let s = sav::parse(&sav::words(&bytes)).expect("a SAV file");
    let (mem, _, _) = p.bus();
    for (addr, ws) in &s.blocks {
        for (k, w) in ws.iter().enumerate() {
            *mem.get_mut(addr.get() as usize + k)
                .expect("address in memory") = *w;
        }
    }
}

/// A line of SMCPU.CMD: the program, the right half of the switches and
/// the iterations (SMMON.TXT, section 5.0: name, passes, switches,
/// iterations in octal; zero is one iteration).
pub fn run_file(d: &Path) -> Vec<(String, String, u64)> {
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

/// How a program of the run file runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    /// Alone: the PASCNT criterion.
    Alone,
    /// Under SUBSM with these left half switches.
    Subsm(&'static str),
    /// DSKCG: under SUBSM, up to the first instruction of test 9.
    Dskcg,
    /// Not for the ITS microcode, with the reason.
    Skip(&'static str),
}

/// The decision for each program of the run file. DSKEB tests the cache,
/// and the reason not to run it depends on the processor. A program
/// without a decision gives None, so that a new one gets one.
pub fn how(name: &str, dskeb: &'static str) -> Option<How> {
    Some(match name {
        "DSKAA" | "DSKAB" | "DSKAC" | "DSKAD" | "DSKAE" | "DSKAF" | "DSKAG" | "DSKAH" => How::Alone,
        "DSKAI" | "DSKAJ" | "DSKAK" | "DSKAL" | "DSKAM" | "DSKCA" | "DSKCB" | "DSKCC" | "DSKCD"
        | "DSKCE" | "DSKCF" => How::Subsm("000100"),
        "DSKBA" => How::Subsm("000110"),
        "DSKCG" => How::Dskcg,
        "DSKEA" => How::Skip("the paging of the DEC microcode"),
        "DSKEB" => How::Skip(dskeb),
        "DSKEC" => How::Skip("the KL paging of the DEC microcode"),
        _ => return None,
    })
}

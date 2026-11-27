//! The TM03 with the tapes of the ITS build (docs/emulator.md, "Image
//! formats in step 0"). The images are third-party files and are not in
//! git: the tests run only if KS10_ITS_DISK names `rp0.dsk` of the build
//! output; the tapes `dskdmp.tape` and `minsys.tape` are in the same
//! directory. The tests use copies.
//!
//! The steps are those of the ITS build (build/pdp10-ks/build.tcl,
//! start_dskdmp, and build/build.tcl, the DUMP commands):
//!
//! - `read_tapes`: the 8080 MB command on transport 2 boots DSKDMP from
//!   `dskdmp.tape` ("MTBOOT", then ESC G); DSKDMP starts ITS from the
//!   disk. ITS lists `minsys.tape` on transport 0 with DUMP. The expected
//!   names are the files that the build wrote to that tape.
//! - `write_tape`: ITS copies a file, dumps it to an empty tape, checks
//!   the tape against the disk (ICHECK) and lists the tape.
//!
//! [measured] 2026-11-27: about 140 million steps each, 6 s each in a
//! release build.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use ks10_core_soft::console::Script;
use ks10_core_soft::{Machine, boot};

struct Copies(Vec<PathBuf>);

impl Drop for Copies {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// A writable copy of `src` (or an empty file) in the target directory.
fn copy(copies: &mut Copies, src: Option<&Path>, name: &str) -> File {
    let dst = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    match src {
        Some(s) => {
            std::fs::copy(s, &dst).expect("copy an image");
        }
        None => std::fs::write(&dst, []).expect("make an empty tape"),
    }
    copies.0.push(dst.clone());
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&dst)
        .expect("open a copy")
}

fn text(m: &Machine) -> String {
    m.console
        .output
        .iter()
        .map(|&b| char::from(b & 0o177))
        .filter(|&c| c != '\r' && c != '\0')
        .collect()
}

/// Run until `done` is in the console output, at most `limit` steps.
fn run(m: &mut Machine, script: &mut Script, done: &str, limit: u64) -> String {
    while m.count < limit {
        script.tick(m.count, &mut m.console);
        if let Err(s) = m.step() {
            panic!("stop {s:?}\n{}", text(m));
        }
        if m.count.is_multiple_of(1_000_000) && text(m).contains(done) {
            break;
        }
    }
    text(m)
}

/// The ITS start and the login (tests/its.rs) with DUMP.
fn login_and_dump(script: &mut Script, dskdmp_from_tape: bool) {
    if dskdmp_from_tape {
        script.answer("MTBOOT", "\x1bg");
    }
    script.answer("DSKDMP", "its\r");
    script.answer("\r", "\x1bg");
    script.answer("DELRNM", "\x1a");
    script.answer("Fair", ":login neoks\r");
}

#[test]
fn read_tapes() {
    let Some(disk) = std::env::var_os("KS10_ITS_DISK") else {
        eprintln!("KS10_ITS_DISK is not set: tape test not run");
        return;
    };
    let disk = PathBuf::from(disk);
    let dir = disk.parent().expect("the build output directory");
    let mut copies = Copies(Vec::new());
    let mut m = Machine::new();
    let f = copy(&mut copies, Some(&disk), "tape-read-rp0.dsk");
    m.io.disk.attach(0, f, false);
    let f = copy(
        &mut copies,
        Some(&dir.join("dskdmp.tape")),
        "tape-read-dskdmp.tape",
    );
    m.io.tape.attach(2, f, true);
    let f = copy(
        &mut copies,
        Some(&dir.join("minsys.tape")),
        "tape-read-minsys.tape",
    );
    m.io.tape.attach(0, f, true);
    boot::boot_tape(&mut m, 2).expect("MB");
    let mut script = Script::default();
    login_and_dump(&mut script, true);
    script.answer("LOGIN  NEOKS", ":dump\r");
    script.answer("DUMP  .", "list\r");
    script.answer("LIST DEV =", "\r");
    let out = run(&mut m, &mut script, "REEL", 900_000_000);
    assert!(out.contains("ITS MTBOOT"), "{out}");
    assert!(out.contains("IN OPERATION"), "{out}");
    // build/ks10/include.tcl and build.tcl write these files to the tape.
    for name in [
        "SALV   RP06",
        "DSKDMP RP06",
        "SYS    TS     DUMP",
        "SYS    TS     PDSET",
    ] {
        assert!(out.contains(name), "{name}\n{out}");
    }
    assert!(out.contains("E-O-T"), "{out}");
}

#[test]
fn write_tape() {
    let Some(disk) = std::env::var_os("KS10_ITS_DISK") else {
        eprintln!("KS10_ITS_DISK is not set: tape test not run");
        return;
    };
    let mut copies = Copies(Vec::new());
    let mut m = Machine::new();
    let f = copy(&mut copies, Some(Path::new(&disk)), "tape-write-rp0.dsk");
    m.io.disk.attach(0, f, false);
    let f = copy(&mut copies, None, "tape-write-blank.tape");
    m.io.tape.attach(0, f, false);
    boot::boot_disk(&mut m, 0).expect("BT");
    let mut script = Script::default();
    login_and_dump(&mut script, false);
    script.answer("LOGIN  NEOKS", ":copy sys;ts pdset,sys;ts pdtest\r");
    script.answer("*", ":dump\r");
    script.answer("DUMP  .", "dump\r");
    script.answer("FILE=", "sys;ts pdtest\r");
    script.answer("FILE=", "\r");
    script.answer("TAPE NO=", "1\r");
    script.answer("_", "rewind\r");
    script.answer("_", "icheck\r");
    script.answer("_", "rewind\r");
    script.answer("_", "list\r");
    script.answer("LIST DEV =", "\r");
    let out = run(&mut m, &mut script, "REEL", 900_000_000);
    let listing = out.rsplit("_list").next().unwrap_or_default();
    assert!(listing.contains("SYS    TS     PDTEST"), "{out}");
    assert!(listing.contains("E-O-T"), "{out}");
    // ICHECK prints only the tape header when the tape agrees with the
    // disk.
    let check = out
        .split("_icheck")
        .nth(1)
        .and_then(|x| x.split("\n_").next())
        .unwrap_or_default();
    let lines: Vec<&str> = check.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "{check}");
    assert!(lines[0].contains("TAPE NO"), "{check}");
    assert!(!out.contains("FORMATTER ERROR"), "{out}");
}

//! Boot ITS from the disk image of the ITS build (docs/emulator.md,
//! "Image formats in step 0"). The image is a third-party file and is not
//! in git: the test runs only if KS10_ITS_DISK names it. The test boots a
//! copy, because ITS writes to its disk.
//!
//! The steps are those of the ITS build (build/ks10/include.tcl and
//! build/build.tcl of the ITS tree): the 8080 BT command, "its" and ESC G
//! at the DSKDMP prompt. The test then logs in on the console and lists
//! the directory SYS. It passes if ITS prints "IN OPERATION" and the
//! listing shows the free blocks.
//! [measured] 2026-11-27: about 140 million steps, 6 s in a release build.

use ks10_core_soft::console::Script;
use ks10_core_soft::{Machine, boot};

#[test]
fn boot_its_and_list_sys() {
    let Some(src) = std::env::var_os("KS10_ITS_DISK") else {
        eprintln!("KS10_ITS_DISK is not set: ITS boot not run");
        return;
    };
    let copy = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("its-boot-rp0.dsk");
    std::fs::copy(&src, &copy).expect("copy the disk image");
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&copy)
        .expect("open the copy");
    let mut m = Machine::new();
    m.io.disk.attach(0, f, false);
    boot::boot_disk(&mut m, 0).expect("BT");
    let mut script = Script::default();
    script.answer("DSKDMP", "its\r");
    script.answer("\r", "\x1bg");
    script.answer("DELRNM", "\x1a");
    script.answer("Fair", ":login neoks\r");
    script.answer("LOGIN  NEOKS", ":listf sys;\r");
    let mut seen = false;
    while m.count < 800_000_000 && !seen {
        script.tick(m.count, &mut m.console);
        if let Err(s) = m.step() {
            panic!("stop {s:?}");
        }
        if m.count.is_multiple_of(1_000_000) {
            seen = String::from_utf8_lossy(&m.console.output).contains("FREE BLOCKS");
        }
    }
    let out = String::from_utf8_lossy(&m.console.output).replace('\r', "");
    let _ = std::fs::remove_file(&copy);
    // ITS rings the bell before " IN OPERATION".
    assert!(out.contains("IN OPERATION"), "{out}");
    assert!(out.contains("FREE BLOCKS"), "{out}");
}

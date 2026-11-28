//! Boot ITS on the model from the disk image of the ITS build, log in
//! and list a directory.
//!
//! The 8080 BT command (`ks10_machine::boot::boot_disk`) loads the boot
//! loader from a copy of the image and starts it at 1000. The answers are
//! those of `tests/its-boot/boot.script` and of `emu/ks10-core-soft/tests/idle.rs`:
//! "its" and ESC G at the DSKDMP prompt, then a login on the console and
//! `:listf sys;`. The test passes when the listing shows the free blocks.
//! `ks10-core-soft` boots ITS in its own test; the two processors do not
//! run in step (docs/microcode-model.md, "Time and the boot of ITS").
//!
//! The test needs `KS10_ITS_DISK` (`make microcode` sets it).

mod common;

use std::path::{Path, PathBuf};

use common::{FILLS, machine};
use ks10_machine::boot;
use ks10_machine::console::Script;

/// The most microcycles without an instruction. The longest
/// instruction of the boot is a BLT that clears memory: about 1.05
/// million microcycles.
const STALL_CYCLES: u64 = 10_000_000;

/// The longest run, in instructions. `ks10-core-soft` lists SYS; after
/// about 138 million.
const LIMIT: u64 = 300_000_000;

fn copy_image(src: &Path, name: &str) -> (PathBuf, std::fs::File) {
    let copy = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::copy(src, &copy).expect("copy the disk image");
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&copy)
        .expect("open the copy");
    (copy, f)
}

#[test]
fn boot_its_from_disk() {
    let Some(src) = std::env::var_os("KS10_ITS_DISK") else {
        println!("SKIPPED: ITS boot: KS10_ITS_DISK is not set");
        return;
    };
    let (copy, f) = copy_image(Path::new(&src), "uop-boot-model.dsk");
    let mut m = machine(FILLS[0]);
    m.io.disk.attach(0, f, false);
    boot::boot_disk(&mut m, 0).expect("BT on the model");

    let mut script = Script::default();
    script.answer("DSKDMP", "its\r");
    script.answer("\r", "\x1bg");
    script.answer("DELRNM", "\x1a");
    script.answer("Fair", ":login neoks\r");
    script.answer("LOGIN  NEOKS", ":listf sys;\r");
    let mut cty = Vec::new();
    let mut stop = None;
    let mut last_progress = (u64::MAX, 0);
    let mut milestones = Vec::new();
    let mut listed = false;
    while m.instructions < LIMIT {
        if m.instructions != last_progress.0 {
            last_progress = (m.instructions, m.cycles);
            script.tick(m.now(), &mut m.cty);
            if m.cty.output.len() > cty.len() {
                cty.extend_from_slice(&m.cty.output[cty.len()..]);
                let out = String::from_utf8_lossy(&cty);
                for text in ["IN OPERATION", "LOGIN  NEOKS", "FREE BLOCKS"] {
                    if out.contains(text) && !milestones.iter().any(|&(t, _)| t == text) {
                        milestones.push((text, m.instructions));
                    }
                }
                if out.contains("FREE BLOCKS") {
                    listed = true;
                    break;
                }
            }
        } else if m.cycles - last_progress.1 > STALL_CYCLES {
            // A word loop without SPEC/NICOND (the halt loop, a wait)
            // would never end the run.
            stop = Some(format!(
                "no instruction for {STALL_CYCLES} microcycles, at {:04o}; location 0 {:o}, 1 {:o}",
                m.upc,
                m.memory[0].get(),
                m.memory[1].get()
            ));
            break;
        }
        if let Err(e) = m.step() {
            stop = Some(e.to_string());
            break;
        }
    }
    let _ = std::fs::remove_file(&copy);
    println!(
        "{} instructions, {} microcycles, {} cache hits, stop: {}",
        m.instructions,
        m.cycles,
        m.cache_hits,
        stop.as_deref().unwrap_or("none")
    );
    for (text, n) in &milestones {
        println!("{text:?} after {n} instructions");
    }
    println!("CTY: {}", String::from_utf8_lossy(&cty));
    assert!(listed, "no listing of SYS;");
}

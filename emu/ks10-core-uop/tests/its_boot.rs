//! Boot ITS on the model from the disk image of the ITS build.
//!
//! The 8080 BT command (`ks10_machine::boot::boot_disk`) loads the boot
//! loader from a copy of the image and starts it at 1000. The test answers
//! the DSKDMP prompt as `tests/its-boot/boot.script` does, and passes when
//! ITS prints "IN OPERATION" on the CTY. `ks10-core-soft` boots ITS in
//! its own test (`emu/ks10-core-soft/tests/its.rs`); the two processors do
//! not run in step (docs/microcode-model.md, "Time and the boot of ITS").
//!
//! The test needs `KS10_ITS_DISK` (`make microcode` sets it).

mod common;

use std::path::{Path, PathBuf};

use common::{FILLS, machine};
use ks10_machine::boot;

/// The most microcycles without an instruction. The longest
/// instruction of the boot is a BLT that clears memory: about 1.05
/// million microcycles.
const STALL_CYCLES: u64 = 10_000_000;

/// The longest run, in instructions.
const LIMIT: u64 = 40_000_000;

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

    // The answers at the DSKDMP prompt (tests/its-boot/boot.script).
    let mut script: Vec<(&str, &[u8])> = vec![("DSKDMP", b"its\r"), ("its", b"\x1bg")];
    script.reverse();
    let mut cty = Vec::new();
    let mut stop = None;
    let mut last_progress = (0, 0);
    let mut in_operation = false;
    while m.instructions < LIMIT {
        // A word loop without SPEC/NICOND (the halt loop, a wait) would
        // never end the run.
        if m.instructions != last_progress.0 {
            last_progress = (m.instructions, m.cycles);
        } else if m.cycles - last_progress.1 > STALL_CYCLES {
            stop = Some(format!(
                "no instruction for {STALL_CYCLES} microcycles, at {:04o}; location 0 {:o}, 1 {:o}",
                m.upc,
                m.memory[0].get(),
                m.memory[1].get()
            ));
            break;
        }
        if !m.cty.output.is_empty() {
            cty.append(&mut m.cty.output);
            let out = String::from_utf8_lossy(&cty).to_lowercase();
            if let Some(&(prompt, answer)) = script.last() {
                if out.contains(&prompt.to_lowercase()) {
                    m.cty.input.extend(answer);
                    cty.clear();
                    script.pop();
                }
            } else if out.contains("in operation") {
                in_operation = true;
                break;
            }
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
    println!("CTY: {}", String::from_utf8_lossy(&cty));
    assert!(in_operation, "ITS is not in operation");
}

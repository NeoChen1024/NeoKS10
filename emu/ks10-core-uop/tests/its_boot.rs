//! Boot ITS on the model as far as the model goes, in step with the
//! software processor.
//!
//! The 8080 BT command (`ks10_machine::boot::boot_disk`) loads the boot
//! loader from the disk image of the ITS build and starts it at 1000. Both
//! processors boot their own copy of the image. After each instruction of
//! the model, the test compares PC and the 16 ACs of the current block
//! with `ks10-core-soft`. The test passes if the two agree at every
//! instruction until the model stops with a function that it does not
//! have. It prints the number of instructions and the stop.
//!
//! The console starts a program with EXECUTE of JRST (cmds.m80, STINT),
//! which the model counts as an instruction; `ks10-core-soft` sets PC
//! instead. So instruction k + 1 of the model is step k of
//! `ks10-core-soft`.
//!
//! The test needs `KS10_ITS_DISK` and `KS10_MCR_LISTING` (`make microcode`
//! sets both).

mod common;

use std::path::{Path, PathBuf};

use common::{FILLS, machine, symbol};
use ks10_core_uop::alu;
use ks10_machine::boot;

/// The longest run, in instructions.
const LIMIT: u64 = 10_000_000;

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
fn boot_from_disk_in_step_with_the_software_processor() {
    let Some(text) = common::listing_text("ITS boot") else {
        return;
    };
    let Some(src) = std::env::var_os("KS10_ITS_DISK") else {
        println!("SKIPPED: ITS boot: KS10_ITS_DISK is not set");
        return;
    };
    let (copy_m, fm) = copy_image(Path::new(&src), "uop-boot-model.dsk");
    let (copy_s, fs) = copy_image(Path::new(&src), "uop-boot-soft.dsk");
    let mut m = machine(&text, FILLS[0]);
    m.io.disk.attach(0, fm, false);
    boot::boot_disk(&mut m, 0).expect("BT on the model");
    let mut s = ks10_core_soft::Machine::new();
    s.io.disk.attach(0, fs, false);
    boot::boot_disk(&mut s, 0).expect("BT on ks10-core-soft");
    let pc_reg = symbol(&text, "PC");
    // The power-up sequence leaves values in the ACs (it uses them for the
    // table of powers of ten); no program can depend on them. Both
    // processors start with the ACs of the model.
    let block = usize::from(m.cur_block & 7) << 4;
    for k in 0..16 {
        s.ac[k] = m.ram_file[block + k];
    }

    let mut stop = None;
    let mut difference = None;
    while m.instructions < LIMIT {
        let before = m.instructions;
        if let Err(e) = m.step() {
            stop = Some(e);
            break;
        }
        if m.instructions == before || m.instructions < 2 {
            continue;
        }
        if let Err(e) = s.step() {
            difference = Some(format!("ks10-core-soft stops: {e:?}"));
            break;
        }
        let pc = alu::word(m.alu.reg[pc_reg]).get() & 0o777777;
        let block = usize::from(m.cur_block & 7) << 4;
        let acs: Vec<u64> = (0..16).map(|k| m.ram_file[block + k].get()).collect();
        let want: Vec<u64> = s.ac.iter().map(|w| w.get()).collect();
        if pc != s.pc.get() || acs != want {
            let octal = |v: &[u64]| {
                v.iter()
                    .map(|w| format!("{w:o}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            difference = Some(format!(
                "instruction {}: model PC {pc:o} ACs {}\n  ks10-core-soft PC {:o} ACs {}",
                m.instructions,
                octal(&acs),
                s.pc.get(),
                octal(&want)
            ));
            break;
        }
    }
    let _ = std::fs::remove_file(&copy_m);
    let _ = std::fs::remove_file(&copy_s);
    let stop = stop.map_or("none".to_string(), |e| e.to_string());
    println!(
        "{} instructions, {} microcycles, stop: {stop}",
        m.instructions, m.cycles
    );
    assert_eq!(difference, None);
}

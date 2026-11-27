//! Boot ITS on the model as far as the model goes, in step with the
//! software processor.
//!
//! The 8080 BT command (`ks10_machine::boot::boot_disk`) loads the boot
//! loader from the disk image of the ITS build and starts it at 1000. Both
//! processors boot their own copy of the image. After each instruction of
//! the model, the test compares PC, the program flags and the 16 ACs of
//! the current block with `ks10-core-soft`. When the model stops with a
//! function that it does not have, the test compares main memory. The
//! test passes if the two agree, and if the model gets at least as far as
//! [`PROGRESS`]. It prints the number of instructions and the stop.
//!
//! The console starts a program with EXECUTE of JRST (cmds.m80, STINT),
//! which the model counts as an instruction; `ks10-core-soft` sets PC
//! instead. So instruction k + 1 of the model is step k of
//! `ks10-core-soft`. With RUN 0 the JRST ends in a console halt (inout,
//! `HALTED`), which writes locations 0 and 1 and the halt status block at
//! 500 (simple, power-up: "ITS makes it 500"). `ks10-core-soft` does not
//! run that halt, so these words are known differences.
//!
//! The test needs `KS10_ITS_DISK` and `KS10_MCR_LISTING` (`make microcode`
//! sets both).

mod common;

use std::path::{Path, PathBuf};

use common::{FILLS, ks10_flags, label, machine, symbol};
use ks10_core_uop::alu;
use ks10_machine::boot;

/// The most microcycles without an instruction. The longest
/// instruction of the boot is a BLT that clears memory: about 1.05
/// million microcycles.
const STALL_CYCLES: u64 = 10_000_000;

/// The longest run, in instructions.
const LIMIT: u64 = 40_000_000;

/// The instructions that the model runs before its first stop
/// (docs/microcode-model.md, "Stage 3", "Results"). A change that stops
/// the model earlier fails the test.
const PROGRESS: u64 = 4_731_024;

/// The words that the console halt writes (see above): the halt code,
/// PC, and the halt status block of 16 registers and the VMA (inout,
/// "WRITE HALT STATUS BLOCK").
fn console_halt_word(a: usize) -> bool {
    a <= 1 || (0o500..=0o520).contains(&a)
}

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

    let pi_label = label(&text, "PI");
    s.hold_interrupts = true;
    let mut release = None;
    let mut waiting = 0;
    let mut longest_wait = 0;
    let mut interrupts = 0;
    // The answers at the DSKDMP prompt (tests/its-boot/boot.script): both
    // processors get them at the same instruction.
    let mut script: Vec<(&str, &[u8])> = vec![("DSKDMP", b"its\r"), ("its", b"\x1bg")];
    script.reverse();
    let mut stop = None;
    let mut difference = None;
    let mut last_progress = (0, 0);
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
        if let Some(&(prompt, answer)) = script.last() {
            let out = String::from_utf8_lossy(&m.cty.output).to_lowercase();
            if out.contains(&prompt.to_lowercase()) {
                m.cty.input.extend(answer);
                s.console.input.extend(answer);
                m.cty.output.clear();
                script.pop();
            }
        }
        let before = m.instructions;
        if let Err(e) = m.step() {
            stop = Some(e.to_string());
            break;
        }
        // The model starts an interrupt: ks10-core-soft must accept the
        // same level now; it takes it at its next step.
        if m.upc == pi_label && release.is_none() {
            let level = m.pi_new();
            if s.acceptable_interrupt() != Some(level) {
                difference = Some(format!(
                    "instruction {}: the model takes an interrupt on level {level}, ks10-core-soft accepts {:?}",
                    m.instructions,
                    s.acceptable_interrupt()
                ));
                break;
            }
            release = Some(level);
        }
        if m.instructions == before || m.instructions < 2 {
            continue;
        }
        s.hold_interrupts = release.is_none();
        let r = s.step();
        s.hold_interrupts = true;
        if let Err(e) = r {
            difference = Some(format!("ks10-core-soft stops: {e:?}"));
            break;
        }
        if release.take().is_some() {
            interrupts += 1;
            println!(
                "interrupt {interrupts} at instruction {}, after a wait of {waiting}",
                m.instructions
            );
            waiting = 0;
        } else if s.acceptable_interrupt().is_some() {
            waiting += 1;
            longest_wait = longest_wait.max(waiting);
        }
        let pc = alu::word(m.alu.reg[pc_reg]).get() & 0o777777;
        let block = usize::from(m.cur_block & 7) << 4;
        let acs: Vec<u64> = (0..16).map(|k| m.ram_file[block + k].get()).collect();
        let want: Vec<u64> = s.ac.iter().map(|w| w.get()).collect();
        let (flags, want_flags) = (ks10_flags(m.flags), s.flags.0);
        if pc != s.pc.get() || acs != want || flags != want_flags {
            let octal = |v: &[u64]| {
                v.iter()
                    .map(|w| format!("{w:o}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            difference = Some(format!(
                "instruction {}: model PC {pc:o} flags {flags:o} ACs {}\n  ks10-core-soft PC {:o} flags {want_flags:o} ACs {}",
                m.instructions,
                octal(&acs),
                s.pc.get(),
                octal(&want)
            ));
            break;
        }
    }
    let memory: Vec<String> = (0..m.memory.len())
        .filter(|&a| m.memory[a] != s.mem[a] && !console_halt_word(a))
        .take(10)
        .map(|a| {
            format!(
                "{a:o}: model {:o}, ks10-core-soft {:o}",
                m.memory[a].get(),
                s.mem[a].get()
            )
        })
        .collect();
    let _ = std::fs::remove_file(&copy_m);
    let _ = std::fs::remove_file(&copy_s);
    let stop = stop.unwrap_or_else(|| "none".to_string());
    println!(
        "{} instructions, {} microcycles, stop: {stop}",
        m.instructions, m.cycles
    );
    println!("{interrupts} interrupts, longest wait {longest_wait} instructions");
    println!("CTY: {}", String::from_utf8_lossy(&s.console.output));
    assert_eq!(difference, None);
    assert_eq!(memory, Vec::<String>::new());
    assert!(m.instructions >= PROGRESS);
}

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
use pdp10::W36;

/// The most microcycles without an instruction. The longest
/// instruction of the boot is a BLT that clears memory: about 1.05
/// million microcycles.
const STALL_CYCLES: u64 = 10_000_000;

/// The longest run, in instructions.
const LIMIT: u64 = 40_000_000;

/// The instructions to "IN OPERATION" on the CTY
/// (docs/microcode-model.md, "Stage 5", "Results").
const PROGRESS: u64 = 11_012_654;

/// The words that the console halt writes (see above): the halt code,
/// PC, and the halt status block of 16 registers and the VMA (inout,
/// "WRITE HALT STATUS BLOCK").
fn console_halt_word(a: usize) -> bool {
    a <= 1 || (0o500..=0o520).contains(&a)
}

/// The level of the interrupt that the model starts, if ks10-core-soft
/// accepts the same level now. The devices of ks10-core-soft first run to
/// its instruction count, as the model ran them at its last `SPEC/NICOND`.
fn accept(
    m: &ks10_core_uop::machine::Machine,
    s: &mut ks10_core_soft::Machine,
) -> Result<u8, String> {
    s.run_devices();
    let level = m.pi_new();
    if s.acceptable_interrupt() == Some(level) {
        Ok(level)
    } else {
        Err(format!(
            "instruction {}: the model takes an interrupt on level {level}, ks10-core-soft accepts {:?}",
            m.instructions,
            s.acceptable_interrupt()
        ))
    }
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
    // The words of the PC sample table: ks10-core-soft samples at the
    // instruction boundary of each millisecond, the microcode at the
    // memory cycle where it takes the 1 ms page fail (itspag, PF-INT,
    // .IF/PCST). The PC word depends on that microcycle
    // (docs/microcode-model.md, "The lockstep run and time").
    let mut samples = std::collections::BTreeSet::new();
    let mut synced = 0;
    let pcst_work = symbol(&text, "PCST");
    let (time1_work, quan_work) = (symbol(&text, "TIME1"), symbol(&text, "QUAN"));
    let mut model_time1 = m.ram_file[time1_work];
    let mut quan_copies = 0;
    let mut pfw_copies = 0;
    let mut model_pcst = m.ram_file[pcst_work];
    let mut stop = None;
    let mut difference = None;
    let mut last_progress = (0, 0);
    let mut seen = 0;
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
        if m.cty.output.len() != seen {
            seen = m.cty.output.len();
            let out = String::from_utf8_lossy(&m.cty.output).to_lowercase();
            if let Some(&(prompt, answer)) = script.last() {
                if out.contains(&prompt.to_lowercase()) {
                    m.cty.input.extend(answer);
                    s.console.input.extend(answer);
                    m.cty.output.clear();
                    seen = 0;
                    script.pop();
                }
            } else if out.contains("in operation") {
                in_operation = true;
                break;
            }
        }
        let before = m.instructions;
        if let Err(e) = m.step() {
            stop = Some(e.to_string());
            break;
        }
        // The model starts an interrupt. ks10-core-soft first does the
        // step of an instruction that the model ended in the same word;
        // then it must accept the same level, and takes it at its next
        // step.
        let starts_interrupt = m.upc == pi_label && release.is_none();
        if m.instructions == before || m.instructions < 2 {
            if starts_interrupt {
                match accept(&m, &mut s) {
                    Ok(l) => release = Some(l),
                    Err(e) => {
                        difference = Some(e);
                        break;
                    }
                }
            }
            continue;
        }
        s.hold_interrupts = release.is_none();
        let pcst = s.pcst;
        let r = s.step();
        s.hold_interrupts = true;
        if s.pcst != pcst && pcst.bit(0) {
            samples.insert(pcst.rh().get() as usize);
        }
        // ITS reads the table later: ks10-core-soft gets each sample that
        // the model writes, as it gets the interrupts of the model. The
        // model writes a sample at the right half of WORK[PCST], and then
        // adds 1,,1 to it.
        let p = m.ram_file[pcst_work];
        if p != model_pcst {
            let next = W36::from_halves(
                pdp10::Half::wrap(model_pcst.lh().get() + 1),
                pdp10::Half::wrap(model_pcst.rh().get() + 1),
            );
            if model_pcst.bit(0) && p == next {
                let a = model_pcst.rh().get() as usize;
                s.mem[a] = m.memory[a];
                synced += 1;
            }
            model_pcst = p;
        }
        // Bits 14-17 of a page fail word are the old VMA 14-17: VMA loads
        // them only with EXT ADR, at a physical reference (DPM4; ks10,
        // `VMA PHYSICAL`). The last such reference depends on the page
        // table of the hardware, which ks10-core-soft does not have. A
        // page fail word that differs only there comes from the model.
        for n in 0..8 {
            let a = s.ept_addr(0o440 + 3 * n) as usize;
            let diff = m.memory[a].get() ^ s.mem[a].get();
            if diff != 0 && diff & !(0o17 << 18) == 0 {
                s.mem[a] = m.memory[a];
                pfw_copies += 1;
            }
        }
        // The quantum counter of ITS: TOCK adds a tick only if no PI level
        // is in progress (inout, TOCK), and the microcode does TOCK at the
        // 1 ms page fail, ks10-core-soft at the instruction boundary. After
        // each TOCK of the model (WORK[TIME1] changes), ks10-core-soft gets
        // the counter of the model.
        let t = m.ram_file[time1_work];
        if t != model_time1 {
            model_time1 = t;
            if s.timer.quan != m.ram_file[quan_work] {
                s.timer.quan = m.ram_file[quan_work];
                quan_copies += 1;
            }
        }
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
        if starts_interrupt {
            match accept(&m, &mut s) {
                Ok(l) => release = Some(l),
                Err(e) => {
                    difference = Some(e);
                    break;
                }
            }
        }
    }
    // The model ran the 8080 and the devices at its last SPEC/NICOND;
    // ks10-core-soft does that at the start of its next step.
    s.run_devices();
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
    println!(
        "{} PC samples, {synced} copied from the model; {quan_copies} quantum counts, {pfw_copies} page fail words copied",
        samples.len()
    );
    println!("CTY: {}", String::from_utf8_lossy(&s.console.output));
    assert_eq!(difference, None);
    assert_eq!(memory, Vec::<String>::new());
    assert!(in_operation, "ITS is not in operation");
    assert!(m.instructions >= PROGRESS);
}

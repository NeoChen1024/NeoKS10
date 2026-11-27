//! The console start on the model: the console command ST reads JRST from
//! the console instruction register with an I/O cycle (cmds.m80, STINT,
//! EXINT, COCMD).
//!
//! The expected values come from the 8080 program and from the
//! instructions: JRST to the address, MOVEI, and HALT, which stops with
//! halt code 1 (ks10, `HALT/HALT=1`) and the PC of E (DECsystem-10/20
//! Processor Reference Manual, JRST 4,).
//!
//! A second test runs the ITS I/O instructions on the Unibus adapters and
//! compares the ACs with `ks10-core-soft`.
//!
//! The tests need the listing of the ITS build: `KS10_MCR_LISTING` (`make
//! microcode` sets it). Without it the tests print SKIPPED.

mod common;

use common::{FILLS, label, machine};
use ks10_core_uop::machine::{Machine, Stop};
use ks10_machine::boot::Processor;
use ks10_machine::devregs;
use pdp10::{Half, W36};

/// After ST the microcode is still in the halt loop: it leaves the loop
/// when it sees CONTINUE. Run until it leaves and comes back.
fn run_to_halt(m: &mut Machine, haltlp: u16) -> Result<bool, Stop> {
    Ok(m.run_until(100, |m| !m.csl.halt_loop)?
        && m.run_until(100_000, |m| m.upc == haltlp && m.csl.halt_loop)?)
}

#[test]
fn start_runs_the_program_at_the_address() {
    let Some(text) = common::listing_text("console start") else {
        return;
    };
    let haltlp = label(&text, "HALTLP");
    for fill in FILLS {
        let mut m = machine(&text, fill);
        m.master_reset();
        assert_eq!(m.pending_stop, None);
        // MOVEI 1,123 at 1000 and HALT 1002 (JRST 4,1002) at 1001.
        m.memory[0o1000] = W36::wrap(0o201_040_000_123);
        m.memory[0o1001] = W36::wrap(0o254_200_001_002);
        m.start_at(0o1000);
        assert_eq!(m.pending_stop, None);
        assert!(m.csl.run && !m.csl.execute);
        let r = run_to_halt(&mut m, haltlp);
        println!("fill {:o}: {} cycles, {r:?}", fill.get(), m.cycles);
        assert_eq!(r, Ok(true));
        assert_eq!(m.ram_file[1], W36::wrap(0o123));
        assert_eq!(m.memory[0], W36::wrap(1), "halt code");
        assert_eq!(m.memory[1].get() & 0o777777, 0o1002, "PC");
    }
}

/// A program of ITS I/O instructions at 1000 (ksdefs, IORDI to IOWRQ;
/// inout, "ITS IO INSTRUCTIONS"), and a HALT at its end. AC n + 1 gets the
/// result of step n.
fn io_program() -> Vec<u64> {
    let sta = u64::from(devregs::uba::STA);
    let wc = u64::from(devregs::DISK.base) + 0o2;
    let i = |op: u64, ac: u64, e: u64| op << 27 | ac << 23 | e;
    vec![
        i(0o201, 1, 0o13),        // MOVEI 1,13: PIH 1, PIL 3
        i(0o714, 1, sta),         // IOWRI 1,UBASTA (adapter 3)
        i(0o710, 2, sta),         // IORDI 2,UBASTA
        i(0o201, 3, 0o123456),    // MOVEI 3,123456
        i(0o715, 3, wc),          // IOWRQ 3,WC of the disk RH11 (adapter 1)
        i(0o711, 4, wc),          // IORDQ 4,WC
        i(0o201, 5, 0o201),       // MOVEI 5,201
        i(0o725, 5, wc + 1),      // IOWRBQ 5,WC+1: the high byte
        i(0o711, 6, wc),          // IORDQ 6,WC
        i(0o721, 7, wc + 1),      // IORDBQ 7,WC+1: the high byte
        i(0o721, 8, wc),          // IORDBQ 10,WC: the low byte
        i(0o254, 4, 0o1000 + 12), // HALT
    ]
}

#[test]
fn io_instructions_agree_with_the_software_processor() {
    let Some(text) = common::listing_text("I/O instructions") else {
        return;
    };
    let haltlp = label(&text, "HALTLP");
    let program = io_program();
    let mut s = ks10_core_soft::Machine::new();
    for (k, w) in program.iter().enumerate() {
        s.mem[0o1000 + k] = W36::wrap(*w);
    }
    s.pc = Half::wrap(0o1000);
    let r = (0..100).find_map(|_| s.step().err());
    assert!(
        matches!(r, Some(ks10_core_soft::Stop::Halt { code: 1, .. })),
        "{r:?}"
    );
    for fill in FILLS {
        let mut m = machine(&text, fill);
        m.master_reset();
        for (k, w) in program.iter().enumerate() {
            m.memory[0o1000 + k] = W36::wrap(*w);
        }
        m.start_at(0o1000);
        let r = run_to_halt(&mut m, haltlp);
        assert_eq!(r, Ok(true), "fill {:o}", fill.get());
        let block = usize::from(m.cur_block & 7) << 4;
        for k in 1..=8 {
            assert_eq!(
                m.ram_file[block + k],
                s.ac[k],
                "AC {k:o}, fill {:o}",
                fill.get()
            );
        }
    }
    // The fields of the status register that a program can write
    // (spec/devices.toml), and the word and byte values of WC.
    assert_eq!(s.ac[2].get() & 0o77, 0o13);
    let got: Vec<u64> = [4, 6, 7, 8].iter().map(|&k| s.ac[k].get()).collect();
    assert_eq!(got, [0o123456, 0o201 << 8 | 0o056, 0o201, 0o056]);
}

/// WRUBR ends with SWEEP, which invalidates the whole page table and the
/// cache directory: SET SWEEP and PAGE WRITE with DP 18 0, for two entries
/// at a time (EK-OKS10-TM-002 5.6.3; inout, `WRUBR`, `SWEEP`). The ITS
/// microcode decodes WRUBR as 701 with AC 3 (inout, dispatch 1723).
#[test]
fn wrubr_invalidates_the_page_table_and_the_cache_directory() {
    let Some(text) = common::listing_text("sweep") else {
        return;
    };
    let haltlp = label(&text, "HALTLP");
    for fill in FILLS {
        let mut m = machine(&text, fill);
        m.master_reset();
        // WRUBR 1010 with 0 in 1010 (no AC blocks, no base address), then
        // HALT.
        m.memory[0o1000] = W36::wrap(0o701_140_001_010);
        m.memory[0o1001] = W36::wrap(0o254_200_001_002);
        m.memory[0o1010] = W36::ZERO;
        m.start_at(0o1000);
        assert_eq!(
            run_to_halt(&mut m, haltlp),
            Ok(true),
            "fill {:o}",
            fill.get()
        );
        assert_eq!(m.memory[0], W36::wrap(1), "halt code");
        let pages: Vec<usize> = (0..512)
            .filter(|&k| m.page_table[k].is_none_or(|e| e.valid))
            .collect();
        let tags: Vec<usize> = (0..512)
            .filter(|&k| m.cache_dir[k].is_none_or(|t| t.valid))
            .collect();
        assert_eq!((pages, tags), (vec![], vec![]), "fill {:o}", fill.get());
    }
}

/// An APR interrupt: WRPI turns on the PI system and all levels. WRAPR
/// sets flag 24 without its enable, then sets and enables the interval
/// flag (30) on level 1. The interrupt
/// takes the instruction at 40 + 2 n (inout, `PI10`: no adapter answers
/// the WRU cycle), a JSR. The interrupt program reads the APR and PI
/// words, clears the flag and returns with `JRST 12,@`. The formats are
/// those of KSDEFS (ksdefs, WRAPR, RDAPR, WRPI). The test compares the
/// ACs, the APR and PI words and the halt PC with `ks10-core-soft`.
///
/// The PC word that the JSR stores differs by design
/// (docs/microcode-model.md, "Interrupt timing"). `ks10-core-soft` takes
/// the interrupt before 1003. The model takes it before 1004
/// **[manual]**: WRAPR loads the APR enables two words before `DONE`,
/// DPEB E167 latches the bus request at the next clock, and INTERRUPT REQ
/// at the clock after it (M8620-0-DPEB rev B). `DONE` starts the fetch of
/// 1003 and latches INT in DPM6 E16 one clock too early (ks10, `DONE`;
/// inout, `WRAPR1`). The fetch of 1004 then page-fails.
#[test]
fn an_apr_interrupt_agrees_with_the_software_processor() {
    let Some(text) = common::listing_text("APR interrupt") else {
        return;
    };
    let haltlp = label(&text, "HALTLP");
    let i = |op: u64, ac: u64, e: u64| op << 27 | ac << 23 | e;
    let program: Vec<(usize, u64)> = vec![
        (0o1000, i(0o700, 0o14, 0o2377)),  // WRPI 2377: on, levels on
        (0o1001, i(0o700, 0o4, 0o014001)), // WRAPR: set 24, level 1
        (0o1002, i(0o700, 0o4, 0o110041)), // WRAPR: enable, set 30, level 1
        (0o1003, i(0o201, 2, 1)),          // MOVEI 2,1
        (0o1004, i(0o201, 3, 1)),          // MOVEI 3,1
        (0o1005, i(0o254, 4, 0o1005)),     // HALT
        (0o42, i(0o264, 0, 0o1100)),       // JSR 1100
        (0o1101, i(0o700, 0o5, 0o1200)),   // RDAPR 1200
        (0o1102, i(0o700, 0o4, 0o024041)), // WRAPR: clear 24 and 30, level 1
        (0o1103, i(0o700, 0o15, 0o1201)),  // RDPI 1201
        (0o1104, i(0o254, 0o12, 0o1100) | 1 << 22), // JRST 12,@1100
    ];
    let mut s = ks10_core_soft::Machine::new();
    for &(a, w) in &program {
        s.mem[a] = W36::wrap(w);
    }
    s.pc = Half::wrap(0o1000);
    let r = (0..100).find_map(|_| s.step().err());
    assert!(
        matches!(r, Some(ks10_core_soft::Stop::Halt { code: 1, .. })),
        "{r:?}"
    );
    assert_eq!(s.mem[0o1100].get() & 0o777777, 0o1003, "ks10-core-soft");
    let want = [0o1200, 0o1201].map(|a| s.mem[a].get());
    for fill in FILLS {
        let mut m = machine(&text, fill);
        m.master_reset();
        for &(a, w) in &program {
            m.memory[a] = W36::wrap(w);
        }
        m.start_at(0o1000);
        assert_eq!(
            run_to_halt(&mut m, haltlp),
            Ok(true),
            "fill {:o}",
            fill.get()
        );
        assert_eq!(m.memory[0], W36::wrap(1), "halt code");
        assert_eq!(m.memory[1].get() & 0o777777, 0o1005, "halt PC");
        let got = [0o1200, 0o1201].map(|a| m.memory[a].get());
        let octal = |v: [u64; 2]| v.map(|w| format!("{w:o}")).join(" ");
        assert_eq!(octal(got), octal(want), "1200, 1201, fill {:o}", fill.get());
        assert_eq!(
            m.memory[0o1100].get() & 0o777777,
            0o1004,
            "the interrupted PC"
        );
        // The program writes AC 2 and AC 3 only.
        let block = usize::from(m.cur_block & 7) << 4;
        for k in [2, 3] {
            assert_eq!(m.ram_file[block + k], s.ac[k], "AC {k}");
        }
    }
}

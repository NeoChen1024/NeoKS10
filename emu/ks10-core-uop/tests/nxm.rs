//! A reference to nonexistent memory on the model, with the ITS
//! microcode.
//!
//! The program turns on paging and maps the page 4000-5777 (ITS page 2) to
//! ITS page 1000, the first page above the 512 Ki words of the model. A
//! reference to it is a hard page failure. The expected values come from
//! the microcode and from KSDEFS:
//!
//! - The page fail dispatch gives code 5 (itspag, `PFD`), and `HARD`
//!   makes the page fail word from %PFNXM (4.7) and the user bit, the
//!   physical bit, the I/O bits and the address of the VMA. %PFWRT is for a
//!   soft failure only (ksdefs), so a write gives the same word.
//! - The microcode stores it at EPTPFW = EPT + 440, the PC word at 441,
//!   and takes the new PC from 442 (ksdefs; itspag, `PFTRAP1`). The PC
//!   word has the address of the instruction, which starts again.
//! - NXM ERR sets APR flag 27, RDAPR 1.9 (M8621-0-DPMB rev B, E139;
//!   EK-OKS10-TM-002 5.6.5; ksdefs, RDAPR).

mod common;

use common::{FILLS, label, machine};
use ks10_core_uop::machine::{Machine, Stop};
use ks10_machine::boot::Processor;
use pdp10::W36;

fn run_to_halt(m: &mut Machine, haltlp: u16) -> Result<bool, Stop> {
    Ok(m.run_until(100, |m| !m.csl.halt_loop)?
        && m.run_until(100_000, |m| m.upc == haltlp && m.csl.halt_loop)?)
}

/// The test cases: the instruction at 1003, and the page fail word and
/// the PC (right half) at 441 that the case gives.
const CASES: [(&str, u64, u64, u64); 3] = [
    ("exec read", 0o200_040_004_000, 0o100_000_004_000, 0o1003), // MOVE 1,4000
    ("exec write", 0o202_040_004_000, 0o100_000_004_000, 0o1003), // MOVEM 1,4000
    // JRST 2,@1005: to 1010 in user mode, MOVE 1,4000 there.
    ("user read", 0o254_120_001_005, 0o500_000_004_000, 0o1010),
];

#[test]
fn a_reference_to_nonexistent_memory_page_fails() {
    let haltlp = label("HALTLP");
    let program: [(usize, u64); 13] = [
        (0o1000, 0o702_540_003_000), // LDBR4 3000: exec low table
        (0o1001, 0o702_400_003_100), // LDBR1 3100: user low table
        (0o1002, 0o701_200_020_000), // WREBR 20000: pager on, EPT at 0
        (0o1005, 0o010_000_001_010), // USER flag (bit 5),,1010
        (0o1010, 0o200_040_004_000), // MOVE 1,4000
        // Each table: ITS page 0 to page 0 and ITS page 2 to page 1000,
        // read and write (2.9 and 2.8).
        (0o3000, 0o600_000_000_000),
        (0o3001, 0o601_000_000_000),
        (0o3100, 0o600_000_000_000),
        (0o3101, 0o601_000_000_000),
        (0o442, 0o1100),             // new PC
        (0o1100, 0o700_240_001_200), // RDAPR 1200
        (0o1101, 0o254_200_001_101), // HALT 1101
        (0o1200, 0),
    ];
    for (name, insn, pfw, pc) in CASES {
        for fill in FILLS {
            let at = format!("{name}, fill {:o}", fill.get());
            let mut m = machine(fill);
            m.master_reset();
            for &(a, w) in &program {
                m.memory[a] = W36::wrap(w);
            }
            m.memory[0o1003] = W36::wrap(insn);
            m.start_at(0o1000);
            assert_eq!(run_to_halt(&mut m, haltlp), Ok(true), "{at}");
            assert_eq!(m.memory[1].get() & 0o777777, 0o1101, "halt PC, {at}");
            let got = [m.memory[0o440].get(), m.memory[0o441].get() & 0o777777];
            assert_eq!(
                got.map(|w| format!("{w:o}")),
                [pfw, pc].map(|w| format!("{w:o}")),
                "440, 441, {at}"
            );
            assert_eq!(m.memory[0o1200].get() & 0o400, 0o400, "RDAPR 1.9, {at}");
            assert!(!m.nxm_err, "NXM ERR after ABORT MEM CYCLE, {at}");
        }
    }
}

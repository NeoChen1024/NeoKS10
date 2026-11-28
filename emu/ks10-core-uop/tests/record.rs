//! The record of a completed instruction (`ks10_core_uop::record`).
//!
//! A program of MOVEI, MOVEM, AOS and HALT at 1000 starts with the console
//! command ST. The expected values come from the instructions (PRM 2.1,
//! 2.4) and from the definition of the record: the console JRST, then one
//! record for each instruction before the HALT, with the address of the
//! next instruction in `pc` (simple.42, `INCPC`; ks10, `DONE`): the JRST
//! goes to 1000, each other instruction to the next word. The halt loop gives
//! no record. The hash of a write is FNV-1a over the address and the word,
//! computed here with its own code.

mod common;

use common::{FILLS, label, machine};
use ks10_core_uop::machine::Machine;
use ks10_core_uop::record::{HASH_START, Record};
use ks10_machine::boot::Processor;
use pdp10::W36;

fn fnv(pairs: &[(u64, u64)]) -> u64 {
    let mut h = HASH_START;
    for &(a, w) in pairs {
        for b in a.to_le_bytes().into_iter().chain(w.to_le_bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// The records of a run from ST 1000 to the halt loop.
fn records(m: &mut Machine) -> Vec<Record> {
    let haltlp = label("HALTLP");
    let mut out = Vec::new();
    let mut left_halt_loop = false;
    for _ in 0..100_000 {
        m.step().expect("a step");
        if let Some(r) = m.record.take() {
            out.push(r);
        }
        left_halt_loop |= !m.csl.halt_loop;
        if left_halt_loop && m.upc == haltlp && m.csl.halt_loop {
            return out;
        }
    }
    panic!("no halt");
}

#[test]
fn one_record_for_each_instruction() {
    for fill in FILLS {
        let mut m = machine(fill);
        m.master_reset();
        let program = [
            0o201_040_000_123, // MOVEI 1,123
            0o202_040_002_000, // MOVEM 1,2000
            0o350_000_002_000, // AOS 2000
            0o254_200_001_003, // HALT 1003
        ];
        for (k, w) in program.iter().enumerate() {
            m.memory[0o1000 + k] = W36::wrap(*w);
        }
        m.start_at(0o1000);
        let r = records(&mut m);
        let f = fill.get();
        assert_eq!(r.len(), 4, "fill {f:o}: the JRST and 3 instructions");
        let counts: Vec<u64> = r.iter().map(|x| x.count).collect();
        assert_eq!(counts, [1, 2, 3, 4], "fill {f:o}");
        assert!(r.windows(2).all(|w| w[0].cycles < w[1].cycles));
        let pcs: Vec<u64> = r.iter().map(|x| x.pc.get() & 0o777777).collect();
        assert_eq!(pcs, [0o1000, 0o1001, 0o1002, 0o1003], "fill {f:o}");
        // ST is EXECUTE of the JRST, then CONTINUE (cmds.m80, STCMD): after
        // the JRST the microcode halts again and writes locations 0 and 1
        // and the halt status block 500 to 520 (inout, DUMP; simple, "ITS
        // makes it 500"). These 19 writes come before MOVEI, which writes
        // AC 1 only. MOVEM: one write of 123 at 2000. AOS: one write of 124
        // at 2000, and AC 0 is not written (A = 0).
        assert_eq!(r[1].ac[1].get(), 0o123);
        assert_eq!(r[1].writes, 2 + 0o21, "fill {f:o}");
        assert_eq!((r[2].writes, r[2].write_hash), (1, fnv(&[(0o2000, 0o123)])));
        assert_eq!((r[3].writes, r[3].write_hash), (1, fnv(&[(0o2000, 0o124)])));
        assert_eq!(r[3].ac[0], r[2].ac[0], "fill {f:o}");
        // Executive mode, no flag set.
        assert!(r.iter().all(|x| x.flags.get() == 0), "fill {f:o}");
    }
}

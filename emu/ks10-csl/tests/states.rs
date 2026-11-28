//! The number of states of each 8080 instruction in `x80-core`, against
//! the "Summary of Processor Instructions" of Intel 98-153B (September
//! 1975), p. 4-15 **[manual]**. The delay loops of the console program and
//! the time of the board depend on these numbers.
//!
//! The table below is a copy of the encodings and the "Clock Cycles"
//! column of that page. Note 2 of the page: "Two possible cycle times,
//! (5/11) indicate instruction cycles dependent on condition flags".

use x80_core::i8080::{I8080, Inputs};
use x80_core::{Bus, Control, Exit, MemoryAccess};

/// The states of `op`: (condition false, condition true), the same number
/// twice if the instruction has no condition. `None` for an opcode that is
/// not in the table.
fn states(op: u8) -> Option<(u64, u64)> {
    let one = |n| Some((n, n));
    let (hi, mid, lo) = (op >> 6, (op >> 3) & 7, op & 7);
    let rp_bit = mid & 1;
    match (hi, mid, lo) {
        // MOV, HLT.
        (1, 6, 6) => one(7),
        (1, 6, _) | (1, _, 6) => one(7),
        (1, _, _) => one(5),
        // ADD ADC SUB SBB ANA XRA ORA CMP.
        (2, _, 6) => one(7),
        (2, _, _) => one(4),
        (0, 6, 6) => one(10), // MVI M
        (0, _, 6) => one(7),  // MVI r
        (0, 6, 4 | 5) => one(10),
        (0, _, 4 | 5) => one(5),             // INR r, DCR r
        (0, _, 7) => one(4),                 // RLC RRC RAL RAR DAA CMA STC CMC
        (0, 0, 0) => one(4),                 // NOP
        (0, _, 1) if rp_bit == 0 => one(10), // LXI
        (0, _, 1) => one(10),                // DAD
        (0, _, 3) => one(5),                 // INX, DCX
        (0, 0..=3, 2) => one(7),             // STAX, LDAX
        (0, 4 | 5, 2) => one(16),            // SHLD, LHLD
        (0, 6 | 7, 2) => one(13),            // STA, LDA
        (3, _, 6) => one(7),                 // ADI ... CPI
        (3, _, 0) => Some((5, 11)),          // Rcc
        (3, _, 2) => one(10),                // Jcc
        (3, _, 4) => Some((11, 17)),         // Ccc
        (3, _, 7) => one(11),                // RST
        (3, _, 5) if rp_bit == 0 => one(11), // PUSH
        (3, 1, 5) => one(17),                // CALL
        (3, _, 1) if rp_bit == 0 => one(10), // POP
        (3, 1, 1) => one(10),                // RET
        (3, 5, 1) | (3, 7, 1) => one(5),     // PCHL, SPHL
        (3, 0, 3) => one(10),                // JMP
        (3, 2 | 3, 3) => one(10),            // OUT, IN
        (3, 4, 3) => one(18),                // XTHL
        (3, 5, 3) => one(4),                 // XCHG
        (3, 6 | 7, 3) => one(4),             // DI, EI
        _ => None,
    }
}

/// 64 KiB of memory; ports read 0.
struct Ram(Vec<u8>);

impl Bus for Ram {
    fn read(&mut self, address: u16, _kind: MemoryAccess) -> u8 {
        self.0[usize::from(address)]
    }
    fn write(&mut self, address: u16, value: u8, _kind: MemoryAccess) {
        self.0[usize::from(address)] = value;
    }
    fn input(&mut self, _port: u8) -> u8 {
        0
    }
    fn output(&mut self, _port: u8, _value: u8) {}
}
impl Control for Ram {}
impl Inputs for Ram {}

/// One step of `op` at address 100 (octal), with its operands 0, the stack
/// at 40000 and all condition flags `flags`.
fn step(op: u8, flags: bool) -> (u64, Exit) {
    let mut ram = Ram(vec![0; 0x10000]);
    ram.0[0o100] = op;
    let mut cpu = I8080::default();
    cpu.reset();
    let r = cpu.registers_mut();
    r.pc = 0o100;
    r.sp = 0o40000;
    r.flags.sign = flags;
    r.flags.zero = flags;
    r.flags.parity = flags;
    r.flags.carry = flags;
    r.flags.auxiliary_carry = flags;
    let report = cpu.step(&mut ram);
    (report.cycles, report.exit)
}

/// The condition CCC of bits 5 to 3 is true with the flags `flags`
/// (p. 4-11: NZ, Z, NC, C, PO, PE, P, M).
fn condition(op: u8, flags: bool) -> bool {
    let ccc = (op >> 3) & 7;
    // The odd conditions test for a set flag.
    (ccc & 1 == 1) == flags
}

#[test]
fn the_states_of_each_opcode_agree_with_the_manual() {
    let mut documented = 0;
    for op in 0..=255u8 {
        let Some((no, yes)) = states(op) else {
            // An opcode outside the table stops x80-core.
            let (cycles, exit) = step(op, false);
            assert!(
                matches!(exit, Exit::Unsupported(_)),
                "opcode {op:03o}: {exit:?} after {cycles} states"
            );
            continue;
        };
        documented += 1;
        for flags in [false, true] {
            let want = if no != yes && condition(op, flags) {
                yes
            } else {
                no
            };
            let (cycles, exit) = step(op, flags);
            assert_eq!(exit, Exit::Budget, "opcode {op:03o}");
            assert_eq!(cycles, want, "opcode {op:03o}, flags {flags}");
        }
    }
    // 98-153B lists 244 opcodes on p. 4-15 when each register field is
    // counted; the other 12 have no documentation.
    assert_eq!(documented, 244);
}

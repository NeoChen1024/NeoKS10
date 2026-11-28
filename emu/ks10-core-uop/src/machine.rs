//! The machine: the microsequencer and the data path around the
//! arithmetic unit (docs/microcode-model.md, "The data path" and "Stage
//! 1").
//!
//! A function has one of two states: modelled, or a stop with
//! [`Stop::Unmodelled`]. The model never guesses a value.

use ks10_machine::INSTRUCTIONS_PER_MS;
use ks10_machine::boot;
use ks10_machine::console::Console as Cty;
use ks10_machine::unibus::{Io, Reply, level_bit};
use pdp10::W36;

use crate::alu::{self, Alu, Controls, Outputs, W40};
use crate::cram::Microword;
use crate::drom::DROM_WORDS;
use crate::fields::{Field, u};
use crate::record::{self, Record};

/// The number of words of the RAM file (DPE7, DPE8).
pub const RAM_FILE_WORDS: usize = 1024;

pub use ks10_machine::MEMORY_WORDS;

/// The microcycles from the command/address of an I/O cycle to the end of
/// I/O BUSY of a Unibus adapter, for a read (then I/O DATA CYCLE) and for
/// a write **[estimate]**: the emulated devices complete at once, and the
/// microcode waits for any time up to its limits (inout, IOWAIT).
const UBA_BUSY_CYCLES: u32 = 4;

/// The microcycles from the command/address of a console read to its data
/// cycle: "two cycles after the command/address" **[manual]**
/// (EK-OKS10-TM-002 5.3.6), taken as microcycles.
const CONSOLE_DATA_CYCLES: u32 = 2;

/// The register address bit that the console decodes for a read of its
/// instruction register: R DATA 19 (M8616-0-CSL4 rev C, E90, E106).
const CONSOLE_REGISTER_BIT: u32 = 0o200000;

/// The microcycles that `master_reset` and `start_at` wait for the
/// microcode, as the delay loops of the 8080 do (cmds.m80, HLTCM, DNF).
const CONSOLE_WAIT_CYCLES: u64 = 100_000;

/// Why the machine stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The microinstruction at `addr` needs a function that the model does
    /// not have yet.
    Unmodelled { addr: u16, what: String },
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stop::Unmodelled { addr, what } => write!(f, "not modelled at {addr:04o}: {what}"),
        }
    }
}

/// The register E603 of DPE5: values of the previous cycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags603 {
    pub func_01: bool,
    pub carry_02: Option<bool>,
    pub carry_out: Option<bool>,
    pub fl_02: Option<bool>,
    pub qr_37: Option<bool>,
}

/// The program flags (M8620-0-DPE9 rev A1, E209, E210, E305).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PcFlags {
    pub ov: bool,
    pub cry0: bool,
    pub cry1: bool,
    pub fov: bool,
    pub fpd: bool,
    pub user: bool,
    /// PCU in executive mode, USER IOT in user mode: one flip-flop (E210 R1).
    pub pcu: bool,
    pub trap2: bool,
    pub trap1: bool,
    pub fxu: bool,
    pub nodiv: bool,
}

/// The bits of byte 1 to 5 for the byte insertion of DBM (M8621-0-DPM1
/// rev B, E72, E86: SEL 2 of 00–06, 07–13, 14–20, 21–27, 28–35).
const BYTE_BITS: [(u32, u32); 5] = [(0, 6), (7, 13), (14, 20), (21, 27), (28, 35)];

/// DBM input 1, `BYTES` **[manual]** (M8621-0-DPM1 and DPM2 rev B, input
/// 1D of each 74LS151): bit b is SCAD 01–07 in turn (SCAD 1 + b mod 7)
/// for bits 00–34. Bit 35 is DP 35 (DPM2 E188: 1D is on the line of 3D);
/// this function gives 0 there, and the callers add DP 35. `scad` has
/// SCAD 00 at the left.
fn bytes(scad: u16) -> W36 {
    let bit = |k: u32| u64::from((scad >> (9 - k)) & 1);
    W36::wrap((0..35).fold(0, |v, b| v | bit(1 + b % 7) << (35 - b)))
}

/// BYTE DISP 09–11 (bits 09–11 of the address) for the byte pointer on
/// DP **[manual]** (M8620-0-DPE3 rev B, E77, E47, E18, E10, E9). E77 and
/// E47 enable the decoder E18 if S (DP 06–11) is 7. E18 decodes DP 00–02,
/// and the mixer E10 by DP 03–05 picks P = 1, 8, 15, 22 or 29: the
/// positions of the five 7-bit bytes. For such a pointer, E9 gives NOT
/// DP 01, NOT DP 02 and NOT DP 04; else 0. The codes agree with the
/// dispatch of the microcode **[microcode]** (simple, `LDB1`: 001 byte 1,
/// 010 byte 2, 100 byte 3, 101 byte 4, 111 byte 5).
fn byte_disp(dp: W36) -> u16 {
    let (p, size) = (dp.field(0, 5), dp.field(6, 11));
    if size != 7 || ![1, 8, 15, 22, 29].contains(&p) {
        return 0;
    }
    u16::from(!dp.bit(1)) << 2 | u16::from(!dp.bit(2)) << 1 | u16::from(!dp.bit(4))
}

/// The VMA flags (M8621-0-DPM4 rev B, E82, E90, E189).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VmaFlags {
    pub user: bool,
    pub fetch: bool,
    pub physical: bool,
    pub previous: bool,
    pub io: bool,
    pub wru: bool,
    pub vector: bool,
    pub io_byte: bool,
    /// VMA EXTENDED: VMA 14–17 go to the bus.
    pub extended: bool,
}

/// The type of the memory cycle (M8621-0-DPM5 rev B, E33).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cycle {
    pub read: bool,
    pub write_test: bool,
    pub write: bool,
    pub cache_inhibit: bool,
}

/// The PXCT register (M8621-0-DPMA rev B, E71).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pxct {
    pub on: bool,
    /// B09 to B12 in bits 3 to 0.
    pub bits: u8,
}

/// An entry of the page table **[manual]** (M8621-0-DPM6 rev C, E130 to
/// E192). The model keeps no parity bits: a written entry has correct
/// parity. An entry that nothing wrote is `None` (docs/unknowns.md, "The
/// microcode-level model").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageEntry {
    pub valid: bool,
    pub writeable: bool,
    pub cacheable: bool,
    pub user: bool,
    /// Physical page bits 16–26.
    pub page: u16,
}

/// An entry of the cache directory **[manual]** (M8621-0-DPM7 rev B, E148
/// to E193): VALID, USER and the virtual page VMA 18–26 of the word in the
/// RAM file at 1000 + VMA 27–35.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheTag {
    pub valid: bool,
    pub user: bool,
    pub page: u16,
}

/// The console board: its inputs to the processor and the HALT LOOP flag
/// (M8616-0-CSL4 rev C). The 8080 sets RUN, CONTINUE and EXECUTE through
/// register 212, TRAP EN through register 205 and 1 MSEC EN and CACHE EN
/// through register 100 (hcore.m80, condef.m80).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Csl {
    pub run: bool,
    pub cont: bool,
    pub execute: bool,
    pub trap_en: bool,
    pub one_ms_en: bool,
    pub cache_en: bool,
    /// HALT LOOP (E161): the microcode sets and clears it.
    pub halt_loop: bool,
    /// The console instruction register, I/O address 0,,200000: the word
    /// that the 8080 writes to registers 102 to 112 (cmds.m80, EXINT,
    /// WDATP).
    pub data: W36,
    /// The console answers a read with I/O DATA CYCLE, not DATA CYCLE:
    /// register 114 (DTARB) bit 1 (cmds.m80, EXINT: "SET BIT FOR "I/O DATA
    /// CYCLE"").
    pub io_data_cycle: bool,
}

/// The state of the KS10 bus for the I/O cycles of the processor, in
/// microcycles (docs/microcode-model.md, "I/O cycles").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bus {
    /// I/O LATCH (CRA2 E148 R2) and the copy that `SKIP/-IO BUSY` reads
    /// (CRA2 E98 R1).
    pub io_latch: bool,
    pub io_latch_skip: bool,
    /// The microcycles in which a controller still asserts I/O BUSY.
    pub io_busy: u32,
    /// The microcycles until the data cycle of a read, and whether it is
    /// an I/O DATA CYCLE.
    pub data_in: Option<(u32, bool)>,
    /// The word of the read in progress.
    pub reply: Option<W36>,
}

/// The microcode-level machine.
pub struct Machine {
    pub cram: Vec<Microword>,
    pub drom: Vec<u64>,
    pub alu: Alu,
    pub e603: Flags603,
    pub ram_file: Vec<W36>,
    /// The address of the current microinstruction.
    pub upc: u16,
    /// The subroutine stack (CRA3): 16 places and a 4-bit counter.
    pub stack: [u16; 16],
    pub sp: u8,
    /// The step counter and the floating exponent register (DPM4), 10 bits.
    pub sc: u16,
    pub fe: u16,
    /// IR: opcode and AC (DPEA E307, E309).
    pub ir: u16,
    pub ac: u8,
    /// XR and the indirect bit (DPEA E311).
    pub xr: u8,
    pub indirect: bool,
    /// The current and previous AC block (DPE5 E308).
    pub cur_block: u8,
    pub prev_block: u8,
    /// XR PREV (DPEA E311).
    pub xr_prev: bool,
    /// VMA (DPM4), as the DP word that loaded it: bits 14–35 are the
    /// register.
    pub vma: W36,
    pub vma_flags: VmaFlags,
    /// VMA SWEEP **[manual]** (DPM4 E189 R1): SET SWEEP of the word that
    /// loaded VMA.
    pub vma_sweep: bool,
    /// The page table by VMA 18–26 and the cache directory by VMA 27–35;
    /// `None` for an entry that nothing has written since power on.
    pub page_table: Vec<Option<PageEntry>>,
    pub cache_dir: Vec<Option<CacheTag>>,
    /// PAGE WRITE EN **[manual]** (DPMC E3 R0): `SPEC/PAGE WRITE` of the
    /// last word.
    pub page_write_en: bool,
    /// The DPE copy of VMA bits 26–35 (DPE5 E318, E324).
    pub vma_ram: u16,
    /// The cycle type (DPM5 E33) and MEMORY CYCLE (DPM6 E16 R0).
    pub cycle: Cycle,
    pub memory_cycle: bool,
    /// INT and 1MS as DPM6 E16 R3 and R2 latched them at MEM WAIT.
    pub int_latched: bool,
    pub one_ms_latched: bool,
    /// The memory buffer (DPM8, DPM9 receive latches).
    pub mb: W36,
    /// Main memory, by physical address.
    pub memory: Vec<W36>,
    pub pxct: Pxct,
    /// The DP word of the last `SPEC/APR EN` (DPMB E123, E132; DPEB E173).
    pub apr_enables: W36,
    /// DP of this word with D all 0 and with D all 1.
    pub dp_bounds: Option<(W36, W36)>,
    /// The DP bits of this word that a byte loop of DBM leaves undefined
    /// ([`Machine::dbm`]).
    pub byte_loop: Option<u64>,
    /// The 2901 state after a byte loop with the undefined bits 1, for the
    /// check in the next word.
    pub byte_loop_alu: Option<Alu>,
    /// The APR flags 24–31 **[manual]** (M8621-0-DPMB rev B, E159, E158,
    /// E139, E147): bit 7 is flag 24, bit 0 is flag 31.
    pub apr_flags: u8,
    /// The DP word of the last `SPEC/LDPI` (DPEB E175, E126, E140). The
    /// registers hold the complement of DP.
    pub pi: W36,
    /// INTERRUPT REQ (DPEB E167 R7).
    pub interrupt_req: bool,
    /// The bus PI requests that DPEB E167 R0–R6 latched at the last clock,
    /// as a mask of levels (`unibus::level_bit`).
    pub bus_requests: u8,
    /// The 1 ms flag (DPMC E1) and its copy (DPMC E3 R3).
    pub one_ms: bool,
    pub one_ms_sync: bool,
    pub csl: Csl,
    pub bus: Bus,
    /// The Unibus adapters and their devices.
    pub io: Io,
    /// The completed instructions: the `SPEC/NICOND` words. This is the
    /// time of the devices, as in `ks10-core-soft` (docs/microcode-model.md,
    /// "Device time").
    pub instructions: u64,
    /// The instruction count of the last 1 ms tick.
    pub last_tick: u64,
    /// The CTY service of the 8080 (the communication words 32 and 33).
    pub cty: Cty,
    /// A stop of `master_reset` or `start_at`, which the next `step`
    /// returns.
    pub pending_stop: Option<Stop>,
    /// TRAP CYCLE (CRA2 E159).
    pub trap_cycle: bool,
    pub flags: PcFlags,
    pub cycles: u64,
    /// Memory reads that the cache answered (CACHE HIT in the word that
    /// completes a read).
    pub cache_hits: u64,
    pub ticks: u64,
    /// The record of the last completed instruction (`record`). `step`
    /// sets it at the end of a `NICOND` word; a caller can take it.
    pub record: Option<Record>,
    /// A `NICOND` word is in progress: `step` takes the record at its end.
    record_due: bool,
    /// The 2901 register PC (ks10, `A/PC`) **[microcode]**.
    pc_register: usize,
    /// The memory writes since the last record, and their hash.
    writes: u64,
    write_hash: u64,
}

fn get(mw: Microword, f: Field) -> u32 {
    mw.get(f)
}

impl Machine {
    /// A machine with the given control RAM and dispatch ROM. `fill` is
    /// the content at power on of the state that no reset defines: the
    /// 2901 registers, the RAM file and main memory get `fill`, and the
    /// other registers and flags get 0 if `fill` is 0 and all ones if not
    /// (docs/microcode-model.md, "Stage 1"). A correct run does not depend
    /// on `fill`, except where the hardware does too: the halt code that
    /// power-up writes to location 0 (docs/microcode-model.md, "Stage
    /// 1").
    pub fn new(cram: Vec<Microword>, drom: Vec<u64>, fill: W40) -> Self {
        assert_eq!(cram.len(), crate::cram::CRAM_WORDS);
        assert_eq!(drom.len(), DROM_WORDS);
        let p = fill != W40::ZERO;
        let pw = if p { W36::ONES } else { W36::ZERO };
        let all = |v: u8| if p { v } else { 0 };
        let flags = if p {
            PcFlags {
                ov: true,
                cry0: true,
                cry1: true,
                fov: true,
                fpd: true,
                user: true,
                pcu: true,
                trap2: true,
                trap1: true,
                fxu: true,
                nodiv: true,
            }
        } else {
            PcFlags::default()
        };
        let vma_flags = VmaFlags {
            user: p,
            fetch: p,
            physical: p,
            previous: p,
            io: p,
            wru: p,
            vector: p,
            io_byte: p,
            extended: p,
        };
        Machine {
            cram,
            drom,
            alu: Alu::new(fill),
            e603: Flags603::default(),
            ram_file: vec![alu::word(fill); RAM_FILE_WORDS],
            upc: 0,
            stack: [0; 16],
            sp: 0,
            sc: if p { 0o1777 } else { 0 },
            fe: if p { 0o1777 } else { 0 },
            ir: if p { 0o777 } else { 0 },
            ac: all(15),
            xr: all(15),
            indirect: p,
            cur_block: all(7),
            prev_block: all(7),
            xr_prev: p,
            vma: pw,
            vma_flags,
            vma_sweep: p,
            page_table: vec![None; 512],
            cache_dir: vec![None; 512],
            page_write_en: p,
            vma_ram: if p { 0o1777 } else { 0 },
            cycle: Cycle {
                read: p,
                write_test: p,
                write: p,
                cache_inhibit: p,
            },
            memory_cycle: p,
            int_latched: p,
            one_ms_latched: p,
            mb: pw,
            memory: vec![alu::word(fill); MEMORY_WORDS],
            pxct: Pxct {
                on: p,
                bits: all(15),
            },
            apr_enables: pw,
            apr_flags: all(0o377),
            byte_loop: None,
            dp_bounds: None,
            byte_loop_alu: None,
            pi: pw,
            interrupt_req: p,
            bus_requests: all(0o177),
            one_ms: p,
            one_ms_sync: p,
            csl: Csl::default(),
            bus: Bus::default(),
            io: Io::default(),
            instructions: 0,
            last_tick: 0,
            cty: Cty::default(),
            pending_stop: None,
            trap_cycle: p,
            flags,
            cycles: 0,
            cache_hits: 0,
            ticks: 0,
            record: None,
            record_due: false,
            pc_register: crate::ucode::symbol("A", "PC").expect("the symbol A/PC") as usize,
            writes: 0,
            write_hash: record::HASH_START,
        }
    }

    /// The physical address of a bus cycle (DPM8 E111, E119, E127; DPM9
    /// E153): VMA 18–35, and VMA 14–17 if VMA EXTENDED is 1.
    pub fn bus_address(&self) -> usize {
        let high = if self.vma_flags.extended {
            self.vma.field(14, 17) as usize
        } else {
            0
        };
        (high << 18) | self.vma.field(18, 35) as usize
    }

    /// VMA AC REF (DPM4 E160, E191, E168).
    pub fn ac_ref(&self) -> bool {
        !self.vma_flags.physical && self.vma.field(18, 31) == 0
    }

    /// PREVIOUS EN (DPMA E62, E63): the PXCT bit that the `PXCT` field
    /// selects, if PXCT ON.
    fn previous_en(&self, mw: Microword) -> bool {
        let sel = get(mw, u::PXCT);
        let b = |k: u8| self.pxct.bits >> (3 - k) & 1 != 0;
        let enabled = self.pxct.on && (sel & 1 != 0 || b(0));
        enabled
            && match sel {
                0 => false,
                1 => b(0),
                2 | 3 => b(1),
                4 | 5 => b(2),
                _ => b(3),
            }
    }

    /// INTERRUPT EN, which DPEB E167 latches as INTERRUPT REQ (DPEB E147,
    /// E134, E148): PI ON, and the highest request is above the highest
    /// level in progress. A level requests with a program request, or with
    /// a latched bus request if the level is active (E168, E154, E161).
    fn interrupt_request(&self) -> bool {
        let on = !self.pi.bit(28);
        let current = (1..=7u8).find(|&n| !self.pi.bit(20 + u32::from(n)));
        on && self
            .pi_request()
            .is_some_and(|r| current.is_none_or(|c| r < c))
    }

    /// The highest level that requests (E168, E154, E161, E147).
    fn pi_request(&self) -> Option<u8> {
        let dp = self.pi;
        let requested = |n: u8| {
            let k = u32::from(n);
            !dp.bit(10 + k) || (self.bus_requests & level_bit(n) != 0 && !dp.bit(28 + k))
        };
        (1..=7).find(|&n| requested(n))
    }

    /// PI NEW (DPEB E147, a 74148 with active low outputs and PI ON at its
    /// enable): the highest level that requests, else 7 **[manual]**. The
    /// drawing names its inputs from D0 (the highest priority) to D7;
    /// level n is at Dn. The microcode dispatches on it to the level
    /// (inout, `PI`: 1001 to 1111 for levels 1 to 7).
    pub fn pi_new(&self) -> u8 {
        if self.pi.bit(28) {
            return 7;
        }
        self.pi_request().unwrap_or(7)
    }

    /// APR INT REQ **[manual]** (M8621-0-DPMB rev B, E131, E140, E135,
    /// E97): an APR flag with its enable, or DP 32 of the last `SPEC/APR
    /// EN` (E123 R2).
    fn apr_int_req(&self) -> bool {
        let en = self.apr_enables.field(24, 31) as u8;
        self.apr_flags & en != 0 || self.apr_enables.bit(32)
    }

    /// The BUS PI REQ lines: the adapters, and APR INT REQ on the level of
    /// DP 33–35 of the last `SPEC/APR EN` **[manual]** (M8620-0-DPEB rev
    /// B, E173, E166). Level 0 selects an output that drives no line.
    fn bus_request_lines(&self) -> u8 {
        let level = self.apr_enables.field(33, 35) as u8;
        let apr = if level != 0 && self.apr_int_req() {
            level_bit(level)
        } else {
            0
        };
        self.io.requests() | apr
    }

    fn unmodelled(&self, what: impl Into<String>) -> Stop {
        Stop::Unmodelled {
            addr: self.upc,
            what: what.into(),
        }
    }

    /// The RAM file address (M8620-0-DPE6 rev B) for a `RAMADR` code.
    /// Bits 00–02 are 0 unless `RAMADR` bit 4 is 1 (E7, E16, E8). Bits
    /// 03–05 are a block or VMA 29–31 (E3, E14, E15, E13), and bits 06–09
    /// come from E22, E23, E24, E21.
    fn ram_address(&mut self, mw: Microword, dp: Option<W40>) -> Result<usize, Stop> {
        let block = |b: u8| usize::from(b & 7) << 4;
        let vma_ram = usize::from(self.vma_ram & 0o1777);
        Ok(match get(mw, u::RAMADR) {
            0 => block(self.cur_block) | usize::from(self.ac & 15),
            1 => {
                // A 74LS181 on DPE6 combines AC with ACN; ACALU selects
                // the function (ks10, "# REDEFINED WHEN USED AS AC
                // CONTROL"). Only the two named functions are modelled.
                let n4 = get(mw, u::ACN) as u8;
                let v = match get(mw, u::ACALU) {
                    0o25 => n4,
                    0o62 => self.ac.wrapping_add(n4),
                    other => return Err(self.unmodelled(format!("ACALU {other:o}"))),
                };
                block(self.cur_block) | usize::from(v & 15)
            }
            // XR: the previous block if XR PREV.
            2 => {
                let b = if self.xr_prev {
                    self.prev_block
                } else {
                    self.cur_block
                };
                block(b) | usize::from(self.xr & 15)
            }
            // VMA: an AC in the current or previous block, or the cache
            // at 1000 plus VMA 27–35.
            4 => {
                if self.ac_ref() {
                    let b = if self.vma_flags.previous {
                        self.prev_block
                    } else {
                        self.cur_block
                    };
                    block(b) | (vma_ram & 15)
                } else {
                    0o1000 | (vma_ram & 0o777)
                }
            }
            6 => vma_ram,
            // #: the number field through DBM, bits 26–28 and 11–17.
            7 => {
                let d = self.dbm(mw, dp)?;
                ((d.field(26, 28) as usize) << 7) | d.field(11, 17) as usize
            }
            other => return Err(self.unmodelled(format!("RAMADR {other}"))),
        })
    }

    /// The SCAD output (M8621-0-DPM3 rev B) for the number field of `mw`
    /// and DP of this cycle. SCAD has 10 bits, 00 at the left.
    fn scad(&self, mw: Microword, dp: Option<W36>) -> Result<u16, Stop> {
        let num = get(mw, u::NUM);
        let nb = |k: u32| u16::from((num >> (17 - k)) & 1 != 0);
        // Bit k of SCAD as a value (bit 09 is 1).
        let at = |k: u32, v: u16| v << (9 - k);
        let sel_a = get(mw, u::SCADA);
        let sel_b = get(mw, u::SCADB);
        let dp = match dp {
            Some(w) => w,
            None if sel_a < 2 && sel_b == 0 => W36::ZERO,
            None => return Err(self.unmodelled("SCAD needs DP, which depends on the DBus")),
        };
        let dpb = |k: u32| u16::from(dp.bit(k));
        // SCADA bits 00, 08 and 09: SC if # 05 is 0, else # 08, 16 and 17
        // (E28).
        let ends = if nb(5) == 0 {
            self.sc & 0o1003
        } else {
            at(0, nb(8)) | at(8, nb(16)) | at(9, nb(17))
        };
        // SCADA bits 01–07 (E75, E51, E91, E67, E83, E99, E107).
        let a = match sel_a {
            0 => self.sc,
            1 => get(mw, u::S_NUM) as u16,
            // PTR44: 1, 0, 0, 1, 0, 0, DP 06.
            2 => ends | at(1, 1) | at(4, 1) | at(7, dpb(6)),
            // BYTE1 to BYTE5: seven DP bits from DP 00, 07, 14, 21, 28.
            n => {
                let first = 7 * (n - 3);
                (1..=7).fold(ends, |v, k| v | at(k, dpb(first + k - 1)))
            }
        };
        // SCADB (E36, E60, E76, E84, E92).
        let b = match sel_b {
            0 => self.fe,
            // EXP: DP 01–08 XOR DP 00 in bits 02–09 (E68, E100, E41).
            1 => (2..=9).fold(0, |v, k| v | at(k, dpb(k - 1) ^ dpb(0))),
            // SHIFT: DP 18 in bits 00 and 01, DP 28–35 in bits 02–09.
            2 => (2..=9).fold(at(0, dpb(18)) | at(1, dpb(18)), |v, k| {
                v | at(k, dpb(k + 26))
            }),
            // SIZE: DP 06–11 in bits 01–06.
            _ => (1..=6).fold(0, |v, k| v | at(k, dpb(k + 5))),
        };
        let m = 0o1777;
        Ok(match get(mw, u::SCAD) {
            0 => a.wrapping_mul(2),
            1 => a | b,
            2 => a.wrapping_sub(b).wrapping_sub(1),
            3 => a.wrapping_sub(b),
            4 => a.wrapping_add(b),
            5 => a & b,
            6 => a.wrapping_sub(1),
            _ => a,
        } & m)
    }

    /// Run one microinstruction.
    ///
    /// `MULTI SHIFT`: while FE SIGN is 1, the CRA clock stops (CSL5 E52,
    /// E33), and the word runs again. When FE SIGN is 0, the CRA clock runs
    /// and the DPE/M clock is off (CSL5 E52, E45, E54): the word does
    /// nothing in the data path **[manual]** (M8616-0-CSL5 rev D;
    /// EK-OKS10-TM-002 5.4.1, bit 79). While FE SIGN is 1, the DPE/M clock
    /// also needs CSL5 FS (1) (E53), which this model takes as 1
    /// (docs/unknowns.md).
    pub fn step(&mut self) -> Result<(), Stop> {
        let r = self.step_cycle();
        if self.record_due {
            self.record_due = false;
            self.take_record();
        }
        r
    }

    /// The record at the end of a `NICOND` word (`record`).
    fn take_record(&mut self) {
        let base = usize::from(self.cur_block & 7) << 4;
        self.record = Some(Record {
            count: self.instructions,
            cycles: self.cycles,
            pc: alu::word(self.alu.reg[self.pc_register]),
            flags: record::flag_half(self.flags),
            ac: std::array::from_fn(|k| self.ram_file[base + k]),
            writes: self.writes,
            write_hash: self.write_hash,
        });
        self.writes = 0;
        self.write_hash = record::HASH_START;
    }

    fn step_cycle(&mut self) -> Result<(), Stop> {
        if let Some(s) = self.pending_stop.take() {
            return Err(s);
        }
        let mw = self.cram[usize::from(self.upc & 0o3777)];
        if get(mw, u::MULTI_SHIFT) == 0 {
            return self.step_word(mw);
        }
        // The data path state that such a word can change.
        if get(mw, u::MEM) != 0 || get(mw, u::FMWRITE) != 0 || get(mw, u::SPEC) >> 3 != 0 {
            return Err(self.unmodelled("MULTI SHIFT with a memory or special function"));
        }
        if self.fe & 0o1000 != 0 {
            let (upc, sp, stack) = (self.upc, self.sp, self.stack);
            self.step_word(mw)?;
            (self.upc, self.sp, self.stack) = (upc, sp, stack);
        } else {
            // The DPE/M clock is off (CSL5 E52, E45, E54). The check above
            // leaves these registers as the only data path state that the
            // word can change.
            let (alu, e603, sc, fe, int, bus) = (
                self.alu.clone(),
                self.e603,
                self.sc,
                self.fe,
                self.interrupt_req,
                self.bus_requests,
            );
            self.step_word(mw)?;
            (self.alu, self.e603, self.sc, self.fe) = (alu, e603, sc, fe);
            (self.interrupt_req, self.bus_requests) = (int, bus);
        }
        Ok(())
    }

    fn step_word(&mut self, mw: Microword) -> Result<(), Stop> {
        let spec = get(mw, u::SPEC);
        let spec_low = spec & 7;
        let spec_en = spec >> 3;
        let dest = get(mw, u::DEST);
        let multi_prec = get(mw, u::MULTI_PREC) != 0;
        let divide = get(mw, u::DIVIDE) != 0;
        let f = self.e603;
        let need = |v: Option<bool>, what: &str| -> Result<bool, Stop> {
            v.ok_or_else(|| self.unmodelled(format!("{what} of the previous cycle is not defined")))
        };
        // DPE5 E503, E602 (docs/microcode-model.md, "DIVIDE and MULTI PREC").
        let flag_carry_out = if divide {
            need(f.carry_out, "CARRY OUT")?
        } else {
            false
        };
        let flag_carry_02 = if multi_prec {
            need(f.carry_02, "CARRY 02")?
        } else {
            false
        };
        // MULTI SHIFT reaches only F 37 of a left shift of F with the shift
        // style 0 (alu.rs, `into_f37`). Another word does not use FL 02,
        // which the 2901 drives only in a shift (Am2901A, Figure 4).
        let multi_shift_used = dest >= 4 && dest & 2 == 0 && spec_low == 0;
        let flag_fl_02 = if multi_prec && multi_shift_used {
            need(f.fl_02, "FL 02")?
        } else {
            false
        };
        let ad = get(mw, u::AD);
        let func01 = (ad >> 3) & 1 != 0 || (multi_prec && f.func_01) || (divide && flag_carry_out);
        let func = ((ad >> 3) as u8 & 0b110) | u8::from(func01);
        let c = Controls {
            func,
            src_left: (ad & 7) as u8,
            src_right: get(mw, u::RSRC) as u8,
            dest: dest as u8,
            a: get(mw, u::A) as u8,
            b: get(mw, u::B) as u8,
            carry_in: get(mw, u::CRY38) != 0
                || (multi_prec && flag_carry_02)
                || (divide && flag_carry_out),
            inhibit_carry_18: spec_en & 4 != 0 && spec_low == 0,
            clock_left: get(mw, u::CLKL) != 0,
            clock_right: get(mw, u::CLKR) != 0,
            shstyle: spec_low as u8,
            multi_shift: multi_prec && flag_fl_02,
            divide_shift: divide && flag_carry_out,
        };

        // DP before the clock, where it does not depend on the DBus.
        let dp_early = self.alu.y_without_d(&c);
        let uses_d = alu::uses_d(c.src_left) || alu::uses_d(c.src_right);

        // Memory control before the clock: the registers hold the values
        // of the last load (docs/microcode-model.md, "Memory functions").
        let num = get(mw, u::NUM);
        let nb = |k: u32| (num >> (17 - k)) & 1 != 0;
        let mem = get(mw, u::MEM) != 0;
        let dw = self.drom_word();
        let drom_bit = |f| crate::drom::drom_field(dw, f) != 0;
        use crate::fields::d;
        // DPM5 E118, E8.
        let mem_en = mem && (nb(16) || (nb(17) && drom_bit(d::COND_FUNC)));
        let spec_mem_wait = spec_en & 1 != 0 && spec_low == 7;
        let spec_mem_clr = spec_en & 2 != 0 && spec_low == 7;
        let mem_wait = mem_en || spec_mem_wait || spec_mem_clr;
        // DPM5 E30: MEMORY DELAY, the word that completes a cycle.
        let memory_delay = (mem_en || spec_mem_wait) && self.memory_cycle;
        let ac_ref = self.ac_ref();
        let io_cycle = self.vma_flags.io || self.vma_flags.wru || self.vma_flags.vector;
        // The page table entry of a virtual memory cycle with PAGE EN:
        // PAGING OK (DPM6 E47) and PAGED REF (DPM9 E38).
        let page_en = self.apr_enables.bit(23);
        let virtual_cycle = page_en && !self.vma_flags.physical && self.memory_cycle;
        if virtual_cycle && io_cycle {
            return Err(self.unmodelled("a virtual I/O cycle with PAGE EN"));
        }
        let int_or_err = self.int_or_err();
        let entry = if virtual_cycle && !ac_ref {
            Some(self.page_entry()?)
        } else {
            None
        };
        // PF ERR EN (DPM6 E116 ANY): PAGE FAIL 4 to 6 with PAGING OK.
        let pf_err = !int_or_err && entry.is_some_and(|e| self.page_fails(e));
        let cache_hit =
            !int_or_err && entry.is_some_and(|e| !self.page_fails(e)) && self.cache_hit()?;
        // STOP MAIN MEMORY (DPM5 E47): AC REF, DPM6 PAGE FAIL (E54, PAGE
        // FAIL EN: PF ERR EN or INT OR ERR) or CACHE HIT, in every word:
        // so ABORT MEM CYCLE after a page fail writes no cache data
        // **[manual]** (M8621-0-DPM5 rev B; DPM6 rev C).
        let stop_main_memory = ac_ref || pf_err || int_or_err || cache_hit;
        // A page fail (DPM6 E72 PAGE FAIL EN: PF ERR EN or INT OR ERR) in
        // the word that completes the cycle: DPM5 E21 PAGE FAIL DELAY,
        // CSL5 E32 PAGE FAIL. Only the CRA clock runs in it.
        if memory_delay && (pf_err || int_or_err) {
            if spec_en & 1 != 0 && spec_low == 5 {
                return Err(self.unmodelled("page fail in a word with SPEC/NICOND"));
            }
            self.page_fail(mw);
            return Ok(());
        }
        // DPM5 E32, E5: with RPW CYCLE (READ and WRITE from the last load,
        // and no stop), a MEM EN word starts no cycle: no bus request
        // (DPM5 E46) and MEMORY CYCLE 0.
        let rpw = self.cycle.read && self.cycle.write && !stop_main_memory;
        let bus_cycle = memory_delay && !stop_main_memory;
        // PAGED REF (DPM8, DPM9 E153 to E179): the physical page from the
        // entry, VMA 27–35 within the page.
        let address = match entry {
            Some(e) => usize::from(e.page) << 9 | self.vma.field(27, 35) as usize,
            None => self.bus_address(),
        };
        if bus_cycle && !io_cycle && address >= self.memory.len() {
            return Err(self.unmodelled(format!("nonexistent memory {address:o}")));
        }
        // A read gives MB on DBM MEM. For an AC reference, FORCE RAMFILE
        // makes the DBus take the RAM file (DPM5 E14, DPE3 E70).
        if bus_cycle && self.cycle.read {
            self.mb = if io_cycle {
                self.io_read_data()?
            } else {
                self.memory[address]
            };
        }
        // DPM5 E14: FORCE RAMFILE for an AC reference or a cache hit.
        let force_ramfile = (ac_ref || cache_hit) && memory_delay && self.cycle.read;
        if cache_hit && memory_delay && self.cycle.read {
            self.cache_hits += 1;
        }
        // DPMA E22, E54: the RAM file takes the DBus at the end of a
        // cycle: an AC write, or a memory cycle (the cache data).
        let ramfile_write = self.memory_cycle
            && mem_wait
            && (if ac_ref {
                !self.cycle.read
            } else {
                !stop_main_memory
            });
        // CACHE WRITE at the end of a cycle that is not an AC reference, or
        // while VMA SWEEP **[manual]** (M8621-0-DPMA rev B, E22, E110).
        let cache_write =
            self.vma_sweep || (self.memory_cycle && mem_wait && !ac_ref && !stop_main_memory);

        let fmwrite = get(mw, u::FMWRITE) != 0;
        let ir_load = spec_en & 4 != 0 && spec_low == 1;
        let xr_load = spec_en & 2 != 0 && spec_low == 1;
        let ram_write = fmwrite || ramfile_write;
        let dbus_needed = uses_d || ram_write || ir_load || xr_load;
        // DP with D all 0 and all 1: the bits that agree do not depend on
        // the DBus ([`Machine::dbm`], input 2).
        self.dp_bounds = Some((
            alu::word(self.alu.y(&c, alu::d_input(W36::ZERO))),
            alu::word(self.alu.y(&c, alu::d_input(W36::ONES))),
        ));
        let dbus = if dbus_needed {
            Some(self.dbus(mw, dp_early, force_ramfile)?)
        } else {
            None
        };
        let ram_addr = if ram_write {
            Some(self.ram_address(mw, dp_early)?)
        } else {
            None
        };

        // SCAD, with the SC of this cycle (before LOADSC).
        let load_sc = get(mw, u::LOADSC) != 0;
        let load_fe = get(mw, u::LOADFE) != 0;
        let disp = get(mw, u::DISP);
        let scad_needed = load_sc || load_fe || (disp >> 3 & 1 == 0 && disp & 7 == 7);
        let scad = if scad_needed {
            let dp = dp_early.or_else(|| dbus.map(|d| self.alu.y(&c, alu::d_input(d))));
            Some(self.scad(mw, dp.map(alu::word))?)
        } else {
            None
        };

        self.check_spec(spec_en, spec_low)?;
        let pc_flags = spec_en & 4 != 0 && spec_low == 6;
        let ash_test = spec_en & 4 != 0 && spec_low == 4;
        let exp_test = spec_en & 4 != 0 && spec_low == 5;
        let exp_scad = if exp_test {
            Some(self.scad(mw, dp_early.map(alu::word))?)
        } else {
            None
        };
        let dbm_flags = if pc_flags {
            Some(self.dbm(mw, dp_early)?)
        } else {
            None
        };
        // SET SWEEP also loads VMA **[manual]** (DPE5 VMA EN, E205, E601).
        let set_sweep = spec_en & 2 != 0 && spec_low == 4;
        let vma_en = (mem && (get(mw, u::LDVMA) != 0 || (nb(12) && drom_bit(d::VMA)))) || set_sweep;
        let previous_en = self.previous_en(mw);

        // The clock. The skip and dispatch conditions see F and DP of this
        // cycle, SC before LOADSC, and E603 before its load.
        let d_in = dbus.map(alu::d_input).unwrap_or(W40::ZERO);
        // The word after a byte loop: the result must be the same with the
        // undefined bits 0 and 1. Its D input must not come from DP.
        let loop_alu = self.byte_loop_alu.take();
        let dbus_dp =
            get(mw, u::DBUS) == 1 || (get(mw, u::DBUS) == 3 && matches!(get(mw, u::DBM), 3 | 4));
        if loop_alu.is_some() && dbus_needed && dbus_dp {
            return Err(self.unmodelled("the word after a byte loop of DBM uses DP on the DBus"));
        }
        let undefined = self.byte_loop.take();
        let before = undefined.map(|_| self.alu.clone());
        let out = self.alu.step(&c, d_in);
        if let Some(mut alt) = loop_alu {
            let alt_out = alt.step(&c, d_in);
            if alt != self.alu || alt_out != out {
                return Err(self.unmodelled("a bit of the byte loop of DBM reaches a result"));
            }
        }
        if let (Some(undef), Some(mut alt), Some(d)) = (undefined, before, dbus) {
            // DPE3 to DPE4: the loop holds only if DP passes D through, and
            // the word has no other use of the undefined bits.
            let d_alt = W36::wrap(d.get() | undef);
            let alt_out = alt.step(&c, alu::d_input(d_alt));
            let passes = alu::word(out.y) == d && alu::word(alt_out.y) == d_alt;
            let other_use = ram_write
                || ir_load
                || xr_load
                || mem
                || spec_en != 0
                || disp >> 3 != 7
                || get(mw, u::SKIP) >> 3 != 7;
            if !passes || other_use {
                return Err(
                    self.unmodelled("a byte loop of DBM where DP is not D or has another use")
                );
            }
            self.byte_loop_alu = Some(alt);
        }
        let dp = alu::word(out.y);
        let jfcl = self.jfcl(dbm_flags);
        let next = self.next_address(mw, scad, &out, jfcl)?;
        if pc_flags || ash_test || exp_test {
            self.flags = self.new_flags(dbm_flags, ash_test, exp_scad, &out, jfcl)?;
        }
        if let Some(addr) = ram_addr {
            self.ram_file[addr] = dbus.expect("the DBus of a RAM file write");
        }
        // A write takes DP of the word that completes the cycle (DPM8,
        // DPM9 MIX: DP in the data phase). DPMA WRITE CYCLE needs NOT MEM
        // READ (DPMA E17, E21, E14).
        let mut io_written = false;
        if bus_cycle && self.cycle.write && !self.cycle.read {
            if io_cycle {
                // An I/O write: the controller asserts I/O BUSY after the
                // data cycle (EK-OKS10-TM-002 5.3.6).
                let a = address as u32;
                let byte = self.vma_flags.io_byte;
                io_written =
                    self.io
                        .write_cycle(&mut self.memory, self.instructions, a, dp.get(), byte);
            } else {
                self.memory[address] = dp;
                self.writes += 1;
                self.write_hash = record::hash_write(self.write_hash, address as u64, dp);
            }
        }
        if let Some(s) = scad {
            if load_sc {
                self.sc = s;
            }
            if load_fe {
                self.fe = s;
            }
        }
        if ir_load {
            let w = dbus.expect("the DBus of an IR load");
            self.ir = w.field(0, 8) as u16;
            self.ac = w.field(9, 12) as u8;
        }
        if xr_load {
            let w = dbus.expect("the DBus of an XR load");
            self.indirect = w.bit(13);
            self.xr = w.field(14, 17) as u8;
            self.xr_prev = previous_en;
        }
        if mem_en {
            // DPM5 E48, E66: the number field, DP, the DROM, or 0.
            let sel = (u8::from(nb(12)) << 1) | u8::from(nb(13));
            let new = match sel {
                0 => Cycle {
                    read: nb(3),
                    write_test: nb(4),
                    write: nb(5),
                    cache_inhibit: nb(7),
                },
                1 => Cycle {
                    read: dp.bit(3),
                    write_test: dp.bit(4),
                    write: dp.bit(5),
                    cache_inhibit: dp.bit(7),
                },
                2 => Cycle {
                    read: drom_bit(d::READ),
                    write_test: drom_bit(d::TEST),
                    write: drom_bit(d::WRITE),
                    cache_inhibit: false,
                },
                _ => Cycle::default(),
            };
            self.cycle = new;
        }
        let mut started = false;
        if mem_wait {
            // DPM5 E5 (SYNC is 1 at the clock) and DPM6 E16.
            let start = mem_en && !rpw && (self.cycle.read || self.cycle.write);
            self.memory_cycle = start;
            started = start;
            self.int_latched = self.interrupt_req;
            self.one_ms_latched = self.one_ms_sync;
        }
        // The page table and the cache directory take the VMA of the
        // cycle, before VMA loads.
        if self.page_write_en {
            self.write_page_table(dp);
        }
        if cache_write {
            self.write_cache_dir();
        }
        self.page_write_en = spec_en & 1 != 0 && spec_low == 4;
        if vma_en {
            self.load_vma(dp, nb, previous_en, spec_en, spec_low);
            self.vma_sweep = set_sweep;
        }
        // The bus in this microcycle, then the cycles that start in it.
        let group_10 = |code: u32| spec_en & 1 != 0 && spec_low == code;
        self.bus_clock(group_10(2), group_10(3));
        if io_written {
            self.bus.io_busy = UBA_BUSY_CYCLES;
        }
        let f = self.vma_flags;
        if started && self.cycle.read && (f.io || f.wru || f.vector) {
            self.io_command()?;
        }
        // DPEB E167 at each clock: the bus request lines and INTERRUPT EN,
        // from the registers before this clock.
        let bus_next = self.bus_request_lines();
        let int_next = self.interrupt_request();
        self.spec(mw, spec_en, spec_low, &out);
        if spec_en & 4 != 0 && spec_low == 3 {
            self.pi = dp;
        }
        self.interrupt_req = int_next;
        self.bus_requests = bus_next;
        self.e603 = Flags603 {
            func_01: func01,
            carry_02: Some(out.carry_02),
            carry_out: Some(out.carry_out),
            fl_02: out.fl_02,
            qr_37: out.qr_37,
        };
        self.cycles += 1;
        self.ticks += 2 + u64::from(get(mw, u::T));
        self.upc = next;
        Ok(())
    }

    /// The entries that VMA selects in a table of two halves: the left half
    /// if `bit` is 0, the right half if `bit` is 1 or VMA SWEEP **[manual]**
    /// (DPM6 and DPM7: the chip selects of the RAM pairs).
    fn halves(&self, bit: bool, index: usize) -> impl Iterator<Item = usize> {
        let left = (!bit).then_some(index);
        let right = (bit || self.vma_sweep).then_some(256 | index);
        left.into_iter().chain(right)
    }

    /// PAGE WRITE **[manual]** (DPMA E119, E134): the entry by VMA 18–26
    /// takes DP 18, 21, 22, VMA USER and DP 25–35 (DPM6). The write takes
    /// place in the next word **[microcode]** (itspag: "Load page table on
    /// next instruction").
    fn write_page_table(&mut self, dp: W36) {
        let e = PageEntry {
            valid: dp.bit(18),
            writeable: dp.bit(21),
            cacheable: dp.bit(22),
            user: self.vma_flags.user,
            page: dp.field(25, 35) as u16,
        };
        let index = self.vma.field(19, 26) as usize;
        for k in self.halves(self.vma.bit(18), index).collect::<Vec<_>>() {
            self.page_table[k] = Some(e);
        }
    }

    /// CACHE WRITE **[manual]** (DPMA E110, E134): the entry by VMA 27–35
    /// takes NOT VMA PHYSICAL as VALID, VMA USER and VMA 18–26 (DPM7).
    fn write_cache_dir(&mut self) {
        let t = CacheTag {
            valid: !self.vma_flags.physical,
            user: self.vma_flags.user,
            page: self.vma.field(18, 26) as u16,
        };
        let index = self.vma.field(28, 35) as usize;
        for k in self.halves(self.vma.bit(27), index).collect::<Vec<_>>() {
            self.cache_dir[k] = Some(t);
        }
    }

    /// Load VMA and the VMA flags (DPM4 E55, E54, E74, E82, E90, E189).
    /// With `DP FUNC`, E82 takes WRU from DP 11 and E90 takes VECTOR, I/O
    /// BYTE and VMA I/O from DP 12, 13 and 10 (M8621-0-DPM4, E115, E72).
    fn load_vma(
        &mut self,
        dp: W36,
        nb: impl Fn(u32) -> bool,
        previous_en: bool,
        spec_en: u32,
        spec_low: u32,
    ) {
        self.vma = dp;
        self.vma_ram = dp.field(26, 35) as u16;
        let user_flag = self.flags.user;
        let f = if nb(13) {
            VmaFlags {
                user: dp.bit(0),
                fetch: dp.bit(2),
                physical: dp.bit(8),
                previous: dp.bit(9),
                io: dp.bit(10),
                wru: dp.bit(11),
                vector: dp.bit(12),
                io_byte: dp.bit(13),
                extended: nb(15),
            }
        } else {
            let previous = previous_en || (spec_en & 2 != 0 && spec_low == 0);
            VmaFlags {
                user: (!nb(1) && !self.csl.execute && user_flag)
                    || (nb(2) && user_flag)
                    || nb(0)
                    || (previous && self.flags.pcu),
                fetch: nb(2),
                physical: nb(8),
                previous,
                io: false,
                wru: false,
                vector: false,
                io_byte: false,
                extended: nb(15),
            }
        };
        self.vma_flags = f;
    }

    /// The DBus (DPE3, DPE4). FORCE RAMFILE turns the DBM select into the
    /// RAM file select (M8620-0-DPE3 rev B, E70).
    fn dbus(&mut self, mw: Microword, dp: Option<W40>, force_ramfile: bool) -> Result<W36, Stop> {
        let mut sel = get(mw, u::DBUS);
        if force_ramfile && sel == 3 {
            sel = 2;
        }
        let dp_word = |m: &Self| {
            dp.map(alu::word)
                .ok_or_else(|| m.unmodelled("DP depends on the DBus, which selects DP"))
        };
        match sel {
            0 => Ok(self.dbus_pc_flags()),
            1 => dp_word(self),
            2 => {
                let a = self.ram_address(mw, dp)?;
                Ok(self.ram_file[a])
            }
            3 => self.dbm(mw, dp),
            other => Err(self.unmodelled(format!("DBUS {other}"))),
        }
    }

    /// DBus input 0 **[manual]** (M8620-0-DPE3 and DPE4 rev B, input 0 of
    /// each 74LS153): the program flags in bits 00–06 and 09–12, PI NEW in
    /// 19–21, 1 in 22–25 (+3V), VMA 26–35 (DPE5 E156, E176, which load
    /// with VMA), and 0 in the other bits.
    fn dbus_pc_flags(&self) -> W36 {
        let f = self.flags;
        let left = [
            (0, f.ov),
            (1, f.cry0),
            (2, f.cry1),
            (3, f.fov),
            (4, f.fpd),
            (5, f.user),
            (6, f.pcu),
            (9, f.trap2),
            (10, f.trap1),
            (11, f.fxu),
            (12, f.nodiv),
        ]
        .iter()
        .filter(|(_, v)| *v)
        .fold(0u64, |acc, (b, _)| acc | 1 << (17 - b));
        let right = u64::from(self.pi_new()) << 14 | 0o17 << 10 | self.vma.field(26, 35);
        W36::wrap(left << 18 | right)
    }

    /// The DBM mixer (M8621-0-DPM1 and DPM2 rev B).
    fn dbm(&mut self, mw: Microword, dp: Option<W40>) -> Result<W36, Stop> {
        let dbm = get(mw, u::DBM);
        let dp_word = |m: &Self| {
            dp.map(alu::word)
                .ok_or_else(|| m.unmodelled("DP depends on the DBus, which selects DP"))
        };
        match dbm {
            0 => self.dbm_apr(mw, dp),
            1 => {
                let b = bytes(self.scad(mw, dp.map(alu::word))?).get();
                Ok(W36::wrap(b | u64::from(self.dp_bit(dp, 35)?)))
            }
            2 => self.dbm_exp(mw, dp),
            // SPEC 1 to 5 insert a byte; 6 and 7 are "SAME AS ZERO" (ks10,
            // "MICROCODE FIELDS -- SPEC", the DBM table).
            3 if matches!(get(mw, u::SPEC) & 7, 0 | 6 | 7) => dp_word(self),
            // DPM1 E94 decodes SPEC 1 to 5 while DBM SEL 4 is 0 and DBM
            // SEL 1 is 1, and clears SEL 2 of that byte (E72, E86): the
            // byte takes input 1 **[manual]** (M8621-0-DPM1 rev B).
            // If DP depends on D, DP and the DBus form a loop: only the
            // inserted byte is defined. The model takes the other bits as
            // 0 and checks that no result depends on them
            // ([`Machine::step_word`]).
            3 => {
                let (first, last) = BYTE_BITS[(get(mw, u::SPEC) & 7) as usize - 1];
                let b = bytes(self.scad(mw, dp.map(alu::word))?).get();
                // Bit 35 of input 1 is DP 35: byte 5 keeps it.
                let mask = (((1u64 << (last - first + 1)) - 1) << (35 - last)) & !1;
                let w = match dp {
                    Some(d) => alu::word(d).get(),
                    None => {
                        self.byte_loop = Some(!mask & 0o777777_777777);
                        0
                    }
                };
                Ok(W36::wrap((w & !mask) | (b & mask)))
            }
            4 => Ok(dp_word(self)?.swap()),
            // VMA FLAGS,,VMA.
            5 => {
                let f = self.vma_flags;
                let c = self.cycle;
                let bits = [
                    (0, f.user),
                    (2, f.fetch),
                    (3, c.read),
                    (4, c.write_test),
                    (5, c.write),
                    (7, c.cache_inhibit),
                    (8, f.physical),
                    (9, f.previous),
                    (10, f.io),
                    (11, f.wru),
                    (12, f.vector),
                    (13, f.io_byte),
                ];
                let left = bits
                    .iter()
                    .filter(|(_, v)| *v)
                    .fold(0u64, |acc, (b, _)| acc | 1 << (17 - b));
                let left = left | self.vma.field(14, 17);
                Ok(W36::wrap(left << 18 | self.vma.field(18, 35)))
            }
            6 => Ok(self.mb),
            7 => {
                let n = u64::from(get(mw, u::NUM));
                Ok(W36::wrap((n << 18) | n))
            }
            other => Err(self.unmodelled(format!("DBM {other}, SPEC {:o}", get(mw, u::SPEC)))),
        }
    }

    /// Bit `k` of DP, also if DP depends on the DBus but not that bit
    /// ([`Machine::dp_bounds`]).
    fn dp_bit(&self, dp: Option<W40>, k: u32) -> Result<bool, Stop> {
        match (dp, self.dp_bounds) {
            (Some(d), _) => Ok(alu::word(d).bit(k)),
            (None, Some((lo, hi))) if lo.bit(k) == hi.bit(k) => Ok(lo.bit(k)),
            _ => Err(self.unmodelled(format!("DBM needs DP {k}, which depends on the DBus"))),
        }
    }

    /// DBM input 2, `EXP` **[manual]** (M8621-0-DPM1 and DPM2 rev B,
    /// input 2D of each 74LS151): bit 00 is 0, bits 01–08 are SCAD 02–09,
    /// bits 09–17 are DP 09–17, bits 18–23 are 0, bits 24–33 are DPMC MSEC
    /// COUNT 24–33, and bits 34–35 are 0. The MSEC counter (DPMC E181,
    /// E189, E180) counts 4.1 MHz in 12 bits, so it wraps about each
    /// millisecond; its two low bits do not reach DBM. The model takes it
    /// as the units of the time base since the last tick: one millisecond
    /// is `INSTRUCTIONS_PER_MS` instructions and 2^12 units, the time base
    /// of `emu/ks10-core-soft` (docs/microcode-model.md, "Time and the boot
    /// of ITS").
    fn dbm_exp(&mut self, mw: Microword, dp: Option<W40>) -> Result<W36, Stop> {
        let scad = self.scad(mw, dp.map(alu::word))?;
        let dp_left = match (dp, self.dp_bounds) {
            (Some(d), _) => alu::word(d).field(9, 17),
            (None, Some((lo, hi))) if lo.field(9, 17) == hi.field(9, 17) => lo.field(9, 17),
            _ => return Err(self.unmodelled("DBM EXP needs DP 09-17, which depends on the DBus")),
        };
        let exponent = u64::from(scad) & 0o377; // SCAD 02-09
        let count = ks10_machine::msec_count(self.instructions, self.last_tick);
        let left = exponent << 9 | dp_left;
        Ok(W36::wrap(left << 18 | (count & 0o7774)))
    }

    /// DBM input 0 **[manual]** (M8621-0-DPM1 and DPM2 rev B, input 0D of
    /// each 74LS151). Bits 00–16 are SCAD 01–07 in turn, as input 1, and
    /// bit 17 is SCAD 00. Bits 18–21 are the page fail dispatch (DPM6),
    /// 22 TRAP EN, 23 PAGE EN, 24–31 the APR flags, 32 APR INT REQ. The
    /// inputs of bits 33–35 are open: the model takes them as 1
    /// **[estimate]** (docs/unknowns.md).
    fn dbm_apr(&mut self, mw: Microword, dp: Option<W40>) -> Result<W36, Stop> {
        let scad = self.scad(mw, dp.map(alu::word))?;
        let mut left = bytes(scad).lh().get() & !1;
        left |= u64::from((scad >> 9) & 1);
        let e = self.apr_enables;
        let right = u64::from(self.pf_disp()?) << 14
            | u64::from(e.bit(22)) << 13
            | u64::from(e.bit(23)) << 12
            | u64::from(self.apr_flags) << 4
            | u64::from(self.apr_int_req()) << 3
            | 0o7;
        Ok(W36::wrap(left << 18 | right))
    }

    /// The page table entry of VMA 18–26 (DPM6), or a stop if nothing
    /// wrote it since power on.
    fn page_entry(&self) -> Result<PageEntry, Stop> {
        let index = self.vma.field(19, 26) as usize;
        let k = if self.vma.bit(18) { 256 | index } else { index };
        self.page_table[k].ok_or_else(|| self.unmodelled("a page table entry that nothing wrote"))
    }

    /// The page fail conditions of DPM6 E116 for an entry: not valid,
    /// USER differs (E124), a write test of a page that is not writeable
    /// (E41) **[manual]** (M8621-0-DPM6 rev C).
    fn page_fails(&self, e: PageEntry) -> bool {
        !e.valid || e.user != self.vma_flags.user || (!e.writeable && self.cycle.write_test)
    }

    /// CACHE HIT **[manual]** (M8621-0-DPM7 rev B, E114, E41, E163, E177,
    /// E185) for a cycle whose entry does not page-fail: a read without
    /// CACHE INH of a valid, cacheable page, CSL3 CACHE EN, and a tag equal
    /// to VALID, VMA USER and VMA 18–26. The model has no parity bits, so
    /// the parity compare (E155, E163 A4) is equal. VMA JUST LOADED (E41)
    /// is 0 in the word that completes the cycle ([`Machine::page_fail`]).
    fn cache_hit(&self) -> Result<bool, Stop> {
        let e = self.page_entry()?;
        if !(self.cycle.read
            && !self.cycle.cache_inhibit
            && e.valid
            && e.cacheable
            && self.csl.cache_en)
        {
            return Ok(false);
        }
        let index = self.vma.field(28, 35) as usize;
        let k = if self.vma.bit(27) { 256 | index } else { index };
        let t =
            self.cache_dir[k].ok_or_else(|| self.unmodelled("a cache tag that nothing wrote"))?;
        Ok(t.valid && t.user == self.vma_flags.user && u64::from(t.page) == self.vma.field(18, 26))
    }

    /// INT OR ERR **[manual]** (M8621-0-DPM6 rev C, E15, E9): INT or 1MS
    /// (DPM6 E16, latched at MEM WAIT) in a memory cycle that is virtual
    /// and not a write, or NXM ERR or BAD DATA EN. The model has neither
    /// error: it stops on nonexistent memory and has no parity.
    fn int_or_err(&self) -> bool {
        self.memory_cycle
            && !self.vma_flags.physical
            && !self.cycle.write
            && (self.int_latched || self.one_ms_latched)
    }

    /// A page fail in the word that completes a memory cycle **[manual]**.
    /// DPM5 E21 gives PAGE FAIL DELAY: MEMORY DELAY AND PAGE FAIL EN AND
    /// NOT VMA JUST LOADED (M8621-0-DPM5 rev B). CSL5 E32 then gives PAGE
    /// FAIL, which turns off the DPE/M clock (E54; M8616-0-CSL5 rev D):
    /// the word changes nothing in the data path. The CRA clock runs:
    /// CRA2 E34 moves the stack as for a CALL, so that the stack holds the
    /// address of the word, and the page fail sets all bits of the next
    /// address (M8622-0-CRA2 rev B; CRA1). The microcode returns to the
    /// word with `RETURN [0]` (itspag, `PAGE-CONTINUE`).
    ///
    /// The model has no wait cycles. It takes VMA JUST LOADED as 0 in the
    /// word that completes the cycle, as after a wait for the memory
    /// **[estimate]** (docs/unknowns.md).
    fn page_fail(&mut self, mw: Microword) {
        self.sp = (self.sp + 1) & 15;
        self.stack[usize::from(self.sp)] = self.upc;
        self.bus_clock(false, false);
        self.cycles += 1;
        self.ticks += 2 + u64::from(get(mw, u::T));
        self.upc = 0o7777;
    }

    /// The page fail dispatch PF DISP 10, 04, 02, 01 **[manual]**
    /// (M8621-0-DPM6 rev C, E116, E115, E9, E47, E124, E41). PAGING OK
    /// enables the 74148 E116; the drawing names its inputs from D0 (the
    /// highest) to D7, so D4 to D7 are its inputs 3 to 0: page not valid,
    /// USER of the page and of VMA differ, and on D6 and D7 (one line) a
    /// write test of a page that is not writeable. Its outputs are active
    /// low. The codes agree with the
    /// dispatch of the microcode **[microcode]** (itspag, `PFD`: 12
    /// nonexistent page, 13 exec/user mismatch, 10 read-only page, 11 the
    /// 1 ms timer). NXM ERR and BAD DATA ERR are 0: the model stops on
    /// nonexistent memory and has no parity.
    fn pf_disp(&self) -> Result<u8, Stop> {
        let f = self.vma_flags;
        let int_or_err = self.int_or_err();
        let paging_ok = !self.ac_ref() && self.apr_enables.bit(23) && !f.physical && !int_or_err;
        // The highest active input of E116, if it is enabled.
        let mut input = None;
        if paging_ok {
            let index = self.vma.field(19, 26) as usize;
            let k = if self.vma.bit(18) { 256 | index } else { index };
            let e = self.page_table[k].ok_or_else(|| {
                self.unmodelled("page fail dispatch: an entry that nothing wrote")
            })?;
            input = [
                (3, !e.valid),
                (2, e.user != f.user),
                (1, !e.writeable && self.cycle.write_test),
            ]
            .iter()
            .find(|(_, on)| *on)
            .map(|&(k, _)| k);
        }
        // E116 outputs A0–A2, active low; all 1 if disabled or no input.
        let a = !input.unwrap_or(0u8) & 7;
        let d01 = a & 1;
        let d02 = u8::from(a & 2 == 0);
        let d04 = u8::from(a & 4 == 0);
        let d10 = u8::from(!int_or_err);
        Ok(d10 << 3 | d04 << 2 | d02 << 1 | d01)
    }

    /// JFCL 09–12 (DPE9 E107, E109): a flag, its AC bit, and JFCL EN (DBM
    /// 14 with SPEC/PC FLAGS, E110).
    fn jfcl(&self, dbm: Option<W36>) -> [bool; 4] {
        let Some(dbm) = dbm else { return [false; 4] };
        let en = dbm.bit(14);
        let f = self.flags;
        let ac = |b: u8| self.ac & b != 0;
        [
            f.ov && ac(8) && en,
            f.cry0 && ac(4) && en,
            f.cry1 && ac(2) && en,
            f.fov && ac(1) && en,
        ]
    }

    /// The program flags after the clock (DPE9). `dbm` is the DBM mixer for
    /// SPEC/PC FLAGS, `None` for ASH TEST and EXP TEST. `exp_scad` is the
    /// SCAD output of an EXP TEST word.
    fn new_flags(
        &self,
        dbm: Option<W36>,
        ash_test: bool,
        exp_scad: Option<u16>,
        out: &Outputs,
        jfcl: [bool; 4],
    ) -> Result<PcFlags, Stop> {
        let f = self.flags;
        let pf = dbm.is_some();
        let n = |i: u32| dbm.is_some_and(|w| w.bit(i));
        let load = n(15);
        let carry = n(17);
        let dp = alu::word(out.y);
        let dp_sign = out.y.get() >> 39 & 1 != 0;
        let overflow = dp_sign != dp.bit(0);
        let pick = |dp_bit: u32, flag: bool| if load { dp.bit(dp_bit) } else { flag };
        let mut g = f;
        if pf {
            // E305, E306, E204, E401.
            g.cry0 = (pick(1, f.cry0) && !jfcl[1]) || (carry && out.carry_out);
            let dp_cry1 = carry && (overflow ^ out.carry_out);
            g.cry1 = (pick(2, f.cry1) && !jfcl[2]) || dp_cry1;
            g.fpd = (pick(4, f.fpd) || n(4)) && !n(3);
            // E210, E211, E106, E110, E302.
            g.user = (f.user && n(5)) || (load && dp.bit(5));
            g.pcu = (load && dp.bit(6) && !f.user)
                || (!load && f.pcu)
                || (f.user && n(9))
                || (n(5) && f.pcu && dp.bit(6) && load);
            g.trap2 = (dp.bit(9) && n(15)) || n(7);
            g.nodiv = n(2) || pick(12, f.nodiv);
        }
        // E209, E205, E107, E207, E206, E501, E208. EXP TEST tests SCAD 01
        // (X: OV, TRAP 1, FOV; E501) and SCAD 00 (FXU; E206 pins 12, 13
        // and 1, rev B E84). E206 drives D3 of E209 without an inversion,
        // and the FXU flag is the Q-bar output, pin 14.
        let x = exp_scad.is_some_and(|s| s >> 8 & 1 != 0);
        let u = exp_scad.is_some_and(|s| s >> 9 & 1 != 0);
        let p = (ash_test && dp.bit(1) != dp.bit(2)) || (carry && overflow);
        g.ov = p || x || pick(0, f.ov) && !jfcl[0] || n(0);
        g.trap1 = p || x || (load && dp.bit(10)) || n(8);
        g.fov = x || (pick(3, f.fov) && !jfcl[3]) || n(1);
        g.fxu = pick(11, f.fxu) || u;
        Ok(g)
    }

    /// Stop before the clock if a special function is not modelled.
    fn check_spec(&self, en: u32, low: u32) -> Result<(), Stop> {
        // Group 40 (DPE5 E102): 0 CRY 18 INH, 1 IR LOAD, 3 PI LOAD,
        // 4 ASH TEST, 5 EXP TEST, 6 PC FLAGS, 7 AC BLOCKS EN.
        if en & 4 != 0 && !matches!(low, 0 | 1 | 3 | 4 | 5 | 6 | 7) {
            return Err(self.unmodelled(format!("SPEC group 40, code {low}")));
        }
        // Group 20 (DPMA E78): 0 PREVIOUS, 1 XR LOAD, 3 APR FLAGS, 4 SET
        // SWEEP, 5 APR EN, 6 PXCT OFF, 7 MEM CLR.
        if en & 2 != 0 && !matches!(low, 0 | 1 | 3 | 4 | 5 | 6 | 7) {
            return Err(self.unmodelled(format!("SPEC group 20, code {low}")));
        }
        // Group 10 (CRA2 E97, DPMA E2): 0 CONSOLE, 1 CLR 1 MSEC, 2 CLR IO
        // LATCH, 3 CLR IO BUSY, 4 PAGE WRITE, 5 NICOND, 6 PXCT EN, 7 MEM
        // WAIT: all are modelled.
        Ok(())
    }

    /// The special functions after the clock, except PI LOAD.
    fn spec(&mut self, mw: Microword, en: u32, low: u32, out: &Outputs) {
        let dp = alu::word(out.y);
        if en & 4 != 0 && low == 7 {
            // AC BLOCKS EN: DP 06–08 to the current block, DP 09–11 to the
            // previous block (DPE5 E308).
            self.cur_block = dp.field(6, 8) as u8;
            self.prev_block = dp.field(9, 11) as u8;
        }
        if en & 2 != 0 && low == 5 {
            self.apr_enables = dp;
        }
        if en & 2 != 0 && low == 3 {
            // SPEC/APR FLAGS clocks the APR flags from DP 24–31 **[manual]**
            // (DPMB, DPM5 CLK APR FLAGS). Flag 25 presets CSL4 E161, the
            // interrupt of the 8080 (M8616-0-CSL4 rev C): the doorbell.
            let new = dp.field(24, 31) as u8;
            if new & !self.apr_flags & 0o100 != 0 {
                self.cty.doorbell(self.instructions);
            }
            self.apr_flags = new;
        }
        if en & 1 != 0 {
            let num = get(mw, u::NUM);
            let nb = |k: u32| (num >> (17 - k)) & 1 != 0;
            match low {
                // CONSOLE (CSL4 E171, E149, E135, E161).
                0 => {
                    let c = &mut self.csl;
                    c.cont &= !nb(12);
                    c.execute &= !nb(13);
                    c.run &= !nb(14);
                    if nb(15) {
                        c.halt_loop = true;
                    }
                    if nb(16) {
                        c.halt_loop = false;
                    }
                }
                // NICOND: TRAP CYCLE is NOT NICOND 09 (CRA2 E159). The
                // word ends an instruction; the devices run to its time.
                5 => {
                    self.trap_cycle = self.nicond() < 4;
                    self.instruction_boundary();
                }
                // PXCT EN, with PXCT OFF from group 20 (DPMA E71).
                6 => {
                    self.pxct = Pxct {
                        on: !(en & 2 != 0),
                        bits: dp.field(9, 12) as u8,
                    }
                }
                _ => {}
            }
        }
        // DPMC E1, E3: the copy follows E1 at each clock; CLR 1 MSEC clears
        // E1 after the clock.
        self.one_ms_sync = self.one_ms;
        if en & 1 != 0 && low == 1 {
            self.one_ms = false;
        }
    }

    /// The end of an instruction. The 1 ms clock, the 8080 and the devices
    /// run to the instruction count before it counts this instruction, in
    /// the order of `ks10-core-soft` (`Machine::step`).
    fn instruction_boundary(&mut self) {
        let now = self.instructions;
        while now >= self.last_tick + INSTRUCTIONS_PER_MS {
            self.last_tick += INSTRUCTIONS_PER_MS;
            self.millisecond();
        }
        if self.cty.run(now, &mut self.memory) {
            // CSL4 INT 10 L, a pulse of the 8080 write to 116, presets APR
            // flag 31 **[manual]** (M8616-0-CSL4 rev C, E171, E170, E163;
            // DPMB E158).
            self.apr_flags |= 1;
        }
        self.io.poll(now);
        if !self.io.events.is_empty() {
            self.io.run_events(&mut self.memory, now);
        }
        self.instructions += 1;
        self.record_due = true;
    }

    /// A 1 ms tick of the timer (DPMC E1): E1 takes 1 MSEC EN.
    pub fn millisecond(&mut self) {
        self.one_ms = self.csl.one_ms_en;
    }

    /// NICOND 09–11 (CRA2 E147, DPE9 E125): the first active input of
    /// 0 EA1, 1 TRAP 3, 2 TRAP 2, 3 TRAP 1, 4 EF1, 5 not RUN, 6 EK1, 7.
    /// EA1, EF1 and EK1 are inactive (docs/microcode-model.md, "NICOND").
    fn nicond(&self) -> u16 {
        let trap_en = self.csl.trap_en && self.apr_enables.bit(22);
        let f = self.flags;
        if trap_en && f.trap2 && f.trap1 {
            1
        } else if trap_en && f.trap2 {
            2
        } else if trap_en && f.trap1 {
            3
        } else if !self.csl.run {
            5
        } else {
            7
        }
    }

    /// The highest PI level in progress, PI CURRENT (DPEB E134). The WRU
    /// cycle sends it on bus data 15 to 17 (DPEB E96; EK-OKS10-TM-002
    /// 5.6.6).
    fn pi_current(&self) -> Option<u8> {
        (1..=7).find(|n| !self.pi.bit(20 + u32::from(*n)))
    }

    /// The bus signals of one microcycle for the I/O LATCH (CRA2 E148,
    /// E183, E184): I/O LATCH takes (I/O LATCH AND NOT `CLR IO LATCH`) OR
    /// BUS I/O BUSY OR BUS I/O DATA CYCLE. E98 takes the old I/O LATCH at
    /// the end of the microcycle (DISP & SKIP EN with SYNC, E31).
    /// `CLR IO BUSY` sends CLR BUSY on the bus (EK-0KS10-TM-001, Figure
    /// 5-28), and the controller ends I/O BUSY.
    fn bus_clock(&mut self, clr_latch: bool, clr_busy: bool) {
        let b = &mut self.bus;
        let busy = b.io_busy > 0;
        let mut io_data = false;
        b.data_in = match b.data_in {
            Some((1, io)) => {
                io_data = io;
                None
            }
            Some((n, io)) => Some((n - 1, io)),
            None => None,
        };
        b.io_latch_skip = b.io_latch;
        b.io_latch = (b.io_latch && !clr_latch) || busy || io_data;
        b.io_busy = b.io_busy.saturating_sub(1);
        if clr_busy {
            b.io_busy = 0;
        }
    }

    /// The command/address cycle of an I/O read, a WRU cycle or a vector
    /// cycle (EK-OKS10-TM-002 5.3.6, 5.3.7). The controller reads its
    /// register now; the word reaches MB at the data cycle.
    fn io_command(&mut self) -> Result<(), Stop> {
        let a = self.bus_address() as u32;
        let f = self.vma_flags;
        let b = &mut self.bus;
        b.reply = None;
        b.data_in = None;
        let ctl = a >> 18;
        if f.wru || f.vector {
            let Some(n) = self.pi_current() else {
                return Err(self.unmodelled("WRU or vector cycle with no PI level in progress"));
            };
            if f.wru {
                // The adapters answer on the data lines in the second
                // cycle; there is no data cycle and no I/O BUSY (5.3.7,
                // 5.6.2).
                self.bus.reply = Some(W36::wrap(self.io.wru(n)));
            } else if let Some(v) = self.io.vector(ctl, n) {
                // The adapter asserts I/O BUSY and sends the vector with
                // I/O DATA CYCLE (EK-OKS10-TM-002 5.9.6).
                self.answer(u64::from(v), UBA_BUSY_CYCLES, true);
            }
            return Ok(());
        }
        if ctl == 0 {
            if a & CONSOLE_REGISTER_BIT == 0 {
                return Err(
                    self.unmodelled("I/O read of controller 0 other than the console register")
                );
            }
            let c = self.csl;
            self.bus.reply = Some(c.data);
            self.bus.data_in = Some((CONSOLE_DATA_CYCLES, c.io_data_cycle));
            return Ok(());
        }
        // A Unibus adapter asserts I/O BUSY, then sends the word with I/O
        // DATA CYCLE (EK-OKS10-TM-002 5.3.6, 5.9.5). For an address with
        // no device the adapter does not answer (docs/unknowns.md).
        if let Reply::Value(v) = self.io.read_cycle(a, f.io_byte, self.instructions) {
            self.answer(v, UBA_BUSY_CYCLES, true);
        }
        Ok(())
    }

    /// A controller answers a read: I/O BUSY for `busy` microcycles, then
    /// the data cycle.
    fn answer(&mut self, v: u64, busy: u32, io_data_cycle: bool) {
        self.bus.reply = Some(W36::wrap(v));
        self.bus.io_busy = busy;
        self.bus.data_in = Some((busy + 1, io_data_cycle));
    }

    /// The word of an I/O read for the word that completes the cycle. If
    /// the data cycle has not come yet, the processor clock waits for it
    /// (EK-OKS10-TM-002 5.6.1), so it comes in this microcycle.
    fn io_read_data(&mut self) -> Result<W36, Stop> {
        let Some(w) = self.bus.reply.take() else {
            return Err(self.unmodelled("I/O read that no controller answers: the processor waits"));
        };
        if let Some((_, io)) = self.bus.data_in {
            self.bus.data_in = Some((1, io));
            self.bus.io_busy = 0;
        }
        Ok(w)
    }

    /// Run until `done` is true, for at most `limit` microcycles. Returns
    /// whether `done` became true.
    pub fn run_until(&mut self, limit: u64, done: impl Fn(&Machine) -> bool) -> Result<bool, Stop> {
        for _ in 0..limit {
            if done(self) {
                return Ok(true);
            }
            self.step()?;
        }
        Ok(done(self))
    }

    /// The console command SM and the start of a boot command (cmds.m80,
    /// SM1, BT.GO). MRINT: RUN, EXECUTE and CONTINUE to 0, CRAM RESET, DP
    /// RESET and the bus RESET (hcore.m80, MRCMD, MRINT). Then the 8080
    /// deposits all ones in location 0, sets the CRAM address to 0, starts
    /// the clock and waits for HALT LOOP. BT.GO then sets the default
    /// enables: 1 MSEC EN and CACHE EN (condef.m80, DEFLTE), TRAP EN
    /// (TRPDEF). Which registers DP RESET clears is not modelled
    /// (docs/unknowns.md); the power-up sequence sets the state that the
    /// microcode uses.
    ///
    /// While SM1 runs, register 100 has only the parity bits (PARBT AND
    /// 140), so 1 MSEC EN and CACHE EN are 0. The wait in HLTCM (DELAY.
    /// with 255 loops of DCR, PUSH, POP and JNZ: 9180 8080 states) takes
    /// more than 1 ms **[estimate]**: the 8080A runs at 3 MHz or less. So
    /// the 1 ms flag (DPMC E1) takes 0 at least once.
    fn console_reset(&mut self) -> Result<(), Stop> {
        let c = &mut self.csl;
        (c.run, c.cont, c.execute) = (false, false, false);
        (c.one_ms_en, c.cache_en) = (false, false);
        c.halt_loop = false;
        self.io.reset();
        self.bus = Bus::default();
        // DP RESET (204 bit 2) clears the APR flags **[manual]** (CSL4
        // E156; M8621-0-DPMA rev B, E23, E87; DPMB).
        self.apr_flags = 0;
        self.memory[0] = W36::ONES;
        self.upc = 0;
        self.last_tick = self.instructions;
        if !self.run_until(CONSOLE_WAIT_CYCLES, |m| m.csl.halt_loop)? {
            return Err(self.unmodelled("the microcode does not reach the halt loop"));
        }
        self.millisecond();
        let c = &mut self.csl;
        (c.one_ms_en, c.cache_en, c.trap_en) = (true, true, true);
        Ok(())
    }

    /// The console command ST (cmds.m80, STINT, EXINT, COCMD): EXECUTE of
    /// JRST `addr`, then CONTINUE and RUN. The 8080 checks that the
    /// microcode clears CONTINUE (DNF). With RUN 0, the microcode halts at
    /// the end of the JRST (inout, `HALTED`: halt code 2 in location 0, PC
    /// in location 1, the halt status block). LONG.DELAY gives it the time
    /// to reach the halt loop before CONTINUE and RUN.
    fn console_start(&mut self, addr: u32) -> Result<(), Stop> {
        let c = &mut self.csl;
        c.data = W36::wrap(0o254000 << 18 | u64::from(addr & 0o777777));
        c.io_data_cycle = true;
        (c.execute, c.cont) = (true, true);
        if !self.run_until(CONSOLE_WAIT_CYCLES, |m| !m.csl.cont)? {
            return Err(self.unmodelled("the microcode does not clear CONTINUE after EXECUTE"));
        }
        if !self.run_until(CONSOLE_WAIT_CYCLES, |m| m.csl.halt_loop)? {
            return Err(self.unmodelled("the microcode does not halt after EXECUTE"));
        }
        // 212/5: CONTINUE and RUN, EXECUTE 0.
        let c = &mut self.csl;
        (c.execute, c.cont, c.run) = (false, true, true);
        Ok(())
    }

    fn drom_word(&self) -> u64 {
        self.drom[usize::from(self.ir & 0o777)]
    }

    /// The address of the next microinstruction (CRA1, CRA2, DPEA), and
    /// the stack.
    fn next_address(
        &mut self,
        mw: Microword,
        scad: Option<u16>,
        out: &Outputs,
        jfcl: [bool; 4],
    ) -> Result<u16, Stop> {
        use crate::drom::drom_field;
        use crate::fields::d;
        let dw = self.drom_word();
        let dr = |f| drom_field(dw, f) as u16;
        let ad_zero = out.adl_zero() && out.adr_zero();
        let y = alu::word(out.y);
        let need = |v: Option<bool>, what: &str| {
            v.ok_or_else(|| self.unmodelled(format!("{what} is not defined")))
        };

        // Skip: bit 11.
        let skip = get(mw, u::SKIP);
        let (en, sel) = (skip >> 3, skip & 7);
        let mut skip_bit = false;
        if sel != 0 {
            if en & 4 == 0 {
                skip_bit |= match sel {
                    1 => out.carry_out,
                    2 => out.adl_zero(),
                    3 => out.adr_zero(),
                    4 => !self.flags.user,
                    5 => self.flags.fpd,
                    6 => self.ac == 0,
                    7 => self.interrupt_req,
                    _ => return Err(self.unmodelled(format!("SKIP group 40, input {sel}"))),
                };
            }
            if en & 2 == 0 {
                skip_bit |= match sel {
                    1 => out.carry_02,
                    2 => out.adl_sign(),
                    3 => out.adr_sign(),
                    4 => self.flags.pcu,
                    // JFCL SKIP (DPE9 E108).
                    5 => jfcl.iter().any(|b| *b),
                    // DP CRY1: DP SIGN XOR DP 00 (DP OVERFLOW), XOR CARRY OUT
                    // (M8620-0-DPE9 rev A1, E504). DP is Y.
                    6 => {
                        let v = out.y.get();
                        let overflow = (v >> 39 & 1) != (v >> 37 & 1);
                        overflow ^ out.carry_out
                    }
                    // TXXX: DROM TXXX EN (a copy of DROM bit 9) XOR AD=0
                    // (DPEA E319; EK-OKS10-TM-002 5.4.2).
                    7 => ((dw >> (35 - 9)) & 1 != 0) ^ ad_zero,
                    _ => return Err(self.unmodelled(format!("SKIP group 20, input {sel}"))),
                };
            }
            if en & 1 == 0 {
                skip_bit |= match sel {
                    1 => self.trap_cycle,
                    2 => ad_zero,
                    3 => self.sc & 0o1000 != 0,
                    // CRA2 E98: the console and the 1 ms flag.
                    4 => self.csl.execute,
                    // -IO BUSY: NOT I/O LATCH (CRA2 E98 D1).
                    5 => !self.bus.io_latch_skip,
                    6 => !self.csl.cont,
                    7 => !self.one_ms_sync,
                    _ => return Err(self.unmodelled(format!("SKIP group 10, input {sel}"))),
                };
            }
        }

        // Dispatch.
        let disp = get(mw, u::DISP);
        let (en, sel) = (disp >> 3, disp & 7);
        let ret = en & 2 == 0 && sel & 3 == 1;
        let call = get(mw, u::CALL) != 0;
        if ret && call {
            return Err(self.unmodelled("CALL and RETURN together"));
        }
        let sbr_ret = self.stack[usize::from(self.sp & 15)];
        // DPEA E118: AC replaces the low four bits of the ROM J.
        let j_low = if dr(d::ACDISP) != 0 {
            u16::from(self.ac & 15)
        } else {
            dr(d::J) & 15
        };
        let a_eq_j = dr(d::I) != 0;
        let mut low = 0u16; // bits 08–11
        let mut high = 0u16; // bits 00–07
        if en & 4 == 0 {
            low |= match sel {
                0 => 0,
                1 => y.field(18, 21) as u16,
                2 => j_low,
                3 => {
                    if a_eq_j {
                        j_low
                    } else {
                        dr(d::A)
                    }
                }
                4 => {
                    (u16::from(ad_zero) << 3)
                        | ((y.field(8, 9) as u16) << 1)
                        | u16::from(out.y.get() >> 39 & 1 != 0)
                }
                5 => y.field(32, 35) as u16,
                6 => dr(d::A),
                _ => dr(d::B),
            };
        }
        if en & 2 == 0 {
            let rom_j_high = (dr(d::J) >> 4) & 15;
            high |= match sel & 3 {
                0 => return Err(self.unmodelled("DISP DIAG ADR")),
                1 => sbr_ret >> 4,
                2 => 0b0011_0000 | rom_j_high,
                _ => {
                    let aread_04_07 = if a_eq_j { rom_j_high } else { 0b0010 };
                    (if a_eq_j { 0b0011_0000 } else { 0 }) | aread_04_07
                }
            };
        }
        if en & 1 == 0 {
            low |= match sel {
                0 => return Err(self.unmodelled("DISP DIAG ADR")),
                1 => sbr_ret & 15,
                2 => {
                    // MUL: 0, FLAG QR 37, FK2, 0. FK2 is a backplane pin
                    // with no known source; it does not matter if J has
                    // bit 10 set.
                    if get(mw, u::J) & 2 == 0 {
                        return Err(self.unmodelled("DISP/MUL with J bit 10 clear (FK2)"));
                    }
                    u16::from(need(self.e603.qr_37, "FLAG QR 37")?) << 2
                }
                // NICOND: MEMORY CYCLE and NICOND 09–11.
                4 => (u16::from(self.memory_cycle) << 3) | self.nicond(),
                5 => byte_disp(y),
                6 => {
                    let jrst0 = self.ir == 0o254 && self.ac == 0;
                    (u16::from(!jrst0) << 3)
                        | (u16::from(self.indirect) << 2)
                        | (u16::from(self.xr == 0) << 1)
                }
                7 => {
                    let s = scad.expect("SCAD for DISP/SCAD0");
                    u16::from(s & 0o1000 != 0) << 1
                }
                _ => return Err(self.unmodelled(format!("DISP group 10, input {sel}"))),
            };
        }
        let next = (get(mw, u::J) as u16 | (high << 4) | low | u16::from(skip_bit)) & 0o7777;
        if call {
            self.sp = (self.sp + 1) & 15;
            self.stack[usize::from(self.sp)] = self.upc;
        }
        if ret {
            self.sp = self.sp.wrapping_sub(1) & 15;
        }
        Ok(next)
    }
}

impl boot::Processor for Machine {
    fn bus(&mut self) -> (&mut [W36], &mut Io, u64) {
        (&mut self.memory, &mut self.io, self.instructions)
    }

    fn master_reset(&mut self) {
        if let Err(s) = self.console_reset() {
            self.pending_stop = Some(s);
        }
    }

    fn start_at(&mut self, addr: u32) {
        if self.pending_stop.is_none()
            && let Err(s) = self.console_start(addr)
        {
            self.pending_stop = Some(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cram::CRAM_WORDS;

    /// A word of the number field with the given bits (0 at the left).
    fn num(bits: &[u32]) -> u32 {
        bits.iter().fold(0, |a, b| a | 1 << (17 - b))
    }

    /// A cycle that starts with the number field `start` at address 100
    /// and completes in the next word with DP = 5. The word at 100 is 7.
    fn run_cycle(start: &[u32]) -> Machine {
        // MEM, the number field, DEST/A (Y is the A register), DBUS/DP,
        // RAMADR/VMA, no dispatch.
        let word = |n: u32, a: u32, j: u32| {
            Microword::from_bits(0)
                .with(u::MEM, 1)
                .with(u::DBUS, 1)
                .with(u::RAMADR, 4)
                .with(u::NUM, n)
                .with(u::A, a)
                .with(u::DISP, 0o70)
                .with(u::J, j)
        };
        let mut cram = vec![Microword::from_bits(0); CRAM_WORDS];
        cram[0] = word(num(start), 1, 1);
        cram[1] = word(num(&[16]), 3, 2);
        let mut m = Machine::new(cram, vec![0; DROM_WORDS], W40::ZERO);
        m.alu.reg[1] = alu::d_input(W36::wrap(0o100));
        m.alu.reg[3] = alu::d_input(W36::wrap(5));
        m.memory[0o100] = W36::wrap(7);
        m.step().unwrap();
        m.step().unwrap();
        m
    }

    // Number field bits: 03 READ CYCLE, 05 WRITE CYCLE, 08 PHYSICAL,
    // 14 LDVMA, 16 WAIT (ks10, "# REDEFINED WHEN USED AS MEMORY CYCLE
    // CONTROL").
    #[test]
    fn a_write_cycle_writes_dp_of_the_completing_word() {
        let m = run_cycle(&[5, 8, 14, 16]);
        assert_eq!(m.memory[0o100], W36::wrap(5));
    }

    #[test]
    fn a_cycle_with_read_and_write_reads_and_does_not_write() {
        // DPMA WRITE CYCLE needs NOT MEM READ (DPMA E17, E21).
        let m = run_cycle(&[3, 5, 8, 14, 16]);
        assert_eq!(m.memory[0o100], W36::wrap(7));
        assert_eq!(m.mb, W36::wrap(7));
    }

    #[test]
    fn rpw_cycle_starts_no_cycle() {
        // E33 holds READ and WRITE and VMA is not an AC: the next MEM EN
        // word starts nothing (DPM5 E32, E5).
        let mut cram = vec![Microword::from_bits(0); CRAM_WORDS];
        cram[0] = Microword::from_bits(0)
            .with(u::MEM, 1)
            .with(u::NUM, num(&[5, 8, 14, 16]))
            .with(u::A, 1)
            .with(u::DISP, 0o70)
            .with(u::J, 1);
        let mut m = Machine::new(cram, vec![0; DROM_WORDS], W40::ZERO);
        m.alu.reg[1] = alu::d_input(W36::wrap(0o100));
        m.vma = W36::wrap(0o200);
        m.vma_flags.physical = true;
        m.cycle = Cycle {
            read: true,
            write: true,
            ..Cycle::default()
        };
        m.step().unwrap();
        assert!(!m.memory_cycle);
        assert!(m.cycle.write && !m.cycle.read);
    }

    #[test]
    fn devices_and_the_clock_run_before_the_instruction_count_increases() {
        // ks10-core-soft polls the devices with the count of the steps
        // before it counts the next one (`Machine::step`).
        use ks10_machine::unibus::Event;
        let mut m = Machine::new(
            vec![Microword::from_bits(0); CRAM_WORDS],
            vec![0; DROM_WORDS],
            W40::ZERO,
        );
        m.instructions = INSTRUCTIONS_PER_MS;
        m.io.events = vec![
            (INSTRUCTIONS_PER_MS, Event::Dz),
            (INSTRUCTIONS_PER_MS + 1, Event::Dz),
        ];
        m.csl.one_ms_en = true;
        m.instruction_boundary();
        assert_eq!(m.io.events, [(INSTRUCTIONS_PER_MS + 1, Event::Dz)]);
        assert!(m.one_ms);
        assert_eq!(m.instructions, INSTRUCTIONS_PER_MS + 1);
    }

    #[test]
    fn dp_func_loads_the_io_flags_from_their_dp_bits() {
        // `VMA_[] WITH FLAGS` (ks10): number field bit 13 DP FUNC, 14
        // LDVMA. The microcode puts IO CYCLE, WRU CYCLE and VECTOR CYCLE
        // in DP 10, 11 and 12 (ks10, number field bits 10 to 12).
        for (bit, want) in [
            (10, [true, false, false]),
            (11, [false, true, false]),
            (12, [false, false, true]),
        ] {
            let mut cram = vec![Microword::from_bits(0); CRAM_WORDS];
            cram[0] = Microword::from_bits(0)
                .with(u::MEM, 1)
                .with(u::NUM, num(&[13, 14]))
                .with(u::A, 1)
                .with(u::DISP, 0o70)
                .with(u::J, 1);
            let mut m = Machine::new(cram, vec![0; DROM_WORDS], W40::ZERO);
            m.alu.reg[1] = alu::d_input(W36::wrap(1 << (35 - bit) | 0o100));
            m.step().unwrap();
            let f = m.vma_flags;
            assert_eq!([f.io, f.wru, f.vector], want, "DP bit {bit}");
        }
    }

    // The codes of the page fail dispatch come from the dispatch table
    // of the microcode, an independent source (itspag, `PFD`: 1
    // interrupt, 10 read-only page, 11 the 1 ms timer, 12 nonexistent
    // page, 13 exec/user mismatch).
    #[test]
    fn the_page_fail_dispatch_gives_the_codes_of_the_microcode() {
        let mut m = Machine::new(
            vec![Microword::from_bits(0); CRAM_WORDS],
            vec![0; DROM_WORDS],
            W40::ZERO,
        );
        m.apr_enables = W36::wrap(1 << (35 - 23)); // PAGE EN
        m.vma = W36::wrap(0o001000); // page 1, not an AC
        m.vma_flags = VmaFlags::default();
        let entry = |valid, writeable, user| {
            Some(PageEntry {
                valid,
                writeable,
                cacheable: false,
                user,
                page: 0,
            })
        };
        let mut code = |e: Option<PageEntry>, user: bool, write_test: bool| {
            m.page_table[1] = e;
            m.vma_flags.user = user;
            m.cycle.write_test = write_test;
            m.pf_disp().unwrap()
        };
        assert_eq!(code(entry(false, true, false), false, false), 0o12);
        assert_eq!(code(entry(true, true, true), false, false), 0o13);
        assert_eq!(code(entry(true, false, false), false, true), 0o10);
        assert_eq!(code(entry(true, true, false), false, true), 0o11);
        // An interrupt in a virtual read: INT OR ERR, code 1.
        m.memory_cycle = true;
        m.int_latched = true;
        m.cycle.write = false;
        assert_eq!(m.pf_disp().unwrap(), 0o1);
    }

    // The 7-bit bytes of the dispatch table of the microcode (simple,
    // `LDB1`): P = 29, 22, 15, 8, 1 give 1, 2, 4, 5, 7. Other pointers
    // give 0.
    #[test]
    fn the_byte_dispatch_gives_the_codes_of_the_microcode() {
        let ptr = |p: u64, s: u64| W36::wrap(p << 30 | s << 24);
        for (p, want) in [(29, 1), (22, 2), (15, 4), (8, 5), (1, 7)] {
            assert_eq!(byte_disp(ptr(p, 7)), want, "P {p}");
        }
        for (p, s) in [(36, 7), (30, 7), (29, 6), (29, 8), (1, 0o47)] {
            assert_eq!(byte_disp(ptr(p, s)), 0, "P {p} S {s}");
        }
    }

    // DBM BYTES: SCAD 01-07 in each of the five 7-bit bytes, from bit 00
    // (M8621-0-DPM1 and DPM2 rev B, input 1). Bit 35 is DP 35, which the
    // callers add; `bytes` gives 0 there.
    #[test]
    fn bytes_repeats_scad_01_to_07() {
        // SCAD 00 1, SCAD 01-07 = 1000001, SCAD 08-09 = 11.
        let scad = 0b11_0000_0111;
        assert_eq!(bytes(scad), W36::wrap(0o406030_140602));
    }

    // CACHE HIT (DPM7 E114): a read of a valid, cacheable page with CSL3
    // CACHE EN and a tag of VALID, VMA USER and VMA 18-26; each missing
    // term gives no hit.
    #[test]
    fn a_cache_hit_needs_every_term_of_e114() {
        let mut m = Machine::new(
            vec![Microword::from_bits(0); CRAM_WORDS],
            vec![0; DROM_WORDS],
            W40::ZERO,
        );
        m.vma = W36::wrap(0o146324);
        m.vma_flags = VmaFlags::default();
        m.cycle.read = true;
        m.csl.cache_en = true;
        let entry = PageEntry {
            valid: true,
            writeable: true,
            cacheable: true,
            user: false,
            page: 0o146,
        };
        let tag = CacheTag {
            valid: true,
            user: false,
            page: 0o146,
        };
        let k = 0o324 & 0o377; // VMA 27 is 0
        let mut hit = |e: PageEntry, t: CacheTag, read: bool, en: bool| {
            m.page_table[0o146] = Some(e);
            m.cache_dir[k] = Some(t);
            m.cycle.read = read;
            m.csl.cache_en = en;
            m.cache_hit().unwrap()
        };
        assert!(hit(entry, tag, true, true));
        assert!(!hit(
            PageEntry {
                cacheable: false,
                ..entry
            },
            tag,
            true,
            true
        ));
        assert!(!hit(
            entry,
            CacheTag {
                valid: false,
                ..tag
            },
            true,
            true
        ));
        assert!(!hit(entry, CacheTag { user: true, ..tag }, true, true));
        assert!(!hit(entry, CacheTag { page: 0o147, ..tag }, true, true));
        assert!(!hit(entry, tag, false, true));
        assert!(!hit(entry, tag, true, false));
    }
}

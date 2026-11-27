//! The machine: the microsequencer and the data path around the
//! arithmetic unit (docs/microcode-model.md, "The data path" and "Stage
//! 1").
//!
//! A function has one of two states: modelled, or a stop with
//! [`Stop::Unmodelled`]. The model never guesses a value.

use std::collections::BTreeSet;

use pdp10::W36;

use crate::alu::{self, Alu, Controls, Outputs, W40};
use crate::cram::Microword;
use crate::fields::{Field, u};
use crate::listing::DROM_WORDS;

/// The number of words of the RAM file (DPE7, DPE8).
pub const RAM_FILE_WORDS: usize = 1024;

/// The size of main memory in words: 512 Ki words, the largest KS10
/// memory (docs/architecture.md).
pub const MEMORY_WORDS: usize = 1 << 19;

/// Why the machine stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The microinstruction at `addr` needs a function that the model does
    /// not have yet.
    Unmodelled { addr: u16, what: String },
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

/// The console inputs and the HALT LOOP flag (M8616-0-CSL4 rev C). The
/// 8080 sets RUN, CONTINUE and EXECUTE through register 212, TRAP EN
/// through register 205 and 1 MSEC EN through register 100.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Console {
    pub run: bool,
    pub cont: bool,
    pub execute: bool,
    pub trap_en: bool,
    pub one_ms_en: bool,
    /// HALT LOOP (E161): the microcode sets and clears it.
    pub halt_loop: bool,
}

/// The codes that a run executed, for the coverage report.
#[derive(Clone, Debug, Default)]
pub struct Coverage {
    pub ad: BTreeSet<u32>,
    pub dest: BTreeSet<u32>,
    /// `SHSTYLE` with the direction: (style, left).
    pub shift: BTreeSet<(u32, bool)>,
    pub skip: BTreeSet<u32>,
    pub disp: BTreeSet<u32>,
    pub dbus: BTreeSet<u32>,
    pub dbm: BTreeSet<u32>,
    pub ramadr: BTreeSet<u32>,
    pub scad: BTreeSet<(u32, u32, u32)>,
    pub spec: BTreeSet<u32>,
}

impl Coverage {
    /// Add the codes of another run.
    pub fn merge(&mut self, o: &Coverage) {
        self.ad.extend(&o.ad);
        self.dest.extend(&o.dest);
        self.shift.extend(&o.shift);
        self.skip.extend(&o.skip);
        self.disp.extend(&o.disp);
        self.dbus.extend(&o.dbus);
        self.dbm.extend(&o.dbm);
        self.ramadr.extend(&o.ramadr);
        self.scad.extend(&o.scad);
        self.spec.extend(&o.spec);
    }
}

impl std::fmt::Display for Coverage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let oct = |s: &BTreeSet<u32>| {
            s.iter()
                .map(|v| format!("{v:o}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        writeln!(f, "AD: {}", oct(&self.ad))?;
        writeln!(f, "DEST: {}", oct(&self.dest))?;
        let sh: Vec<_> = self
            .shift
            .iter()
            .map(|(s, l)| format!("{s}{}", if *l { "L" } else { "R" }))
            .collect();
        writeln!(f, "SHSTYLE: {}", sh.join(" "))?;
        writeln!(f, "SKIP: {}", oct(&self.skip))?;
        writeln!(f, "DISP: {}", oct(&self.disp))?;
        writeln!(f, "DBUS: {}", oct(&self.dbus))?;
        writeln!(f, "DBM: {}", oct(&self.dbm))?;
        writeln!(f, "RAMADR: {}", oct(&self.ramadr))?;
        let sc: Vec<_> = self
            .scad
            .iter()
            .map(|(a, b, c)| format!("{a}/{b}/{c}"))
            .collect();
        writeln!(f, "SCAD/SCADA/SCADB: {}", sc.join(" "))?;
        write!(f, "SPEC: {}", oct(&self.spec))
    }
}

/// The microcode-level machine.
#[derive(Clone, Debug)]
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
    /// The DPE copy of VMA bits 26–35 (DPE5 E318, E324).
    pub vma_ram: u16,
    /// The cycle type (DPM5 E33) and MEMORY CYCLE (DPM6 E16 R0).
    pub cycle: Cycle,
    pub memory_cycle: bool,
    /// INT and 1MS as DPM6 E16 R3 and R2 latched them at MEM WAIT. INT is
    /// `None` if it depended on an APR interrupt request.
    pub int_latched: Option<bool>,
    pub one_ms_latched: bool,
    /// The memory buffer (DPM8, DPM9 receive latches).
    pub mb: W36,
    /// Main memory, by physical address.
    pub memory: Vec<W36>,
    pub pxct: Pxct,
    /// The DP word of the last `SPEC/APR EN` (DPMB E123, E132; DPEB E173).
    pub apr_enables: W36,
    /// The DP word of the last `SPEC/LDPI` (DPEB E175, E126, E140). The
    /// registers hold the complement of DP.
    pub pi: W36,
    /// INTERRUPT REQ (DPEB E167 R7), `None` if it depends on an APR
    /// interrupt request.
    pub interrupt_req: Option<bool>,
    /// The 1 ms flag (DPMC E1) and its copy (DPMC E3 R3).
    pub one_ms: bool,
    pub one_ms_sync: bool,
    pub console: Console,
    /// TRAP CYCLE (CRA2 E159).
    pub trap_cycle: bool,
    pub flags: PcFlags,
    pub cycles: u64,
    pub ticks: u64,
    pub coverage: Coverage,
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
    /// on `fill`. The cycle type of DPM5 E33 is the exception: it starts
    /// as no cycle.
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
            vma_ram: if p { 0o1777 } else { 0 },
            // Assumption: E33 holds no cycle at power on. No reset clears
            // it, and the power-up sequence depends on it
            // (docs/unknowns.md, "The microcode-level model").
            cycle: Cycle::default(),
            memory_cycle: p,
            int_latched: Some(p),
            one_ms_latched: p,
            mb: pw,
            memory: vec![alu::word(fill); MEMORY_WORDS],
            pxct: Pxct {
                on: p,
                bits: all(15),
            },
            apr_enables: pw,
            pi: pw,
            interrupt_req: Some(p),
            one_ms: p,
            one_ms_sync: p,
            console: Console::default(),
            trap_cycle: p,
            flags,
            cycles: 0,
            ticks: 0,
            coverage: Coverage::default(),
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

    /// INTERRUPT REQ for the next cycle (DPEB E147, E134, E148): the
    /// highest program request is above the highest level in progress.
    /// Stage 1 has no bus requests; an APR request makes it unknown.
    fn interrupt_request(&self) -> Option<bool> {
        if self.apr_enables.field(24, 31) != 0 {
            return None;
        }
        let dp = self.pi;
        let on = !dp.bit(28);
        let first = |base: u32| (1..=7).find(|n| !dp.bit(base + n));
        let request = first(10);
        let current = first(20);
        Some(on && request.is_some_and(|r| current.is_none_or(|c| r < c)))
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

    /// The SCAD output (DPM3) for the number field of `mw`.
    fn scad(&self, mw: Microword) -> Result<u16, Stop> {
        let a = match get(mw, u::SCADA) {
            0 => self.sc,
            1 => get(mw, u::S_NUM) as u16,
            other => return Err(self.unmodelled(format!("SCADA {other}"))),
        };
        let b = match get(mw, u::SCADB) {
            0 => self.fe,
            other => return Err(self.unmodelled(format!("SCADB {other}"))),
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
    pub fn step(&mut self) -> Result<(), Stop> {
        let mw = self.cram[usize::from(self.upc & 0o3777)];
        if get(mw, u::MULTI_SHIFT) != 0 {
            return Err(self.unmodelled("MULTI SHIFT"));
        }
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
        let flag_fl_02 = if multi_prec {
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
        let cov = &mut self.coverage;
        cov.ad.insert(ad);
        cov.dest.insert(dest);
        if dest >= 4 {
            cov.shift.insert((spec_low, dest & 2 == 0));
        }
        cov.spec.insert(spec);
        cov.skip.insert(get(mw, u::SKIP));
        cov.disp.insert(get(mw, u::DISP));

        // DP before the clock, where it does not depend on the DBus.
        let dp_early = self.alu.y_without_d(&c);
        let uses_d = alu::uses_d(c.src_left) || alu::uses_d(c.src_right);

        // Memory control before the clock: the registers hold the values
        // of the last load (docs/microcode-model.md, "Memory functions").
        let num = get(mw, u::NUM);
        let nb = |k: u32| (num >> (17 - k)) & 1 != 0;
        let mem = get(mw, u::MEM) != 0;
        let dw = self.drom_word();
        let drom_bit = |f| crate::listing::drom_field(dw, f) != 0;
        use crate::fields::d;
        // DPM5 E118, E8.
        let mem_en = mem && (nb(16) || (nb(17) && drom_bit(d::COND_FUNC)));
        let spec_mem_wait = spec_en & 1 != 0 && spec_low == 7;
        let spec_mem_clr = spec_en & 2 != 0 && spec_low == 7;
        let mem_wait = mem_en || spec_mem_wait || spec_mem_clr;
        // DPM5 E30: MEMORY DELAY, the word that completes a cycle.
        let memory_delay = (mem_en || spec_mem_wait) && self.memory_cycle;
        let ac_ref = self.ac_ref();
        // The page table, the cache hit and PAGED REF need PAGE EN and a
        // virtual VMA (DPM6 E47, DPM7 E114, DPM9 E38). Stage 1 stops if
        // they can act on a cycle.
        let page_en = self.apr_enables.bit(23);
        if page_en && !self.vma_flags.physical && (self.memory_cycle || mem_en) {
            return Err(self.unmodelled("paging (PAGE EN is 1)"));
        }
        let stop_main_memory = ac_ref;
        // DPM6 E15, E9: INT OR ERR makes a page fail.
        if self.memory_cycle && !self.vma_flags.physical && !self.cycle.write {
            let int = self
                .int_latched
                .ok_or_else(|| self.unmodelled("INT latched from an APR interrupt request"))?;
            if int || self.one_ms_latched {
                return Err(self.unmodelled("page fail: interrupt or 1 ms timer"));
            }
        }
        // DPM5 E32, E5: a read-pause-write cycle does not start a cycle.
        let rpw = self.cycle.read && self.cycle.write && !stop_main_memory;
        if mem_en && rpw {
            return Err(self.unmodelled("read-pause-write cycle"));
        }
        let bus_cycle = memory_delay && !stop_main_memory;
        if bus_cycle && (self.vma_flags.io || self.vma_flags.wru || self.vma_flags.vector) {
            return Err(self.unmodelled("I/O cycle"));
        }
        let address = self.bus_address();
        if bus_cycle && address >= self.memory.len() {
            return Err(self.unmodelled(format!("nonexistent memory {address:o}")));
        }
        // A read gives MB on DBM MEM. For an AC reference, FORCE RAMFILE
        // makes the DBus take the RAM file (DPM5 E14, DPE3 E70).
        if bus_cycle && self.cycle.read {
            self.mb = self.memory[address];
        }
        let force_ramfile = ac_ref && memory_delay && self.cycle.read;
        // DPMA E22, E54: the RAM file takes the DBus at the end of a
        // cycle: an AC write, or a memory cycle (the cache data).
        let ramfile_write =
            self.memory_cycle && mem_wait && (if ac_ref { !self.cycle.read } else { true });

        let fmwrite = get(mw, u::FMWRITE) != 0;
        let ir_load = spec_en & 4 != 0 && spec_low == 1;
        let xr_load = spec_en & 2 != 0 && spec_low == 1;
        let ram_write = fmwrite || ramfile_write;
        let dbus_needed = uses_d || ram_write || ir_load || xr_load;
        let dbus = if dbus_needed {
            Some(self.dbus(mw, dp_early, force_ramfile)?)
        } else {
            None
        };
        let ram_addr = if ram_write {
            self.coverage.ramadr.insert(get(mw, u::RAMADR));
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
            self.coverage
                .scad
                .insert((get(mw, u::SCAD), get(mw, u::SCADA), get(mw, u::SCADB)));
            Some(self.scad(mw)?)
        } else {
            None
        };

        self.check_spec(spec_en, spec_low)?;
        let pc_flags = spec_en & 4 != 0 && spec_low == 6;
        let ash_test = spec_en & 4 != 0 && spec_low == 4;
        let dbm_flags = if pc_flags {
            Some(self.dbm(mw, dp_early)?)
        } else {
            None
        };
        let vma_en = mem && (get(mw, u::LDVMA) != 0 || (nb(12) && drom_bit(d::VMA)));
        let previous_en = self.previous_en(mw);

        // The clock. The skip and dispatch conditions see F and DP of this
        // cycle, SC before LOADSC, and E603 before its load.
        let d_in = dbus.map(alu::d_input).unwrap_or(W40::ZERO);
        let out = self.alu.step(&c, d_in);
        let dp = alu::word(out.y);
        let jfcl = self.jfcl(dbm_flags);
        let next = self.next_address(mw, scad, &out, jfcl)?;
        if pc_flags || ash_test {
            self.flags = self.new_flags(dbm_flags, ash_test, &out, jfcl)?;
        }
        if let Some(addr) = ram_addr {
            self.ram_file[addr] = dbus.expect("the DBus of a RAM file write");
        }
        // A write takes DP of the word that completes the cycle (DPM8,
        // DPM9 MIX: DP in the data phase).
        if bus_cycle && self.cycle.write {
            self.memory[address] = dp;
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
        if mem_wait {
            // DPM5 E5 (SYNC is 1 at the clock) and DPM6 E16.
            let start = mem_en && !rpw && (self.cycle.read || self.cycle.write);
            self.memory_cycle = start;
            self.int_latched = self.interrupt_req;
            self.one_ms_latched = self.one_ms_sync;
        }
        if vma_en {
            self.load_vma(dp, nb, previous_en, spec_en, spec_low);
        }
        self.spec(mw, spec_en, spec_low, &out);
        // DPEB E167 R7 every cycle, from the PI registers before a load.
        let int_next = self.interrupt_request();
        if spec_en & 4 != 0 && spec_low == 3 {
            self.pi = dp;
        }
        self.interrupt_req = int_next;
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

    /// Load VMA and the VMA flags (DPM4 E55, E54, E74, E82, E90, E189).
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
                io: dp.bit(12),
                wru: dp.bit(11),
                vector: dp.bit(10),
                io_byte: dp.bit(13),
                extended: nb(15),
            }
        } else {
            let previous = previous_en || (spec_en & 2 != 0 && spec_low == 0);
            VmaFlags {
                user: (!nb(1) && !self.console.execute && user_flag)
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
        self.coverage.dbus.insert(sel);
        if force_ramfile && sel == 3 {
            sel = 2;
        }
        let dp_word = |m: &Self| {
            dp.map(alu::word)
                .ok_or_else(|| m.unmodelled("DP depends on the DBus, which selects DP"))
        };
        match sel {
            1 => dp_word(self),
            2 => {
                self.coverage.ramadr.insert(get(mw, u::RAMADR));
                let a = self.ram_address(mw, dp)?;
                Ok(self.ram_file[a])
            }
            3 => self.dbm(mw, dp),
            other => Err(self.unmodelled(format!("DBUS {other}"))),
        }
    }

    /// The DBM mixer (M8621-0-DPM1 and DPM2 rev B).
    fn dbm(&mut self, mw: Microword, dp: Option<W40>) -> Result<W36, Stop> {
        let dbm = get(mw, u::DBM);
        self.coverage.dbm.insert(dbm);
        let dp_word = |m: &Self| {
            dp.map(alu::word)
                .ok_or_else(|| m.unmodelled("DP depends on the DBus, which selects DP"))
        };
        match dbm {
            // SPEC 1 to 5 insert a byte; 6 and 7 are "SAME AS ZERO" (ks10,
            // "MICROCODE FIELDS -- SPEC", the DBM table).
            3 if matches!(get(mw, u::SPEC) & 7, 0 | 6 | 7) => dp_word(self),
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
    /// SPEC/PC FLAGS, `None` for ASH TEST.
    fn new_flags(
        &self,
        dbm: Option<W36>,
        ash_test: bool,
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
        // E209, E205, E107, E207, E206, E501, E208.
        let p = (ash_test && dp.bit(1) != dp.bit(2)) || (carry && overflow);
        g.ov = p || pick(0, f.ov) && !jfcl[0] || n(0);
        g.trap1 = p || (load && dp.bit(10)) || n(8);
        g.fov = (pick(3, f.fov) && !jfcl[3]) || n(1);
        g.fxu = pick(11, f.fxu);
        Ok(g)
    }

    /// Stop before the clock if a special function is not modelled.
    fn check_spec(&self, en: u32, low: u32) -> Result<(), Stop> {
        // Group 40 (DPE5 E102): 0 CRY 18 INH, 1 IR LOAD, 3 PI LOAD,
        // 4 ASH TEST, 6 PC FLAGS, 7 AC BLOCKS EN. EXP TEST needs the FXU
        // gate of DPE9 (E206), which is not read yet.
        if en & 4 != 0 && !matches!(low, 0 | 1 | 3 | 4 | 6 | 7) {
            return Err(self.unmodelled(format!("SPEC group 40, code {low}")));
        }
        // Group 20 (DPMA E78): 0 PREVIOUS, 1 XR LOAD, 5 APR EN, 6 PXCT OFF,
        // 7 MEM CLR.
        if en & 2 != 0 && !matches!(low, 0 | 1 | 5 | 6 | 7) {
            return Err(self.unmodelled(format!("SPEC group 20, code {low}")));
        }
        // Group 10 (CRA2 E97, DPMA E2): 0 CONSOLE, 1 CLR 1 MSEC, 5 NICOND,
        // 6 PXCT EN, 7 MEM WAIT.
        if en & 1 != 0 && !matches!(low, 0 | 1 | 5 | 6 | 7) {
            return Err(self.unmodelled(format!("SPEC group 10, code {low}")));
        }
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
        if en & 1 != 0 {
            let num = get(mw, u::NUM);
            let nb = |k: u32| (num >> (17 - k)) & 1 != 0;
            match low {
                // CONSOLE (CSL4 E171, E149, E135, E161).
                0 => {
                    let c = &mut self.console;
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
                // NICOND: TRAP CYCLE is NOT NICOND 09 (CRA2 E159).
                5 => self.trap_cycle = self.nicond() < 4,
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

    /// A 1 ms tick of the timer (DPMC E1): E1 takes 1 MSEC EN.
    pub fn millisecond(&mut self) {
        self.one_ms = self.console.one_ms_en;
    }

    /// NICOND 09–11 (CRA2 E147, DPE9 E125): the first active input of
    /// 0 EA1, 1 TRAP 3, 2 TRAP 2, 3 TRAP 1, 4 EF1, 5 not RUN, 6 EK1, 7.
    /// EA1, EF1 and EK1 are inactive (docs/microcode-model.md, "NICOND").
    fn nicond(&self) -> u16 {
        let trap_en = self.console.trap_en && self.apr_enables.bit(22);
        let f = self.flags;
        if trap_en && f.trap2 && f.trap1 {
            1
        } else if trap_en && f.trap2 {
            2
        } else if trap_en && f.trap1 {
            3
        } else if !self.console.run {
            5
        } else {
            7
        }
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
        use crate::fields::d;
        use crate::listing::drom_field;
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
                    7 => self.interrupt_req.ok_or_else(|| {
                        self.unmodelled("INTERRUPT REQ from an APR interrupt request")
                    })?,
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
                    4 => self.console.execute,
                    6 => !self.console.cont,
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

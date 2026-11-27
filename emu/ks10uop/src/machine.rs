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
    /// VMA (DPM4), as the DP word that loaded it.
    pub vma: W36,
    /// The DPE copy of VMA bits 26–35 (DPE5 E318, E324).
    pub vma_ram: u16,
    /// The DP words of functions that only store state in stage 1
    /// (docs/microcode-model.md, "Stage 1").
    pub apr_enables: Option<W36>,
    pub pi_load: Option<W36>,
    /// TRAP CYCLE (CRA2 E108).
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
    /// the content of the 2901 registers and the RAM file at power on,
    /// which the hardware does not define.
    pub fn new(cram: Vec<Microword>, drom: Vec<u64>, fill: W40) -> Self {
        assert_eq!(cram.len(), crate::cram::CRAM_WORDS);
        assert_eq!(drom.len(), DROM_WORDS);
        Machine {
            cram,
            drom,
            alu: Alu::new(fill),
            e603: Flags603::default(),
            ram_file: vec![alu::word(fill); RAM_FILE_WORDS],
            upc: 0,
            stack: [0; 16],
            sp: 0,
            sc: 0,
            fe: 0,
            ir: 0,
            ac: 0,
            xr: 0,
            indirect: false,
            cur_block: 0,
            prev_block: 0,
            vma: W36::ZERO,
            vma_ram: 0,
            apr_enables: None,
            pi_load: None,
            trap_cycle: false,
            flags: PcFlags::default(),
            cycles: 0,
            ticks: 0,
            coverage: Coverage::default(),
        }
    }

    fn unmodelled(&self, what: impl Into<String>) -> Stop {
        Stop::Unmodelled {
            addr: self.upc,
            what: what.into(),
        }
    }

    /// The RAM file address (DPE6) for a `RAMADR` code.
    fn ram_address(&self, mw: Microword) -> Result<usize, Stop> {
        let block = |b: u8| usize::from(b & 7) << 4;
        let n = get(mw, u::NUM) as usize;
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
            6 => usize::from(self.vma_ram & 0o1777),
            7 => n & 0o1777,
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
        let fmwrite = get(mw, u::FMWRITE) != 0;
        let ir_load = spec_en & 4 != 0 && spec_low == 1;
        let xr_load = spec_en & 2 != 0 && spec_low == 1;
        let dbus_needed = uses_d || fmwrite || ir_load || xr_load;
        let dbus = if dbus_needed {
            Some(self.dbus(mw, dp_early)?)
        } else {
            None
        };
        let ram_addr = if fmwrite {
            self.coverage.ramadr.insert(get(mw, u::RAMADR));
            Some(self.ram_address(mw)?)
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
        let mem = get(mw, u::MEM) != 0;
        if mem {
            self.check_memory_function(mw)?;
        }

        // The clock. The skip and dispatch conditions see F and DP of this
        // cycle, SC before LOADSC, and E603 before its load.
        let d = dbus.map(alu::d_input).unwrap_or(W40::ZERO);
        let out = self.alu.step(&c, d);
        let jfcl = self.jfcl(dbm_flags);
        let next = self.next_address(mw, scad, &out, jfcl)?;
        if pc_flags || ash_test {
            self.flags = self.new_flags(dbm_flags, ash_test, &out, jfcl)?;
        }
        if let Some(addr) = ram_addr {
            self.ram_file[addr] = dbus.expect("the DBus of an FMWRITE");
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
            // check_spec stops before an XR load: XR PREV has no source yet.
            let w = dbus.expect("the DBus of an XR load");
            self.indirect = w.bit(13);
            self.xr = w.field(14, 17) as u8;
        }
        self.spec(spec_en, spec_low, &out);
        if mem && get(mw, u::LDVMA) != 0 {
            // VMA gets DP; DPE5 keeps a copy of bits 26–35 for the RAM
            // file address (DPE5 E318, E324).
            self.vma = alu::word(out.y);
            self.vma_ram = self.vma.field(26, 35) as u16;
        }
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

    /// The DBus (DPE3, DPE4) and the DBM mixer (DPM1, DPM2).
    fn dbus(&mut self, mw: Microword, dp: Option<W40>) -> Result<W36, Stop> {
        let sel = get(mw, u::DBUS);
        self.coverage.dbus.insert(sel);
        let dp_word = |m: &Self| {
            dp.map(alu::word)
                .ok_or_else(|| m.unmodelled("DP depends on the DBus, which selects DP"))
        };
        match sel {
            1 => dp_word(self),
            2 => {
                self.coverage.ramadr.insert(get(mw, u::RAMADR));
                Ok(self.ram_file[self.ram_address(mw)?])
            }
            3 => self.dbm(mw, dp),
            other => Err(self.unmodelled(format!("DBUS {other}"))),
        }
    }

    /// The DBM mixer (DPM1, DPM2).
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
        // Group 20 (DPE5 E101, DPMA E411): 5 APR EN, 7 MEM CLR. XR LOAD
        // also loads XR PREV from DPMA PREVIOUS EN, which is not read yet.
        if en & 2 != 0 && !matches!(low, 5 | 7) {
            return Err(self.unmodelled(format!("SPEC group 20, code {low}")));
        }
        // Group 10 (DPMA E410, CRA2 E113): none yet.
        if en & 1 != 0 {
            return Err(self.unmodelled(format!("SPEC group 10, code {low}")));
        }
        Ok(())
    }

    /// The special functions after the clock.
    fn spec(&mut self, en: u32, low: u32, out: &Outputs) {
        let dp = alu::word(out.y);
        if en & 4 != 0 {
            match low {
                // PI LOAD: stage 1 has no priority interrupts; it keeps DP.
                3 => self.pi_load = Some(dp),
                // AC BLOCKS EN: DP 06–08 to the current block, DP 09–11 to
                // the previous block (DPE5 E308).
                7 => {
                    self.cur_block = dp.field(6, 8) as u8;
                    self.prev_block = dp.field(9, 11) as u8;
                }
                _ => {}
            }
        }
        if en & 2 != 0 && low == 5 {
            // APR EN: stage 1 has no APR conditions; it keeps DP.
            self.apr_enables = Some(dp);
        }
        // MEM CLR does not change the state of stage 1: it has no page
        // fail.
    }

    /// Stop before the clock if a memory function is not modelled. Stage 1
    /// so far loads VMA only.
    fn check_memory_function(&self, mw: Microword) -> Result<(), Stop> {
        let cycle = [
            u::READ_CYCLE,
            u::WRITE_TEST,
            u::WRITE_CYCLE,
            u::AREAD,
            u::DP_FUNC,
            u::WAIT,
            u::BWRITE,
        ];
        if let Some(f) = cycle.iter().find(|f| get(mw, **f) != 0) {
            return Err(self.unmodelled(format!("memory function {}", f.name)));
        }
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

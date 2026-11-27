//! The machine state and the instruction cycle of the software processor.
//!
//! Every reference of an instruction goes through [`Machine::vread`] and
//! [`Machine::vwrite`]: locations 0 to 17 are the ACs, other addresses go
//! through the pager when it is enabled (see `pager`). A reference has a
//! kind ([`Ref`]); under XCTR, the AC field of the XCTR selects the kinds
//! that use the previous context (ks10.47, PXCT field: E1, D1, E2, D2;
//! ksdefs.194, XCTR bits).
//!
//! The order of one step follows the microcode **[microcode]**:
//! interrupts (inout.50, PI), then the trap of a trap flag (simple.42,
//! NICOND; inout.50, TRAP), then the one-proceed trap (simple.42, XCTGO,
//! 1PROC-TRAP), then the instruction. A page failure delivers the page fail
//! trap (itspag.98, PFTRAP).

use pdp10::instructions::{self as ins, Class};
use pdp10::{Half, W36};

use ks10_machine::MEMORY_WORDS;
use ks10_machine::console::Console;
use ks10_machine::unibus::Io;

use crate::apr::{Apr, Pi, flag};
use crate::flags::Flags;
use crate::ops;
use crate::pager::{self, Pager, pfw};
use crate::timer::{self, Timer};

/// Halt codes (ks10.47, HALT field).
pub mod halt {
    pub const HALT: u32 = 0o1;
    pub const IOPF: u32 = 0o100;
    pub const ILLII: u32 = 0o101;
    pub const ILLINT: u32 = 0o102;
}

/// Why the processor stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The processor halted with a halt code (`halt`): 1 for a HALT
    /// instruction in executive mode (the PC is then its effective
    /// address), 100 for a page failure in an interrupt instruction, 101
    /// for an illegal interrupt instruction.
    Halt { pc: Half, code: u32 },
    /// An instruction that this version does not implement.
    Unimplemented { pc: Half, word: W36 },
    /// The step limit of [`Machine::run`] was reached.
    Limit,
}

/// The kind of a memory reference, for XCTR (ks10.47, PXCT field).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ref {
    /// Instruction fetch: always the current context.
    Fetch,
    /// The effective address calculation of the instruction (PXCT E1).
    E1,
    /// The data at E (PXCT D1; also the BLT destination).
    D1,
    /// The effective address of a byte pointer or of EXTEND (PXCT E2).
    E2,
    /// Byte data, stack words, the BLT source (PXCT D2).
    D2,
    /// UMOVE, UMOVEM: always the previous context (inout.50, SPEC/PREV).
    Prev,
}

/// The one-proceed state (simple.42, FLG.1PROC and FLG.2PROC).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OneProc {
    #[default]
    Off,
    /// A JRSTF loaded the one-proceed bit: the next instruction runs.
    Armed,
    /// That instruction ran: trap before the next one.
    Running,
}

/// One KS10: the processor state and the memory. Devices come later.
pub struct Machine {
    pub mem: Vec<W36>,
    /// The current AC block (fast memory, locations 0 to 17).
    pub ac: [W36; 16],
    pub pc: Half,
    pub flags: Flags,
    pub apr: Apr,
    pub pi: Pi,
    /// Completed steps (the virtual time, docs/emulator.md).
    pub count: u64,
    /// The EBR register as the microcode keeps it (inout.50, WREBR): the
    /// WREBR operand shifted left 9 places, bit 0 set for "Tops-20 style".
    pub ebr: W36,
    /// The UBR register (inout.50, WRUBR, the ITS version): bits 0 and 2
    /// as last written, the AC blocks in bits 6 to 11, the physical address
    /// of the UPT in bits 16 to 35.
    pub ubr: W36,
    /// The pager and the traps are enabled (ks10.47, WORK[APR] bits 2.5
    /// and 2.4; set by WREBR).
    pub paging: bool,
    pub pager: Pager,
    /// The 8 AC blocks. The current block is in `ac`; its slot here is
    /// out of date until the block changes.
    pub blocks: [[W36; 16]; 8],
    /// The current AC block, the number of the block in `ac`.
    pub cur_block: u8,
    /// The PC sample table pointer (inout.50, WORK[PCST]).
    pub pcst: W36,
    pub console: Console,
    pub timer: Timer,
    /// The ITS page table pointers DBR1 to DBR4 (inout.50, LDBRn).
    pub dbr: [W36; 4],
    /// The halt status block address (inout.50, WRHSB). The ITS microcode
    /// starts with 500 (simple.42, at location 3).
    pub hsb: W36,
    /// The PXCT bits of the XCTR in progress (0 if none).
    pub pxct: u8,
    pub oneproc: OneProc,
    /// The I/O bus: adapters and devices.
    pub io: Io,
    /// The timer ticks come from [`Machine::tick`], which the host calls
    /// from the wall clock, and not from the instruction count. The run
    /// then depends on the host and cannot be repeated (`neoks10 run
    /// --pace normal`).
    pub wall_clock: bool,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

/// The 8080 boot commands (`ks10_machine::boot`) on this processor.
impl ks10_machine::boot::Processor for Machine {
    fn bus(&mut self) -> (&mut [W36], &mut Io, u64) {
        (&mut self.mem, &mut self.io, self.count)
    }

    fn master_reset(&mut self) {
        self.reset();
    }

    fn start_at(&mut self, addr: u32) {
        self.pc = Half::wrap(u64::from(addr));
    }
}

/// The parts of an instruction word (PRM 1.6: bits 0 to 8 opcode, 9 to 12
/// AC, 13 indirect, 14 to 17 index, 18 to 35 address).
#[derive(Clone, Copy, Debug)]
pub struct Inst {
    pub word: W36,
    pub op: u16,
    pub ac: usize,
}

impl Inst {
    pub fn new(word: W36) -> Self {
        Inst {
            word,
            op: word.field(0, 8) as u16,
            ac: word.field(9, 12) as usize,
        }
    }
}

/// What an instruction does to the PC.
pub enum Next {
    /// Continue with PC+1.
    Step,
    /// Skip: continue with PC+2.
    Skip,
    /// Jump to the address.
    Jump(Half),
    /// Execute this instruction word now (XCT), at the same PC.
    Execute(W36),
    /// Execute this word with these PXCT bits (XCTR, XCTRI).
    Xctr(W36, u8),
    /// Stop the processor.
    Stop(Stop),
}

/// An instruction that does not complete normally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The emulator does not have this instruction yet.
    Unimplemented,
    /// The microcode does a monitor UUO (an illegal or monitor call).
    Uuo,
    /// A page failure with its page fail word.
    PageFail(W36),
}

/// The limit of indirect words in one effective address calculation, and
/// of XCTs in one step. The real processor has no limit; it can take an
/// interrupt during a long chain. Here a longer chain ends the step without
/// a result, and the next step does the instruction again, so that a guest
/// loop cannot block the emulator.
const MAX_INDIRECT: u32 = 4096;

impl Machine {
    pub fn new() -> Self {
        Machine {
            mem: vec![W36::ZERO; MEMORY_WORDS],
            ac: [W36::ZERO; 16],
            pc: Half::ZERO,
            flags: Flags::default(),
            apr: Apr::default(),
            pi: Pi::default(),
            count: 0,
            ebr: W36::ZERO,
            ubr: W36::ZERO,
            paging: false,
            pager: Pager::default(),
            blocks: [[W36::ZERO; 16]; 8],
            cur_block: 0,
            pcst: W36::ZERO,
            console: Console::default(),
            timer: Timer::default(),
            dbr: [W36::ZERO; 4],
            hsb: W36::wrap(0o500),
            pxct: 0,
            oneproc: OneProc::Off,
            io: Io::default(),
            wall_clock: false,
        }
    }

    /// A master reset and the start of the microcode (8080 MR command;
    /// simple.42 at location 3): the processor state, the pager, the
    /// timers and the Unibus adapters and devices go to their initial
    /// state. Memory, the ACs and the disk attachments stay.
    pub fn reset(&mut self) {
        self.pc = Half::ZERO;
        self.flags = Flags::default();
        self.apr = Apr::default();
        self.pi = Pi::default();
        self.ebr = W36::ZERO;
        self.ubr = W36::ZERO;
        self.paging = false;
        self.pager.clear_all();
        self.load_ac_blocks();
        self.pcst = W36::ZERO;
        self.timer = Timer {
            last_tick: self.count,
            ..Timer::default()
        };
        self.dbr = [W36::ZERO; 4];
        self.hsb = W36::wrap(0o500);
        self.pxct = 0;
        self.oneproc = OneProc::Off;
        self.io.uba = Default::default();
        self.io.disk.reset();
        self.io.dz.reset();
        self.io.chaos.reset();
        self.io.imp.reset();
        self.io.events.clear();
        self.io.memstat = 0;
    }

    /// The physical address of a word of the EPT: the EBR plus the
    /// offset, 20 bits (inout.50, "[AR]_[AR]+[EBR]").
    pub fn ept_addr(&self, offset: u32) -> u32 {
        phys(self.ebr, offset)
    }

    /// The physical address of a word of the UPT (simple.42, UUOGO,
    /// "[ARX]_[ARX]+[UBR]").
    pub fn upt_addr(&self, offset: u32) -> u32 {
        phys(self.ubr, offset)
    }

    /// LOAD AC BLOCKS: make the block in UBR bits 6 to 8 current.
    pub fn load_ac_blocks(&mut self) {
        let new = self.ubr.field(6, 8) as u8;
        if new != self.cur_block {
            self.blocks[usize::from(self.cur_block)] = self.ac;
            self.ac = self.blocks[usize::from(new)];
            self.cur_block = new;
        }
    }

    /// The previous AC block (UBR bits 9 to 11).
    fn prev_block(&self) -> u8 {
        self.ubr.field(9, 11) as u8
    }

    /// AC k of the current block, or of the previous block.
    pub fn ac_get(&self, k: usize, prev: bool) -> W36 {
        let b = self.prev_block();
        if !prev || b == self.cur_block {
            self.ac[k]
        } else {
            self.blocks[usize::from(b)][k]
        }
    }

    fn ac_set(&mut self, k: usize, prev: bool, w: W36) {
        let b = self.prev_block();
        if !prev || b == self.cur_block {
            self.ac[k] = w;
        } else {
            self.blocks[usize::from(b)][k] = w;
        }
    }

    /// User mode and AC block of a reference: the previous context if XCTR
    /// selects this kind; the previous context is user if PCU (flag bit
    /// 6 in executive mode) is set.
    fn context(&self, r: Ref) -> (bool, bool) {
        let bit = match r {
            Ref::Fetch => 0,
            Ref::E1 => 0o10,
            Ref::D1 => 0o4,
            Ref::E2 => 0o2,
            Ref::D2 => 0o1,
            Ref::Prev => 0o17,
        };
        if r == Ref::Prev || self.pxct & bit != 0 {
            (self.flags.has(Flags::USERIO), true)
        } else {
            (self.flags.is_user(), false)
        }
    }

    /// A physical read; a nonexistent address is a hard page failure.
    pub fn phys_read(&self, a: u32) -> Result<W36, Fault> {
        self.mem
            .get(a as usize)
            .copied()
            .ok_or(Fault::PageFail(W36::wrap(
                pfw::NO_MEMORY | pfw::PHYSICAL | u64::from(a),
            )))
    }

    pub fn phys_write(&mut self, a: u32, w: W36) -> Result<(), Fault> {
        match self.mem.get_mut(a as usize) {
            Some(x) => {
                *x = w;
                Ok(())
            }
            None => Err(Fault::PageFail(W36::wrap(
                pfw::NO_MEMORY | pfw::PHYSICAL | u64::from(a),
            ))),
        }
    }

    /// The physical address of a memory reference (not an AC).
    fn map(&mut self, a: Half, user: bool, write: bool) -> Result<usize, Fault> {
        let p = pager::translate(self, a, user, write)? as usize;
        if p < self.mem.len() {
            Ok(p)
        } else {
            Err(Fault::PageFail(pager::fail_word(
                a,
                user,
                write,
                pfw::NO_MEMORY,
            )))
        }
    }

    pub fn vread(&mut self, a: Half, r: Ref) -> Result<W36, Fault> {
        let (user, prev) = self.context(r);
        let k = a.get() as usize;
        if k < 16 {
            return Ok(self.ac_get(k, prev));
        }
        let p = self.map(a, user, false)?;
        Ok(self.mem[p])
    }

    pub fn vwrite(&mut self, a: Half, w: W36, r: Ref) -> Result<(), Fault> {
        let (user, prev) = self.context(r);
        let k = a.get() as usize;
        if k < 16 {
            self.ac_set(k, prev, w);
            return Ok(());
        }
        let p = self.map(a, user, true)?;
        self.mem[p] = w;
        Ok(())
    }

    /// Check a reference without doing it (the AREAD of the microcode
    /// starts the memory cycle of E before the instruction runs).
    fn touch(&mut self, a: Half, write: bool) -> Result<(), Fault> {
        let (user, _) = self.context(Ref::D1);
        if a.get() >= 16 {
            self.map(a, user, write)?;
        }
        Ok(())
    }

    /// Read the data at E (PXCT D1).
    pub fn read(&mut self, a: Half) -> Result<W36, Fault> {
        self.vread(a, Ref::D1)
    }

    pub fn write(&mut self, a: Half, w: W36) -> Result<(), Fault> {
        self.vwrite(a, w, Ref::D1)
    }

    /// Physical memory, for the console and the loader.
    pub fn deposit(&mut self, addr: u32, w: W36) -> Result<(), u32> {
        let m = self.mem.get_mut(addr as usize).ok_or(addr)?;
        *m = w;
        Ok(())
    }

    /// The effective address calculation of an instruction (PRM 1.6).
    pub fn ea(&mut self, w: W36) -> Result<Option<(Half, W36)>, Fault> {
        self.ea_ref(w, Ref::E1)
    }

    /// The effective address calculation of a word with I, X and Y, with
    /// the index registers and indirect words of reference kind `r`.
    /// Returns E and the last word of the calculation (JRSTF takes the
    /// flags from it), or `None` after MAX_INDIRECT indirect words.
    pub fn ea_ref(&mut self, mut w: W36, r: Ref) -> Result<Option<(Half, W36)>, Fault> {
        let (_, prev) = self.context(r);
        for _ in 0..=MAX_INDIRECT {
            let x = w.field(14, 17) as usize;
            let y = w.field(18, 35);
            let xr = self.ac_get(x, prev);
            let e = if x != 0 {
                Half::wrap(y + xr.rh().get())
            } else {
                Half::wrap(y)
            };
            if !w.bit(13) {
                return Ok(Some((e, if x != 0 { xr } else { w })));
            }
            w = self.vread(e, r)?;
        }
        Ok(None)
    }

    /// The interrupt requests of the APR (and later the devices) as a
    /// mask of levels.
    fn requests(&self) -> u8 {
        let apr = if self.apr.requesting() && self.apr.level != 0 {
            Pi::level_bit(self.apr.level)
        } else {
            0
        };
        apr | self.io.requests()
    }

    /// The PC word to save at an interrupt or a page failure, with the
    /// one-proceed bit if an instruction is being one-proceeded
    /// (itspag.98, STORE-INT-PC); that ends the one-proceed state.
    fn int_pc_word(&mut self) -> W36 {
        let mut w = self.flags.pc_word(self.pc);
        if self.oneproc != OneProc::Off {
            w = W36::wrap(w.get() | (u64::from(Flags::OIP) << 18));
            self.oneproc = OneProc::Off;
        }
        w
    }

    /// GOEXEC (simple.42): new flags and PC from a word. LEAVE USER: the
    /// word may clear User. LOAD PCU: in executive mode the previous
    /// context is the old mode.
    pub fn goexec(&mut self, w: W36) {
        let old_user = self.flags.is_user();
        let mut f = Flags((w.lh().get() as u32) & Flags::ALL);
        if !f.is_user() {
            if old_user {
                f.set(Flags::USERIO);
            } else {
                f.clear(Flags::USERIO);
            }
        }
        self.flags = f;
        self.pc = w.rh();
    }

    /// The processor halts (inout.50, HALTED, WRTHSB, DUMP): the halt
    /// status block if its address is positive, then the halt code in
    /// location 0 and the PC in location 1.
    pub fn halt(&mut self, code: u32, hr: W36) -> Stop {
        let a = self.hsb;
        if a.signed() > 0 {
            // The 2901 registers that are temporaries of the microcode (AR,
            // ARX, BR, BRX, FLG, T0, VMA) are not modelled; they are 0
            // here (docs/unknowns.md).
            let block = [
                W36::wrap(0o377777_777777), // MAG
                W36::from_halves(Half::ZERO, self.pc),
                hr,
                W36::ZERO,
                W36::ZERO,
                W36::ZERO,
                W36::ZERO,
                W36::wrap(1), // ONE
                self.ebr,
                self.ubr,
                W36::wrap(0o777777_777777), // MASK
                W36::ZERO,
                self.pi.word(),
                W36::from_halves(Half::wrap(1), Half::wrap(1)), // XWD1
                W36::ZERO,
                W36::wrap(u64::from(code)), // T1
                W36::ZERO,
            ];
            for (k, w) in block.iter().enumerate() {
                let _ = self.deposit(a.get() as u32 + k as u32, *w);
            }
        }
        let _ = self.deposit(0, W36::wrap(u64::from(code)));
        let _ = self.deposit(1, W36::from_halves(Half::ZERO, self.pc));
        Stop::Halt { pc: self.pc, code }
    }

    /// Deliver a page failure (itspag.98, PFTRAP, PFTDSP, PFTRAP1): the
    /// three words at EPT + 440 if no interrupt is in progress, else at
    /// 443, 446, ..., 465 for the highest level in progress: the page fail
    /// word, the PC word, the new PC word.
    fn page_fail(&mut self, w: W36) -> Result<(), Stop> {
        let off = match (1..=7u8).find(|&n| self.pi.in_progress & Pi::level_bit(n) != 0) {
            None => 0o440,
            Some(n) => 0o440 + 3 * u32::from(n),
        };
        let a = self.ept_addr(off);
        let pcw = self.int_pc_word();
        let ok = self.phys_write(a, w).is_ok() && self.phys_write(a + 1, pcw).is_ok();
        match self.phys_read(a + 2) {
            Ok(new) if ok => {
                self.goexec(new);
                Ok(())
            }
            _ => Err(self.halt(halt::IOPF, W36::ZERO)),
        }
    }

    /// Take an interrupt on level n **[microcode]** (inout.50, PI to PI50,
    /// PIJSR, PIXPCW; PRM 4.1): hold the level, then execute the
    /// instruction at EPT + 40 + 2n in executive mode. It must be JSR or
    /// XPCW (JRST 7,); else the processor halts with code 101 (ILLII). A
    /// JSR stores the PC word of the interrupted program at E, clears all
    /// flags and continues at E+1. A page failure halts with code 100
    /// (PFT: "IO Page Failure"). Unibus vectors come with the devices.
    fn interrupt(&mut self, n: u8) -> Result<(), Stop> {
        self.pi.in_progress |= Pi::level_bit(n);
        let loc = self.ept_addr(0o40 + 2 * u32::from(n));
        // The WRU and vector cycles (inout.50, PI10, VECINT): a Unibus
        // device gives a vector; EPT + 100 + adapter points to the table
        // of the adapter, and the instruction is at table + vector / 4 in
        // the executive address space.
        let vectored = match self.io.acknowledge(n) {
            None => None,
            Some((a, v)) => {
                let t = self
                    .phys_read(self.ept_addr(0o100 + a))
                    .unwrap_or(W36::ZERO);
                if t.rh() == Half::ZERO {
                    return Err(self.halt(halt::ILLINT, W36::ZERO));
                }
                Some(Half::wrap(t.get().wrapping_add(u64::from(v >> 2))))
            }
        };
        let old = self.flags;
        let pc = self.pc;
        let pcw = self.int_pc_word();
        // The interrupt instruction runs in executive mode (FORCE EXEC).
        self.flags = Flags(0);
        let r = (|| -> Result<Option<(W36, Half)>, Fault> {
            let word = match vectored {
                Some(a) => self.vread(a, Ref::D1)?,
                None => self.phys_read(loc)?,
            };
            let Some((e, _)) = self.ea(word)? else {
                return Ok(None);
            };
            Ok(Some((word, e)))
        })();
        let (word, e) = match r {
            Ok(Some(x)) => x,
            Ok(None) => return Ok(()),
            Err(_) => return Err(self.halt(halt::IOPF, W36::ZERO)),
        };
        if word.field(0, 8) == 0o264 {
            if self.write(e, pcw).is_err() {
                return Err(self.halt(halt::IOPF, word));
            }
            self.pc = Half::wrap(e.get() + 1);
            Ok(())
        } else if word.field(0, 12) == 0o254 << 4 | 7 {
            let f = Flags((pcw.lh().get() as u32) & Flags::ALL);
            match ops::xpcw(self, e, f, pc) {
                Ok((f, p)) => {
                    self.flags = f;
                    self.pc = p;
                    Ok(())
                }
                Err(_) => Err(self.halt(halt::IOPF, word)),
            }
        } else {
            self.flags = old;
            Err(self.halt(halt::ILLII, word))
        }
    }

    /// The trap of a trap flag (simple.42, NICOND; inout.50, TRAP, TRP1):
    /// the instruction at UPT (user mode) or EPT + 421, 422 or 423, read
    /// physically, runs in the current mode with the trap flags cleared,
    /// as if it were at the location before PC (so that it returns to PC
    /// unless it jumps or skips). An opcode below 100 is a monitor UUO,
    /// with the trap flags still set (so it takes the trap UUO vector).
    fn trap_cycle(&mut self) -> Result<(), Stop> {
        let t =
            (self.flags.has(Flags::TRAP1) as u32) | ((self.flags.has(Flags::TRAP2) as u32) << 1);
        let loc = if self.flags.is_user() {
            self.upt_addr(0o420 + t)
        } else {
            self.ept_addr(0o420 + t)
        };
        let trappc = self.flags.pc_word(self.pc);
        let word = match self.phys_read(loc) {
            Ok(w) => w,
            Err(Fault::PageFail(w)) => return self.page_fail(w),
            Err(_) => return Ok(()),
        };
        let base = Half::wrap(self.pc.get().wrapping_sub(1));
        if word.field(0, 2) == 0 {
            self.pc = base;
            let inst = Inst::new(word);
            let r = match self.ea(word) {
                Ok(Some((e, _))) => ops::muuo(self, inst, e),
                Ok(None) => Ok(()),
                Err(f) => Err(f),
            };
            self.count += 1;
            return match r {
                Ok(()) => Ok(()),
                Err(Fault::PageFail(w)) => {
                    self.restore_trap_pc(trappc);
                    self.page_fail(w)
                }
                Err(_) => Err(Stop::Unimplemented { pc: base, word }),
            };
        }
        self.flags.clear(Flags::TRAP1 | Flags::TRAP2);
        self.pc = base;
        self.run_inst(word, base, Some(trappc))
    }

    /// After a page failure in a trap cycle, the PC word is the one before
    /// the trap (itspag.98, CLTRP: "This is a trap cycle").
    fn restore_trap_pc(&mut self, trappc: W36) {
        self.flags = Flags((trappc.lh().get() as u32) & Flags::ALL);
        self.pc = trappc.rh();
    }

    /// The one-proceed trap (simple.42, 1PROC-TRAP): the PC word to UPT +
    /// 432, new flags and PC from UPT + 433.
    fn oneproc_trap(&mut self) -> Result<(), Stop> {
        self.oneproc = OneProc::Off;
        let a = self.upt_addr(0o432);
        let pcw = self.flags.pc_word(self.pc);
        self.count += 1;
        let r = self.phys_write(a, pcw).and_then(|()| self.phys_read(a + 1));
        match r {
            Ok(new) => {
                self.goexec(new);
                Ok(())
            }
            Err(Fault::PageFail(w)) => self.page_fail(w),
            Err(_) => Ok(()),
        }
    }

    /// The 8080 console works between two instructions.
    fn console(&mut self) {
        let mem = &self.mem;
        let word = |a: u32| mem[a as usize];
        let Some(e) = self.console.poll(self.count, word) else {
            return;
        };
        if let Some(a) = e.clear {
            self.mem[a as usize] = W36::ZERO;
        }
        if let Some(w) = e.input {
            self.mem[ks10_machine::console::CTYIN as usize] = w;
        }
        if e.interrupt {
            self.apr.flags |= flag::FROM_8080;
        }
    }

    /// Do one instruction, or take an interrupt or a trap. `Ok(())` if the
    /// processor can continue.
    pub fn step(&mut self) -> Result<(), Stop> {
        timer::run(self);
        self.console();
        self.io.poll(self.count);
        if !self.io.events.is_empty() {
            self.io.run_events(&mut self.mem, self.count);
        }
        if let Some(n) = self.pi.accept(self.requests()) {
            // An interrupt at the fetch after a one-proceeded instruction
            // takes the one-proceed trap first (itspag.98, CLTRP1, CLTRP2).
            if self.oneproc == OneProc::Running {
                return self.oneproc_trap();
            }
            self.count += 1;
            return self.interrupt(n);
        }
        if self.paging && self.flags.0 & (Flags::TRAP1 | Flags::TRAP2) != 0 {
            return self.trap_cycle();
        }
        if self.oneproc == OneProc::Running {
            return self.oneproc_trap();
        }
        let pc = self.pc;
        let word = match self.vread(pc, Ref::Fetch) {
            Ok(w) => w,
            Err(Fault::PageFail(w)) => {
                self.count += 1;
                return self.page_fail(w);
            }
            Err(_) => return Ok(()),
        };
        if self.oneproc == OneProc::Armed {
            self.oneproc = OneProc::Running;
        }
        self.run_inst(word, pc, None)
    }

    /// Execute `word` as the instruction at `pc` (with its XCT chain).
    fn run_inst(&mut self, mut word: W36, pc: Half, trappc: Option<W36>) -> Result<(), Stop> {
        self.count += 1;
        let r = (|| -> Result<Next, Fault> {
            for _ in 0..=MAX_INDIRECT {
                let inst = Inst::new(word);
                let Some((e, last)) = self.ea(word)? else {
                    break;
                };
                match self.execute(inst, e, last)? {
                    Next::Execute(w) => word = w,
                    Next::Xctr(w, bits) => {
                        word = w;
                        self.pxct = bits;
                    }
                    n => return Ok(n),
                }
            }
            // A chain without end: do it again in the next step.
            Ok(Next::Jump(pc))
        })();
        self.pxct = 0;
        match r {
            Ok(Next::Step) => self.pc = Half::wrap(pc.get() + 1),
            Ok(Next::Skip) => self.pc = Half::wrap(pc.get() + 2),
            Ok(Next::Jump(t)) => self.pc = t,
            Ok(Next::Execute(_) | Next::Xctr(..)) => {}
            Ok(Next::Stop(s)) => return Err(s),
            Err(Fault::PageFail(w)) => {
                if let Some(t) = trappc {
                    self.restore_trap_pc(t);
                } else {
                    self.pc = pc;
                }
                return self.page_fail(w);
            }
            Err(Fault::Unimplemented | Fault::Uuo) => {
                return Err(Stop::Unimplemented { pc, word });
            }
        }
        Ok(())
    }

    /// The idle loop, if the processor is in it: the AC that the loop
    /// counts in, and the value of that AC.
    ///
    /// The loop is the one of the ITS null job **[its]** (its.1652,
    /// NULJOB): `MOVE 17,[AOJA 17]` and a jump to 17, so that AC 17 holds
    /// `AOJA 0,17`, which adds 1 to AC 0 and jumps to itself. The test
    /// takes any `AOJA` in the ACs that jumps to itself and does not count
    /// in its own AC.
    fn idle_loop(&self) -> Option<(usize, W36)> {
        let pc = self.pc.get() as usize;
        if pc >= 16
            || self.oneproc != OneProc::Off
            || self.flags.0 & (Flags::TRAP1 | Flags::TRAP2) != 0
        {
            return None;
        }
        let w = self.ac[pc];
        let i = Inst::new(w);
        // Bits 13 to 17: no indirect, no index; E is the PC.
        let jumps_to_itself = w.field(13, 17) == 0 && w.rh().get() as usize == pc;
        (ins::OPS[usize::from(i.op)].name == "AOJA" && jumps_to_itself && i.ac != pc)
            .then(|| (i.ac, self.ac[i.ac]))
    }

    /// The processor is in the idle loop ([`Machine::idle_loop`]).
    pub fn idle(&self) -> bool {
        self.idle_loop().is_some()
    }

    /// One tick of the 1 ms clock, if [`Machine::wall_clock`] is set. True
    /// if the interval expired at it (the APR flag "interval done").
    pub fn tick(&mut self) -> bool {
        self.timer.last_tick = self.count;
        timer::tock(self)
    }

    /// One [`Machine::step`]. If the step was a pass of the idle loop
    /// ([`Machine::idle_loop`]), also do the next passes up to the first
    /// instruction count at which the timer, the 8080 or a device can do
    /// something, and at most to `limit`. Returns the number of passes done
    /// at once.
    ///
    /// The state after it is the same as after the same number of steps:
    /// each pass only adds 1 to the AC and to the count. Passes that would
    /// set a flag (the AC at 377777,,777777 or 777777,,777777) are done as
    /// steps. The step before the passes makes sure that the devices have
    /// seen the last I/O of the processor. The RTL has no such shortcut;
    /// `step` stays the model for it.
    pub fn step_idle(&mut self, limit: u64) -> Result<u64, Stop> {
        let before = self.idle_loop();
        let start = self.count;
        let printed = self.console.output.len();
        self.step()?;
        // A step that printed is not followed by passes, so that a reader
        // of the CTY output (a script, `expect`) sees the text at the same
        // count as with steps.
        if self.console.output.len() != printed {
            return Ok(0);
        }
        let Some((a, x)) = before else {
            return Ok(0);
        };
        // The step was the pass if it added 1 to the AC and the loop is
        // still there. Then no interrupt request can be acceptable: the
        // step tested the same requests before the pass, and the pass
        // changes none.
        let x = W36::wrap(x.get() + 1);
        if self.count != start + 1 || self.idle_loop() != Some((a, x)) {
            return Ok(0);
        }
        let tick = if self.wall_clock {
            u64::MAX
        } else {
            self.timer.last_tick + timer::INSTRUCTIONS_PER_MS
        };
        let deadline = limit
            .min(tick)
            .min(
                self.console
                    .deadline(self.mem[ks10_machine::console::CTYIN as usize]),
            )
            .min(self.io.deadline());
        let to_flag = |top: u64| top.wrapping_sub(x.get()) & W36::MASK;
        let n = deadline
            .saturating_sub(self.count)
            .min(to_flag(W36::SIGN - 1))
            .min(to_flag(W36::MASK));
        self.ac[a] = W36::wrap(x.get() + n);
        self.count += n;
        Ok(n)
    }

    /// Do instructions until the processor stops or `limit` steps are
    /// done.
    pub fn run(&mut self, limit: u64) -> Stop {
        let end = self.count.saturating_add(limit);
        while self.count < end {
            if let Err(s) = self.step() {
                return s;
            }
        }
        Stop::Limit
    }

    fn execute(&mut self, inst: Inst, e: Half, last: W36) -> Result<Next, Fault> {
        let op = &ins::OPS[usize::from(inst.op)];
        let ac_uuo = op.class == Class::AcDisp
            && ins::ac_op(inst.op, inst.ac as u8).is_some_and(|a| a.class == Class::Uuo);
        let d = &op.dram;
        let r = if ac_uuo || matches!(op.class, Class::Muuo | Class::Uuo) {
            Err(Fault::Uuo)
        } else {
            // AREAD: the memory cycle of E starts before the instruction
            // (simple.42, "THE INSTRUCTION LOOP -- FETCH ARGUMENTS").
            let pre = if !d.i && d.vma && (d.read || d.test || d.write) {
                self.touch(e, d.test || d.write)
            } else {
                Ok(())
            };
            pre.and_then(|()| match op.class {
                Class::Luuo => ops::luuo(self, inst, e),
                _ => ops::execute(self, inst, e, last),
            })
        };
        match r {
            Err(Fault::Uuo) => ops::muuo(self, inst, e).map(|()| Next::Jump(self.pc)),
            r => r,
        }
    }
}

/// A 20-bit physical address: a base register plus an offset, added as
/// full words (the microcode adds with the 2901s).
fn phys(base: W36, offset: u32) -> u32 {
    (base.get().wrapping_add(u64::from(offset)) & 0o3777777) as u32
}

/// The process context word as RDUBR gives it **[microcode]** (inout.50,
/// RDUBR and GETPCW of the ITS version): UBR AND 507703,,777777.
pub fn context_word(m: &Machine) -> W36 {
    W36::wrap(m.ubr.get() & 0o507703_777777)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine with paging on: EPT and UPT at 0, DBR4 (executive low)
    /// table at 100: ITS page 0 (0 to 1777) read/write to physical page 0,
    /// ITS page 1 (2000 to 3777) no access. DBR1 (user low) table at 200:
    /// ITS page 0 read/write to physical ITS page 2 (4000 to 5777).
    fn paged() -> Machine {
        let mut m = Machine::new();
        m.paging = true;
        m.dbr[3] = W36::wrap(0o100);
        m.mem[0o100] = W36::from_halves(Half::wrap(0o600000), Half::ZERO);
        m.dbr[0] = W36::wrap(0o200);
        m.mem[0o200] = W36::from_halves(Half::wrap(0o600002), Half::ZERO);
        m.pc = Half::wrap(0o1000);
        m
    }

    fn put(m: &mut Machine, a: u32, w: u64) {
        m.mem[a as usize] = W36::wrap(w);
    }

    // itspag.98, PFTRAP: page fail word at EPT+440, the PC word at 441,
    // the new PC from 442. MOVE does not change AC (the AREAD fails).
    #[test]
    fn page_fail_trap() {
        let mut m = paged();
        put(&mut m, 0o1000, 0o200040_002000); // MOVE 1,2000
        put(&mut m, 0o442, 0o1500);
        m.ac[1] = W36::wrap(7);
        m.step().unwrap();
        assert_eq!(m.mem[0o440].get(), 0o2000);
        assert_eq!(m.mem[0o441].get(), 0o1000);
        assert_eq!(m.pc.get(), 0o1500);
        assert_eq!(m.ac[1].get(), 7);
        // With level 3 in progress: EPT+451 (PFTD10).
        let mut m = paged();
        put(&mut m, 0o1000, 0o200040_002000);
        put(&mut m, 0o453, 0o1600);
        m.pi.in_progress = Pi::level_bit(3);
        m.step().unwrap();
        assert_eq!((m.mem[0o451].get(), m.pc.get()), (0o2000, 0o1600));
    }

    // PRM 2.9 (traps): with paging on, an overflow executes the
    // instruction at 421 after the instruction; the trap flags are
    // cleared, and a JSR stores the PC of the next instruction.
    #[test]
    fn overflow_trap() {
        let mut m = paged();
        put(&mut m, 0o1000, 0o271040_000001); // ADDI 1,1
        put(&mut m, 0o421, 0o264000_001700); // JSR 1700
        m.ac[1] = W36::wrap(0o377777_777777);
        m.step().unwrap();
        assert!(m.flags.has(Flags::TRAP1));
        m.step().unwrap();
        let f = u64::from(Flags::AROV | Flags::CRY1) << 18;
        assert_eq!(m.mem[0o1700].get(), f | 0o1001);
        assert_eq!(m.pc.get(), 0o1701);
        // A trap instruction that does not jump returns to the program.
        let mut m = paged();
        put(&mut m, 0o1000, 0o271040_000001);
        put(&mut m, 0o421, 0o350000_001600); // AOS 1600
        m.ac[1] = W36::wrap(0o377777_777777);
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!((m.mem[0o1600].get(), m.pc.get()), (1, 0o1001));
        assert!(!m.flags.has(Flags::TRAP1));
        // Paging off: no trap (PRM: "no effect at all when paging is
        // disabled").
        let mut m = paged();
        m.paging = false;
        put(&mut m, 0o1000, 0o271040_000001);
        put(&mut m, 0o1001, 0o255000_000000); // JFCL
        m.ac[1] = W36::wrap(0o377777_777777);
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.pc.get(), 0o1002);
    }

    // inout.50, TRP1: an opcode below 100 in the trap location is a
    // monitor UUO with the trap flags set: new PC from UPT+431.
    #[test]
    fn trap_uuo() {
        let mut m = paged();
        put(&mut m, 0o1000, 0o271040_000001);
        put(&mut m, 0o421, 0o040000_000123);
        put(&mut m, 0o431, 0o1400);
        m.ac[1] = W36::wrap(0o377777_777777);
        m.step().unwrap();
        m.step().unwrap();
        assert_eq!(m.mem[0o424].get(), 0o040000_000123);
        assert_eq!(m.mem[0o425].rh().get(), 0o1001);
        assert_eq!(m.pc.get(), 0o1400);
    }

    // ksdefs.194, XCTR: AC bit 4 (D1) makes the data reference use the
    // user map when PCU is set; UMOVE does so always.
    #[test]
    fn xctr_and_umove() {
        let mut m = paged();
        m.flags = Flags(Flags::USERIO); // executive mode, PCU
        put(&mut m, 0o4300, 0o555);
        put(&mut m, 0o300, 0o111);
        put(&mut m, 0o1000, 0o103200_001010); // XCTR 4,1010
        put(&mut m, 0o1010, 0o200040_000300); // MOVE 1,300
        m.step().unwrap();
        assert_eq!(m.ac[1].get(), 0o555);
        assert_eq!(m.pc.get(), 0o1001);
        put(&mut m, 0o1001, 0o200040_000300); // MOVE 1,300 (exec)
        m.step().unwrap();
        assert_eq!(m.ac[1].get(), 0o111);
        put(&mut m, 0o1002, 0o705040_000301); // UMOVEM 1,301
        m.step().unwrap();
        assert_eq!(m.mem[0o4301].get(), 0o111);
        // An AC address in the previous context is the previous AC block
        // (UBR bits 9 to 11).
        m.ubr = W36::from_halves(Half::wrap(0o000100), Half::ZERO);
        m.blocks[1][5] = W36::wrap(0o707);
        put(&mut m, 0o1003, 0o704040_000005); // UMOVE 1,5
        m.step().unwrap();
        assert_eq!(m.ac[1].get(), 0o707);
        // XCTR in user mode is a UUO.
        let mut m = paged();
        m.flags = Flags(Flags::USER);
        m.paging = false;
        put(&mut m, 0o1000, 0o103200_001010);
        put(&mut m, 0o434, 0o1300);
        m.step().unwrap();
        assert_eq!(m.pc.get(), 0o1300);
    }

    // simple.42, JRST2 and 1PROC-TRAP: JRSTF with bit 8 runs one
    // instruction, then stores the PC word at UPT+432 and takes the new
    // PC from 433.
    #[test]
    fn one_proceed() {
        let mut m = Machine::new();
        m.pc = Half::wrap(0o1000);
        put(&mut m, 0o1000, 0o254120_001010); // JRSTF @1010
        put(&mut m, 0o1010, 0o001000_001100); // bit 8,,1100
        put(&mut m, 0o1100, 0o271040_000001); // ADDI 1,1
        put(&mut m, 0o433, 0o1200);
        m.step().unwrap();
        assert_eq!(m.pc.get(), 0o1100);
        m.step().unwrap();
        assert_eq!(m.ac[1].get(), 1);
        m.step().unwrap();
        assert_eq!(m.mem[0o432].get(), 0o1101);
        assert_eq!(m.pc.get(), 0o1200);
        assert_eq!(m.oneproc, OneProc::Off);
    }

    // inout.50, IORDI: address 3,,E; no device is a page failure with "no
    // IO register" and "IO cycle" (itspag.98, HARD).
    #[test]
    fn io_page_fail() {
        let mut m = Machine::new();
        m.pc = Half::wrap(0o1000);
        put(&mut m, 0o1000, 0o710040_000100); // IORDI 1,100
        put(&mut m, 0o442, 0o1500);
        m.step().unwrap();
        assert_eq!(
            m.mem[0o440].get(),
            pfw::NO_IO | pfw::IO_CYCLE | (3 << 18) | 0o100
        );
        assert_eq!(m.pc.get(), 0o1500);
    }

    // inout.50, WRTHSB and DUMP: the block at HSBADR (500 after the ITS
    // microcode starts): MAG, PC, HR, ..., ONE at +7, T1 (the halt code)
    // at +17.
    #[test]
    fn halt_status_block() {
        let mut m = Machine::new();
        m.pc = Half::wrap(0o1000);
        put(&mut m, 0o1000, 0o254200_001234); // HALT 1234
        let s = m.step().unwrap_err();
        assert_eq!(
            s,
            Stop::Halt {
                pc: Half::wrap(0o1234),
                code: 1
            }
        );
        assert_eq!(m.mem[0o500].get(), 0o377777_777777);
        assert_eq!(m.mem[0o501].get(), 0o1234);
        assert_eq!(m.mem[0o502].get(), 0o254200_001234);
        assert_eq!(m.mem[0o507].get(), 1);
        assert_eq!(m.mem[0o517].get(), 1);
        assert_eq!((m.mem[0].get(), m.mem[1].get()), (1, 0o1234));
    }

    // inout.50, PFT: a page failure in an interrupt instruction halts with
    // code 100 (IOPF).
    #[test]
    fn page_fail_in_interrupt() {
        let mut m = paged();
        m.pi = Pi {
            on: true,
            active: 0o177,
            ..Pi::default()
        };
        m.apr.enabled = flag::INTERVAL;
        m.apr.flags = flag::INTERVAL;
        m.apr.level = 1;
        put(&mut m, 0o42, 0o264000_002000); // JSR 2000: no access
        assert_eq!(
            m.step().unwrap_err(),
            Stop::Halt {
                pc: Half::wrap(0o1000),
                code: 0o100
            }
        );
    }

    // itspag.98, USER-DBR: a user address from 400000 uses DBR2.
    #[test]
    fn user_high_uses_dbr2() {
        let mut m = paged();
        m.dbr[1] = W36::wrap(0o300);
        m.mem[0o300] = W36::from_halves(Half::wrap(0o600003), Half::ZERO);
        m.mem[(3 << 10) | 5] = W36::wrap(0o4242);
        let w = m.vread(Half::wrap(0o400005), Ref::Prev);
        // Ref::Prev with PCU clear is an executive reference: no access.
        assert!(w.is_err());
        m.flags = Flags(Flags::USERIO);
        assert_eq!(
            m.vread(Half::wrap(0o400005), Ref::Prev).unwrap().get(),
            0o4242
        );
    }

    // simple.42, GOEXEC: LOAD PCU. A MUUO from user mode enters
    // executive mode with PCU (bit 6) set.
    #[test]
    fn muuo_sets_pcu() {
        let mut m = Machine::new();
        m.flags = Flags(Flags::USER);
        m.pc = Half::wrap(0o1000);
        put(&mut m, 0o1000, 0o040000_000000); // MUUO
        put(&mut m, 0o434, 0o1500); // user, no trap: 430 + 4
        m.step().unwrap();
        assert_eq!(m.pc.get(), 0o1500);
        assert!(!m.flags.is_user());
        assert!(m.flags.has(Flags::USERIO));
    }

    // simple.42, "FETCH ARGUMENTS": ADDM tests E for writing before the
    // add, so a read-only page fails before the flags change.
    #[test]
    fn aread_write_test() {
        let mut m = paged();
        m.mem[0o100] = W36::from_halves(Half::wrap(0o600000), Half::wrap(0o200001)); // page 1 read only
        put(&mut m, 0o1000, 0o272040_002000); // ADDM 1,2000
        put(&mut m, 0o442, 0o1500);
        m.ac[1] = W36::wrap(0o377777_777777);
        m.mem[0o2000] = W36::wrap(1);
        m.step().unwrap();
        assert_eq!(m.pc.get(), 0o1500);
        assert_eq!(m.mem[0o440].get(), pfw::WRITE | pfw::ACCESS_28 | 0o2000);
        // The saved PC word has no Overflow.
        assert_eq!(m.mem[0o441].get(), 0o1000);
    }

    // A PUSH that fails on the stack word leaves AC as it was, so that it
    // can run again (simple.42, PUSH1: the AC is stored after the write).
    #[test]
    fn push_page_fail_keeps_ac() {
        let mut m = paged();
        put(&mut m, 0o1000, 0o261040_000100); // PUSH 1,100
        put(&mut m, 0o442, 0o1500);
        m.ac[1] = W36::from_halves(Half::wrap(0o777770), Half::wrap(0o1777));
        m.step().unwrap();
        assert_eq!(m.pc.get(), 0o1500);
        assert_eq!(
            m.ac[1],
            W36::from_halves(Half::wrap(0o777770), Half::wrap(0o1777))
        );
    }

    /// The state that a pass of the idle loop can change, and the state of
    /// the timer, the 8080 and the DZ11 that a skipped pass must not miss.
    fn idle_state(m: &Machine) -> String {
        let z = &m.io.dz;
        let lines: Vec<_> = z
            .lines
            .iter()
            .map(|l| (l.input.len(), l.busy_until))
            .collect();
        format!(
            "{} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {:?} {} {} {:?} {} {} {} {:?}",
            m.count,
            m.pc,
            m.flags,
            m.ac,
            m.apr,
            m.pi,
            m.timer,
            m.io.events,
            m.console,
            m.mem[ks10_machine::console::CTYIN as usize],
            z.csr,
            z.tcr,
            z.silo,
            z.rx_irq,
            z.tx_irq,
            z.tick_pending,
            lines
        )
    }

    /// The ITS null job loop (its.1652, NULJOB): AOJA 0,17 in AC 17. AC 0
    /// overflows after 40000 passes. The sources of work have times that
    /// differ, so that each one ends a skip: the timer ticks at 1300 +
    /// 1000 k, the DZ11 polls at 1000 k (line 4 has input), the transmitter
    /// of line 2 is busy to 3500, and a tape event is at 2650.
    fn idle_machine() -> Machine {
        use ks10_machine::devregs::dz11 as d;
        let mut m = Machine::new();
        m.ac[0o17] = W36::from_halves(Half::wrap(0o344000), Half::wrap(0o17));
        m.ac[0] = W36::wrap(W36::SIGN - 40_000);
        m.pc = Half::wrap(0o17);
        m.timer.period = W36::wrap(5 << 12);
        m.timer.ttg = m.timer.period;
        m.timer.last_tick = 300;
        m.io.events
            .push((2650, ks10_machine::unibus::Event::Tape(999)));
        let z = &mut m.io.dz;
        z.csr = d::csr::MSE as u16;
        z.tcr = 1 << 2;
        z.lines[2].busy_until = 3500;
        z.lines[4].lpr = d::rbuf::LPR_RXON as u16;
        z.lines[4].input.extend(b"xyz");
        m
    }

    // A skipped pass gives the same state as a step: compare a machine
    // that steps with one that skips, at each count where the skipping one
    // stops. The CTY input starts at the first stop (700), and at each
    // stop both take the CTY character, so that the 8080 gives the next
    // one INPUT_GAP (1000) after the last: at 700 + 1000 k, times that no
    // other source has. The other limits are at 700 + 1777 k.
    #[test]
    fn idle_skip_is_exact() {
        let mut a = idle_machine();
        let mut b = idle_machine();
        let mut skipped = 0;
        let mut limit = 700;
        let mut printed = 0;
        let mut bell = 2450;
        let ctyin = ks10_machine::console::CTYIN as usize;
        while b.count < 60_000 {
            let n = b.step_idle(limit).unwrap();
            skipped += n;
            if b.console.output.len() != printed {
                printed = b.console.output.len();
                assert_eq!(n, 0, "passes after the step that printed");
            }
            while a.count < b.count {
                a.step().unwrap();
            }
            assert_eq!(idle_state(&a), idle_state(&b));
            a.mem[ctyin] = W36::ZERO;
            b.mem[ctyin] = W36::ZERO;
            if b.count == 700 {
                a.console.input.extend(b"abcdefghijklmnopqrstuvwxyz");
                b.console.input.extend(b"abcdefghijklmnopqrstuvwxyz");
            }
            // The doorbell about every 5000 steps: the 8080 prints a
            // character OUTPUT_DELAY after it.
            if b.count >= bell {
                bell = b.count + 5000;
                for m in [&mut a, &mut b] {
                    m.mem[ks10_machine::console::CTYOT as usize] = W36::wrap(1 << 8 | 0o101);
                    m.console.doorbell(m.count);
                }
            }
            if b.count >= limit {
                limit += 1777;
            }
        }
        // The pass that overflows AC 0 is a step: it sets the flags. After
        // it, TRAP1 stays set (paging is off) and no pass is skipped.
        assert!(b.flags.has(Flags::AROV));
        assert!(skipped > 39_000, "{skipped} passes skipped");
        assert!(b.io.dz.lines[4].input.is_empty());
        assert!(b.console.input.is_empty());
        assert!(b.io.events.is_empty());
        assert!(printed > 5, "{printed} characters printed");
    }

    // The pass from 777777,,777777 to 0 sets CRY0 and CRY1 (PRM 2.6) and
    // is a step; the passes before and after it are skipped.
    #[test]
    fn idle_skip_stops_at_the_carry() {
        let start = || {
            let mut m = Machine::new();
            m.ac[0o17] = W36::from_halves(Half::wrap(0o344000), Half::wrap(0o17));
            m.ac[0] = W36::wrap(W36::MASK - 2500);
            m.pc = Half::wrap(0o17);
            m
        };
        let (mut a, mut b) = (start(), start());
        let mut skipped = 0;
        while b.count < 5000 {
            skipped += b.step_idle(5000).unwrap();
            while a.count < b.count {
                a.step().unwrap();
            }
            assert_eq!(idle_state(&a), idle_state(&b));
        }
        assert!(b.flags.has(Flags::CRY0) && b.flags.has(Flags::CRY1));
        assert!(!b.flags.has(Flags::AROV));
        assert_eq!(b.ac[0].get(), 5000 - 2501);
        assert!(skipped > 4000, "{skipped} passes skipped");
    }

    // Not the idle loop: an AOJA that counts in its own AC, and an AOJA
    // at the PC that jumps elsewhere.
    #[test]
    fn idle_skip_needs_the_loop() {
        let mut m = Machine::new();
        m.ac[0o17] = W36::from_halves(Half::wrap(0o344740), Half::wrap(0o17));
        m.pc = Half::wrap(0o17);
        assert_eq!(m.step_idle(1000).unwrap(), 0);
        let mut m = Machine::new();
        m.ac[0o16] = W36::from_halves(Half::wrap(0o344000), Half::wrap(0o17));
        m.ac[0o17] = W36::from_halves(Half::wrap(0o344000), Half::wrap(0o16));
        m.pc = Half::wrap(0o17);
        assert_eq!(m.step_idle(1000).unwrap(), 0);
        assert_eq!(m.step_idle(1000).unwrap(), 0);
        assert_eq!(m.count, 2);
    }
}

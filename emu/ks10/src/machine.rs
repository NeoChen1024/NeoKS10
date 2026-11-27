//! The machine state and the instruction cycle of the software processor.
//!
//! This first version has no paging, no interrupts and no devices: the
//! processor runs in executive mode on physical memory, as a stand-alone
//! program does before it enables the pager.

use pdp10::instructions::{self as ins, Class};
use pdp10::{Half, W36};

use crate::apr::{Apr, Pi};
use crate::flags::Flags;
use crate::ops;

/// Words of memory: 512 Ki (docs/emulator.md, configuration DB, `TSYSM`).
pub const MEMORY_WORDS: usize = 512 * 1024;

/// Why the processor stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The processor halted with a halt code (ksdefs.194, "Halt Codes"):
    /// 1 for a HALT instruction in executive mode (the PC is then its
    /// effective address), 101 for an illegal interrupt instruction.
    Halt { pc: Half, code: u32 },
    /// An instruction that this version does not implement.
    Unimplemented { pc: Half, word: W36 },
    /// A reference to memory above the installed memory.
    Nxm { pc: Half, address: u32 },
    /// The step limit of [`Machine::run`] was reached.
    Limit,
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
    /// Completed instructions (the virtual time, docs/emulator.md).
    pub count: u64,
    /// Physical address of the executive process table (EPT). WREBR sets
    /// it; 0 until then.
    pub ept: u32,
    /// Physical address of the user process table (UPT), from WRUBR; 0
    /// until then.
    pub ubr: u32,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
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
    /// Stop the processor.
    Stop(Stop),
}

/// The limit of indirect words in one effective address calculation. The
/// real processor has no limit; it can take an interrupt during a long
/// chain. Here a longer chain ends the step without a result, and the next
/// step does the instruction again, so that a guest loop cannot block the
/// emulator.
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
            ept: 0,
            ubr: 0,
        }
    }

    /// Read a word: locations 0 to 17 are the ACs (PRM 1.2, fast memory).
    pub fn read(&self, addr: Half) -> Result<W36, u32> {
        let a = addr.get() as usize;
        if a < 16 {
            Ok(self.ac[a])
        } else {
            self.mem.get(a).copied().ok_or(a as u32)
        }
    }

    pub fn write(&mut self, addr: Half, w: W36) -> Result<(), u32> {
        let a = addr.get() as usize;
        if a < 16 {
            self.ac[a] = w;
            Ok(())
        } else if let Some(m) = self.mem.get_mut(a) {
            *m = w;
            Ok(())
        } else {
            Err(a as u32)
        }
    }

    /// Physical memory, for the console and the loader.
    pub fn deposit(&mut self, addr: u32, w: W36) -> Result<(), u32> {
        let m = self.mem.get_mut(addr as usize).ok_or(addr)?;
        *m = w;
        Ok(())
    }

    /// The effective address calculation of a word with I, X and Y
    /// (PRM 1.6). Returns E and the last word of the calculation (JRSTF
    /// takes the flags from it), or `None` after MAX_INDIRECT indirect
    /// words.
    pub fn ea(&self, mut w: W36) -> Result<Option<(Half, W36)>, u32> {
        for _ in 0..=MAX_INDIRECT {
            let x = w.field(14, 17) as usize;
            let y = w.field(18, 35);
            let e = if x != 0 {
                Half::wrap(y + self.ac[x].rh().get())
            } else {
                Half::wrap(y)
            };
            if !w.bit(13) {
                return Ok(Some((e, if x != 0 { self.ac[x] } else { w })));
            }
            w = self.read(e)?;
        }
        Ok(None)
    }

    /// The interrupt requests of the APR (and later the devices) as a
    /// mask of levels.
    fn requests(&self) -> u8 {
        if self.apr.requesting() && self.apr.level != 0 {
            Pi::level_bit(self.apr.level)
        } else {
            0
        }
    }

    /// Take an interrupt on level n **[microcode]** (inout.50, PI to PI50,
    /// PIJSR, PIXPCW; PRM 4.1): hold the level, then execute the
    /// instruction at EPT + 40 + 2n. It must be JSR or XPCW (JRST 7,);
    /// else the processor halts with code 101 (ILLII). A JSR stores the PC
    /// word of the interrupted program at E, clears all flags (executive
    /// mode) and continues at E+1. Unibus vectors come with the devices.
    fn interrupt(&mut self, n: u8) -> Result<(), Stop> {
        let pc = self.pc;
        self.pi.in_progress |= Pi::level_bit(n);
        let loc = self.ept + 0o40 + 2 * u32::from(n);
        let word = *self
            .mem
            .get(loc as usize)
            .ok_or(Stop::Nxm { pc, address: loc })?;
        let Some((e, _)) = self.ea(word).map_err(|a| Stop::Nxm { pc, address: a })? else {
            return Ok(());
        };
        let nxm = |a| Stop::Nxm { pc, address: a };
        if word.field(0, 8) == 0o264 {
            self.write(e, self.flags.pc_word(pc)).map_err(nxm)?;
            self.flags = Flags(0);
            self.pc = Half::wrap(e.get() + 1);
            Ok(())
        } else if word.field(0, 12) == 0o254 << 4 | 7 {
            let (f, p) = ops::xpcw(self, e, pc).map_err(|f| match f {
                Fault::Nxm(a) => nxm(a),
                Fault::Unimplemented | Fault::Uuo => Stop::Unimplemented { pc, word },
            })?;
            self.flags = f;
            self.pc = p;
            Ok(())
        } else {
            self.deposit(0, W36::wrap(0o101)).map_err(nxm)?;
            self.deposit(1, W36::from_halves(Half::ZERO, pc))
                .map_err(nxm)?;
            Err(Stop::Halt { pc, code: 0o101 })
        }
    }

    /// Do one instruction, or take an interrupt. `Ok(())` if the processor
    /// can continue.
    pub fn step(&mut self) -> Result<(), Stop> {
        if let Some(n) = self.pi.accept(self.requests()) {
            return self.interrupt(n);
        }
        let pc = self.pc;
        let mut word = self.read(pc).map_err(|a| Stop::Nxm { pc, address: a })?;
        // XCT executes the word at E; a chain of XCTs stays in this step.
        for _ in 0..=MAX_INDIRECT {
            let inst = Inst::new(word);
            let Some((e, last)) = self.ea(word).map_err(|a| Stop::Nxm { pc, address: a })? else {
                return Ok(());
            };
            let next = self.execute(inst, e, last);
            match next {
                Next::Step => self.pc = Half::wrap(pc.get() + 1),
                Next::Skip => self.pc = Half::wrap(pc.get() + 2),
                Next::Jump(t) => self.pc = t,
                Next::Execute(w) => {
                    word = w;
                    continue;
                }
                Next::Stop(s) => return Err(s),
            }
            self.count += 1;
            return Ok(());
        }
        Ok(())
    }

    /// Do instructions until the processor stops or `limit` instructions
    /// are done.
    pub fn run(&mut self, limit: u64) -> Stop {
        let end = self.count.saturating_add(limit);
        while self.count < end {
            if let Err(s) = self.step() {
                return s;
            }
        }
        Stop::Limit
    }

    fn execute(&mut self, inst: Inst, e: Half, last: W36) -> Next {
        let op = &ins::OPS[usize::from(inst.op)];
        let ac_uuo = op.class == Class::AcDisp
            && ins::ac_op(inst.op, inst.ac as u8).is_some_and(|a| a.class == Class::Uuo);
        let r = match op.class {
            _ if ac_uuo => Err(Fault::Uuo),
            Class::Luuo => ops::luuo(self, inst, e),
            Class::Inst | Class::AcDisp => ops::execute(self, inst, e, last),
            Class::Muuo | Class::Uuo => Err(Fault::Uuo),
        };
        let r = match r {
            Err(Fault::Uuo) => ops::muuo(self, inst, e).map(|()| Next::Jump(self.pc)),
            r => r,
        };
        match r {
            Ok(n) => n,
            Err(Fault::Unimplemented | Fault::Uuo) => Next::Stop(Stop::Unimplemented {
                pc: self.pc,
                word: inst.word,
            }),
            Err(Fault::Nxm(a)) => Next::Stop(Stop::Nxm {
                pc: self.pc,
                address: a,
            }),
        }
    }
}

/// An instruction that does not complete normally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The emulator does not have this instruction yet.
    Unimplemented,
    /// The microcode does a monitor UUO (an illegal or monitor call).
    Uuo,
    Nxm(u32),
}

/// The process context word, as RDUBR gives it (ksdefs.194, WRUBR and
/// RDUBR): bits 0 and 2 set, the current and previous AC blocks (one block
/// in this version), the UBR address in bits 16 to 35.
pub fn context_word(m: &Machine) -> W36 {
    W36::wrap((0o500000u64 << 18) | u64::from(m.ubr))
}

impl From<u32> for Fault {
    fn from(a: u32) -> Self {
        Fault::Nxm(a)
    }
}

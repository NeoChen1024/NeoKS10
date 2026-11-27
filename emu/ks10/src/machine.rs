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
    /// A HALT instruction in executive mode, at `pc` (the PC is the
    /// effective address of the HALT).
    Halt { pc: Half },
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

    /// Do one instruction. `Ok(())` if the processor can continue.
    pub fn step(&mut self) -> Result<(), Stop> {
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
        let r = match op.class {
            Class::Luuo => ops::luuo(self, inst, e),
            Class::Inst | Class::AcDisp => ops::execute(self, inst, e, last),
            Class::Muuo | Class::Uuo => Err(Fault::Unimplemented),
        };
        match r {
            Ok(n) => n,
            Err(Fault::Unimplemented) => Next::Stop(Stop::Unimplemented {
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

/// An instruction that cannot complete in this version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Unimplemented,
    Nxm(u32),
}

impl From<u32> for Fault {
    fn from(a: u32) -> Self {
        Fault::Nxm(a)
    }
}

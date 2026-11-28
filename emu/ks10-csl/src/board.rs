//! The CSL board: the 8080, its ROM and RAM, the two 8251 USARTs, and the
//! I/O ports that go to the KS10.
//!
//! The 8080 is the crate `x80-core` (design decision 2026-11-28,
//! docs/console-interface.md). Time counts in 8080 states.
//!
//! The model has no KS10 yet. A write to a port of the KS10 goes into a
//! log. A read of a port, or an access to an address, that has no model
//! stops the run with the reason; the model does not return a guessed
//! value (docs/unknowns.md, "CSL board").

use x80_core::i8080::{I8080, Inputs};
use x80_core::{Bus, Control, Exit, MemoryAccess};

use crate::rom;
use crate::usart::{self, Usart};

/// The first RAM address (`RAMST`, KNS10.PNT).
pub const RAM_START: u16 = 0o20000;
/// The size of the RAM in bytes: 1 KiB (`PWRUP`: `LXI SP,RAMST+^O2000`,
/// and `CLRLP` clears ^O2000 bytes).
pub const RAM_SIZE: usize = 0o2000;

/// The ports of the USARTs (CONDEF.M80: `CTYDAT`, `CTYCTL`, `REMDAT`,
/// `REMCTL`).
const CTYDAT: u8 = 0o200;
const CTYCTL: u8 = 0o201;
const REMDAT: u8 = 0o202;
const REMCTL: u8 = 0o203;
/// The port of the switches that `PWRUP` reads (CONDEF.M80: `TTYSW`).
const TTYSW: u8 = 0o300;

/// The output ports of the KS10 that the console program uses (KNS10.PNT,
/// all `OUT` instructions). The model logs a write to them.
fn ks10_output(port: u8) -> bool {
    matches!(port, 0o100..=0o116 | 0o204..=0o206 | 0o210 | 0o212)
}

/// The inputs of the board that have no source yet (docs/unknowns.md).
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// The value of port 300 (`TTYSW`).
    pub switches: u8,
    /// The CTS input of both USARTs is low (asserted).
    pub cts: bool,
    /// The time of one character on the serial lines, in 8080 states.
    pub char_states: u64,
}

/// A write to a port of the KS10.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Write {
    /// The time in 8080 states, at the start of the instruction.
    pub at: u64,
    pub port: u8,
    pub value: u8,
}

/// The reason that a run stopped before its time ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// An access to an address without a model.
    Memory { pc: u16, address: u16, write: bool },
    /// A read of a port without a model.
    Input { pc: u16, port: u8 },
    /// A write to a port without a model.
    Output { pc: u16, port: u8, value: u8 },
    /// A mode of a USART that the model does not implement.
    Usart { pc: u16, port: u8, mode: u8 },
    /// An opcode that `x80-core` does not execute.
    Opcode { pc: u16, opcode: u8 },
    /// The 8080 halted. No interrupt can come yet.
    Halt { pc: u16 },
    /// An exit of `x80-core` that this model does not know.
    Other { pc: u16 },
}

/// Addresses, ports and values in octal.
impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            Stop::Memory { pc, address, write } => {
                let what = if write { "write to" } else { "read of" };
                write!(
                    f,
                    "PC {pc:06o}: {what} address {address:06o} without a model"
                )
            }
            Stop::Input { pc, port } => write!(f, "PC {pc:06o}: IN {port:03o} without a model"),
            Stop::Output { pc, port, value } => {
                write!(
                    f,
                    "PC {pc:06o}: OUT {port:03o} ({value:03o}) without a model"
                )
            }
            Stop::Usart { pc, port, mode } => {
                write!(
                    f,
                    "PC {pc:06o}: USART {port:03o}: mode {mode:03o} without a model"
                )
            }
            Stop::Opcode { pc, opcode } => {
                write!(
                    f,
                    "PC {pc:06o}: opcode {opcode:03o} not executed by x80-core"
                )
            }
            Stop::Halt { pc } => write!(f, "PC {pc:06o}: HLT, and no interrupt can come"),
            Stop::Other { pc } => write!(f, "PC {pc:06o}: an exit of x80-core without a model"),
        }
    }
}

/// Everything around the 8080.
struct Board {
    rom: Vec<u8>,
    ram: Vec<u8>,
    switches: u8,
    cty: Usart,
    rem: Usart,
    now: u64,
    /// The address of the current instruction.
    pc: u16,
    writes: Vec<Write>,
    stop: Option<Stop>,
}

impl Board {
    fn ram_index(address: u16) -> Option<usize> {
        let i = usize::from(address.wrapping_sub(RAM_START));
        (address >= RAM_START && i < RAM_SIZE).then_some(i)
    }

    fn stop(&mut self, s: Stop) {
        self.stop.get_or_insert(s);
    }

    fn control(&mut self, port: u8, value: u8) {
        let u = if port == CTYCTL {
            &mut self.cty
        } else {
            &mut self.rem
        };
        if let Err(usart::Unsupported { mode }) = u.write_control(value, self.now) {
            let pc = self.pc;
            self.stop(Stop::Usart { pc, port, mode });
        }
    }
}

impl Bus for Board {
    fn read(&mut self, address: u16, _kind: MemoryAccess) -> u8 {
        if usize::from(address) < rom::SIZE {
            return self.rom[usize::from(address)];
        }
        if let Some(i) = Board::ram_index(address) {
            return self.ram[i];
        }
        let pc = self.pc;
        self.stop(Stop::Memory {
            pc,
            address,
            write: false,
        });
        0
    }

    fn write(&mut self, address: u16, value: u8, _kind: MemoryAccess) {
        if let Some(i) = Board::ram_index(address) {
            self.ram[i] = value;
            return;
        }
        let pc = self.pc;
        self.stop(Stop::Memory {
            pc,
            address,
            write: true,
        });
    }

    fn input(&mut self, port: u8) -> u8 {
        match port {
            CTYDAT => self.cty.read_data(self.now),
            CTYCTL => self.cty.status(self.now),
            REMDAT => self.rem.read_data(self.now),
            REMCTL => self.rem.status(self.now),
            TTYSW => self.switches,
            _ => {
                let pc = self.pc;
                self.stop(Stop::Input { pc, port });
                0
            }
        }
    }

    fn output(&mut self, port: u8, value: u8) {
        match port {
            CTYDAT => self.cty.write_data(value, self.now),
            REMDAT => self.rem.write_data(value, self.now),
            CTYCTL | REMCTL => self.control(port, value),
            _ => {}
        }
        if ks10_output(port) || matches!(port, CTYCTL | REMCTL) {
            self.writes.push(Write {
                at: self.now,
                port,
                value,
            });
        } else if !matches!(port, CTYDAT | REMDAT) {
            let pc = self.pc;
            self.stop(Stop::Output { pc, port, value });
        }
    }
}

impl Control for Board {
    fn stop_requested(&mut self) -> bool {
        self.stop.is_some()
    }
}

/// The INT input of the 8080 stays high: its source on the board is not
/// known yet (docs/unknowns.md).
impl Inputs for Board {}

/// The CSL board after power-up.
pub struct Csl {
    cpu: I8080,
    board: Board,
}

impl Csl {
    /// The board with the ROM `rom` (`rom::load`), at the power-up state:
    /// the 8080 at address 0 (`HCORE.M80`, "AT POWER UP, ALWAYS BEGIN AT
    /// 0000") and the USARTs after RESET.
    pub fn new(rom: Vec<u8>, config: Config) -> Result<Csl, String> {
        if rom.len() != rom::SIZE {
            return Err(format!("ROM of {} bytes, not {}", rom.len(), rom::SIZE));
        }
        let mut cpu = I8080::default();
        cpu.reset();
        Ok(Csl {
            cpu,
            board: Board {
                rom,
                // The content of the RAM at power-up has no source; PWRUP
                // clears it before it reads it.
                ram: vec![0; RAM_SIZE],
                switches: config.switches,
                cty: Usart::new(config.cts, config.char_states),
                rem: Usart::new(config.cts, config.char_states),
                now: 0,
                pc: 0,
                writes: Vec::new(),
                stop: None,
            },
        })
    }

    /// Run for at least `states` 8080 states. `Ok` if the time ended,
    /// else the reason of the stop. A stop stays: the next run returns it
    /// again.
    pub fn run(&mut self, states: u64) -> Result<(), Stop> {
        let end = self.board.now.saturating_add(states);
        while self.board.now < end {
            if let Some(s) = self.board.stop {
                return Err(s);
            }
            // The address of the last instruction that started.
            let last = self.board.pc;
            let pc = self.cpu.registers().pc;
            self.board.pc = pc;
            let r = self.cpu.step(&mut self.board);
            self.board.now += r.cycles;
            match r.exit {
                Exit::Budget | Exit::Stop => {}
                // The HLT ran in the step before.
                Exit::Halt => self.board.stop(Stop::Halt { pc: last }),
                Exit::Unsupported(u) => self.board.stop(Stop::Opcode {
                    pc: u.pc,
                    opcode: u.opcode,
                }),
                _ => self.board.stop(Stop::Other { pc }),
            }
        }
        match self.board.stop {
            Some(s) => Err(s),
            None => Ok(()),
        }
    }

    /// The time since power-up, in 8080 states.
    pub fn now(&self) -> u64 {
        self.board.now
    }

    /// The characters that the CTY USART sent.
    pub fn cty_output(&self) -> &[u8] {
        self.board.cty.sent()
    }

    /// The characters in the CTY USART that are not sent yet.
    pub fn cty_queued(&self) -> Vec<u8> {
        self.board.cty.queued()
    }

    /// The characters that the KLINIK USART sent.
    pub fn klinik_output(&self) -> &[u8] {
        self.board.rem.sent()
    }

    /// The writes to the ports of the KS10 and to the control ports of
    /// the USARTs, in order.
    pub fn writes(&self) -> &[Write] {
        &self.board.writes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: Config = Config {
        switches: 0,
        cts: true,
        char_states: 0,
    };

    /// A ROM with `code` at address 0 and zero bytes after it.
    fn csl(code: &[u8]) -> Csl {
        let mut rom = vec![0; rom::SIZE];
        rom[..code.len()].copy_from_slice(code);
        Csl::new(rom, CONFIG).unwrap()
    }

    // Opcodes of 98-153B p. 4-15 (the summary of the instructions).
    const MVI_A: u8 = 0o076;
    const OUT: u8 = 0o323;
    const IN: u8 = 0o333;
    const STA: u8 = 0o062;
    const LDA: u8 = 0o072;
    const HLT: u8 = 0o166;
    const JMP: u8 = 0o303;

    #[test]
    fn a_ks10_write_goes_into_the_log() {
        let mut c = csl(&[MVI_A, 0o5, OUT, 0o204, JMP, 4, 0]);
        assert_eq!(c.run(1000), Ok(()));
        assert_eq!(
            c.writes()[0],
            Write {
                at: 7,
                port: 0o204,
                value: 5
            }
        );
    }

    #[test]
    fn a_read_of_an_unknown_port_stops_the_run() {
        let mut c = csl(&[IN, 0o301]);
        assert_eq!(c.run(1000), Err(Stop::Input { pc: 0, port: 0o301 }));
        assert_eq!(c.run(1000), Err(Stop::Input { pc: 0, port: 0o301 }));
    }

    #[test]
    fn a_write_to_an_unknown_port_stops_the_run() {
        let mut c = csl(&[OUT, 0o5]);
        assert_eq!(
            c.run(1000),
            Err(Stop::Output {
                pc: 0,
                port: 0o5,
                value: 0
            })
        );
    }

    #[test]
    fn the_ram_is_one_kib_at_20000() {
        let mut c = csl(&[
            MVI_A, 0o42, STA, 0o377, 0o43, LDA, 0o377, 0o43, OUT, 0o210, HLT,
        ]);
        assert_eq!(c.run(1000), Err(Stop::Halt { pc: 10 }));
        assert_eq!(c.writes()[0].value, 0o42);

        let mut c = csl(&[STA, 0, 0o44]);
        assert_eq!(
            c.run(1000),
            Err(Stop::Memory {
                pc: 0,
                address: 0o22000,
                write: true
            })
        );
        let mut c = csl(&[STA, 0, 0]);
        assert_eq!(
            c.run(1000),
            Err(Stop::Memory {
                pc: 0,
                address: 0,
                write: true
            })
        );
    }

    #[test]
    fn a_stop_is_octal_text() {
        let s = Stop::Input {
            pc: 0o15232,
            port: 0o301,
        };
        assert_eq!(s.to_string(), "PC 015232: IN 301 without a model");
    }

    #[test]
    fn an_unsupported_opcode_stops_the_run() {
        // 010 is not in the summary of p. 4-15.
        let mut c = csl(&[0o010]);
        assert_eq!(
            c.run(1000),
            Err(Stop::Opcode {
                pc: 0,
                opcode: 0o010
            })
        );
    }
}

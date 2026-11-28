//! The CSL board: the 8080, its ROM and RAM, the two 8251 USARTs, and the
//! I/O ports that go to the KS10.
//!
//! The 8080 is the crate `x80-core` (design decision 2026-11-28,
//! docs/console-interface.md). Time counts in 8080 states.
//!
//! The ports of the KS10 go to `ks10::Interface`, and through it to a
//! `Ks10`. Each write to them goes into a log for the tests. A port, an
//! address or a function without a model stops the run with the reason;
//! the model does not return a guessed value (docs/unknowns.md, "CSL
//! board").

use x80_core::i8080::{I8080, Inputs};
use x80_core::{Bus, Control, Exit, MemoryAccess};

use crate::ks10::{Interface, Ks10};
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
/// The instruction that the 8228 gives at an interrupt acknowledge: RST 7.
/// Its INTA pin is on +12 V through 1 kOhm (MP00540 CSL2 rev E, E50, R32),
/// which selects this instruction (Intel 98-153B p. 5-8).
const RST_7: u8 = 0o377;

/// The inputs of the board that are not part of the drawings.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Bits 7 to 4 of port 300 (`TTYSW`) as the 8080 reads them: the DIP
    /// switches E173 #3, #2, #7 and #4 (CSLA E166). A switch that is on
    /// reads 1. The bits 3 to 0 come from the board.
    pub switches: u8,
    /// The carrier inputs of the CTY and of the KLINIK line (port 302
    /// bits 1 and 0, CSL9).
    pub cty_carrier: bool,
    pub klinik_carrier: bool,
    /// The time of one character on the serial lines, in 8080 states. The
    /// baud rate is open (docs/unknowns.md).
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
    /// A KS10 register or bus function without a model.
    Ks10 {
        pc: u16,
        port: u8,
        value: u8,
        what: &'static str,
    },
    /// A USART access while CSL9 USART CLR holds it in reset.
    UsartReset { pc: u16, port: u8 },
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
            Stop::Ks10 {
                pc,
                port,
                value,
                what,
            } => write!(
                f,
                "PC {pc:06o}: OUT {port:03o} ({value:03o}): {what} without a model"
            ),
            Stop::UsartReset { pc, port } => {
                write!(f, "PC {pc:06o}: USART {port:03o} while it is held in reset")
            }
            Stop::Halt { pc } => write!(f, "PC {pc:06o}: HLT, and no interrupt can come"),
            Stop::Other { pc } => write!(f, "PC {pc:06o}: an exit of x80-core without a model"),
        }
    }
}

/// Everything around the 8080.
struct Board<K: Ks10> {
    rom: Vec<u8>,
    ram: Vec<u8>,
    config: Config,
    interface: Interface,
    ks10: K,
    cty: Usart,
    rem: Usart,
    now: u64,
    /// The address of the current instruction.
    pc: u16,
    writes: Vec<Write>,
    stop: Option<Stop>,
}

impl<K: Ks10> Board<K> {
    fn ram_index(address: u16) -> Option<usize> {
        let i = usize::from(address.wrapping_sub(RAM_START));
        (address >= RAM_START && i < RAM_SIZE).then_some(i)
    }

    /// A write to a port of the KS10 side, after the devices of the KS10
    /// run to the time of the write.
    fn ks10_write(&mut self, port: u8, value: u8) -> Result<bool, &'static str> {
        self.ks10.idle_to(self.now);
        self.interface.write(port, value, self.now, &mut self.ks10)
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

impl<K: Ks10> Bus for Board<K> {
    fn read(&mut self, address: u16, kind: MemoryAccess) -> u8 {
        if kind == MemoryAccess::InterruptAcknowledge {
            return RST_7;
        }
        if usize::from(address) < rom::SIZE {
            return self.rom[usize::from(address)];
        }
        if let Some(i) = Self::ram_index(address) {
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
        if let Some(i) = Self::ram_index(address) {
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
        let pc = self.pc;
        if matches!(port, CTYDAT..=REMCTL) && self.interface.power_up_reset {
            self.stop(Stop::UsartReset { pc, port });
            return 0;
        }
        match port {
            CTYDAT => self.cty.read_data(self.now),
            CTYCTL => self.cty.status(self.now),
            REMDAT => self.rem.read_data(self.now),
            REMCTL => self.rem.status(self.now),
            _ => {
                self.ks10.idle_to(self.now);
                let c = self.config;
                let carrier = (c.cty_carrier, c.klinik_carrier);
                match self.interface.read(port, c.switches, carrier, &self.ks10) {
                    Some(v) => v,
                    None => {
                        self.stop(Stop::Input { pc, port });
                        0
                    }
                }
            }
        }
    }

    fn output(&mut self, port: u8, value: u8) {
        let pc = self.pc;
        if matches!(port, CTYDAT..=REMCTL) && self.interface.power_up_reset {
            self.stop(Stop::UsartReset { pc, port });
            return;
        }
        if !matches!(port, CTYDAT | REMDAT) {
            self.writes.push(Write {
                at: self.now,
                port,
                value,
            });
        }
        match port {
            CTYDAT => self.cty.write_data(value, self.now),
            REMDAT => self.rem.write_data(value, self.now),
            CTYCTL | REMCTL => self.control(port, value),
            _ => match self.ks10_write(port, value) {
                Ok(true) => {}
                Ok(false) => {
                    self.writes.pop();
                    self.stop(Stop::Output { pc, port, value });
                }
                Err(what) => self.stop(Stop::Ks10 {
                    pc,
                    port,
                    value,
                    what,
                }),
            },
        }
    }
}

impl<K: Ks10> Control for Board<K> {
    fn stop_requested(&mut self) -> bool {
        self.stop.is_some()
    }
}

/// INT is RxRDY of the two USARTs (MP00540 CSL2 rev E, E23). The model of
/// the USART has no receiver yet, so INT stays inactive.
impl<K: Ks10> Inputs for Board<K> {}

/// The CSL board after power-up, with the KS10 `K`.
pub struct Csl<K: Ks10> {
    cpu: I8080,
    board: Board<K>,
}

impl<K: Ks10> Csl<K> {
    /// The board with the ROM `rom` (`rom::load`), at the power-up state:
    /// the 8080 at address 0 (`HCORE.M80`, "AT POWER UP, ALWAYS BEGIN AT
    /// 0000") and the USARTs in reset.
    pub fn new(rom: Vec<u8>, config: Config, mut ks10: K) -> Result<Csl<K>, String> {
        ks10.attach();
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
                config,
                interface: Interface::default(),
                ks10,
                // CTS is on GND (CSL2 E42, CSL9 E6).
                cty: Usart::new(true, config.char_states),
                rem: Usart::new(true, config.char_states),
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
            if self.board.stop.is_some() {
                break;
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
        self.board.ks10.idle_to(self.board.now);
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

    /// The KS10 side of the board.
    pub fn interface(&self) -> &Interface {
        &self.board.interface
    }

    pub fn ks10(&self) -> &K {
        &self.board.ks10
    }

    pub fn ks10_mut(&mut self) -> &mut K {
        &mut self.board.ks10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ks10::Detached;

    const CONFIG: Config = Config {
        switches: 0,
        cty_carrier: false,
        klinik_carrier: false,
        char_states: 0,
    };

    /// A ROM with `code` at address 0 and zero bytes after it.
    fn csl(code: &[u8]) -> Csl<Detached> {
        let mut rom = vec![0; rom::SIZE];
        rom[..code.len()].copy_from_slice(code);
        Csl::new(rom, CONFIG, Detached::default()).unwrap()
    }

    // Opcodes of 98-153B p. 4-15 (the summary of the instructions).
    const MVI_A: u8 = 0o076;
    const OUT: u8 = 0o323;
    const IN: u8 = 0o333;
    const STA: u8 = 0o062;
    const LDA: u8 = 0o072;
    const HLT: u8 = 0o166;
    const JMP: u8 = 0o303;

    /// A KS10 that keeps the times of `idle_to`.
    #[derive(Default)]
    struct Clock(Vec<u64>, crate::ks10::Inputs);

    impl Ks10 for Clock {
        fn memory_read(&mut self, _a: u32) -> Result<crate::ks10::Reply, &'static str> {
            Err("no memory")
        }
        fn memory_write(&mut self, _a: u32, _v: pdp10::W36) -> Result<bool, &'static str> {
            Err("no memory")
        }
        fn io_read(&mut self, _a: u32, _b: bool) -> Result<crate::ks10::Reply, &'static str> {
            Err("no device")
        }
        fn io_write(&mut self, _a: u32, _v: pdp10::W36, _b: bool) -> Result<bool, &'static str> {
            Err("no device")
        }
        fn idle_to(&mut self, now: u64) {
            self.0.push(now);
        }
        fn inputs(&self) -> crate::ks10::Inputs {
            self.1
        }
        fn set_inputs(&mut self, i: crate::ks10::Inputs) {
            self.1 = i;
        }
        fn halt_loop(&self) -> bool {
            false
        }
    }

    #[test]
    fn the_ks10_gets_the_time_before_each_access() {
        // OUT and IN take 10 states, HLT 7 (98-153B p. 4-15): the accesses
        // start at 0 and 10, and the run ends at 27.
        let mut rom = vec![0; rom::SIZE];
        rom[..5].copy_from_slice(&[OUT, 0o101, IN, 0o301, HLT]);
        let mut c = Csl::new(rom, CONFIG, Clock::default()).unwrap();
        assert!(matches!(c.run(1000), Err(Stop::Halt { .. })));
        assert_eq!(c.ks10().0, [0, 10, 27]);
    }

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
        // CSL3 E93 decodes no read of 204 to 277 but the USARTs.
        let mut c = csl(&[IN, 0o204]);
        assert_eq!(c.run(1000), Err(Stop::Input { pc: 0, port: 0o204 }));
        assert_eq!(c.run(1000), Err(Stop::Input { pc: 0, port: 0o204 }));
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
            MVI_A, 0o42, STA, 0o377, 0o43, LDA, 0o377, 0o43, OUT, 0o204, HLT,
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
    fn the_usarts_wait_for_the_first_write_to_100() {
        let mut c = csl(&[OUT, 0o201]);
        assert_eq!(c.run(1000), Err(Stop::UsartReset { pc: 0, port: 0o201 }));
        // An asynchronous mode word (98-153B p. 5-140).
        let mut c = csl(&[MVI_A, 0o116, OUT, 0o101, OUT, 0o201, JMP, 6, 0]);
        assert_eq!(c.run(1000), Ok(()));
    }

    #[test]
    fn a_bus_function_needs_a_ks10() {
        let mut c = csl(&[OUT, 0o101, MVI_A, 0o343, OUT, 0o210]);
        assert!(matches!(
            c.run(1000),
            Err(Stop::Ks10 {
                pc: 4,
                port: 0o210,
                value: 0o343,
                ..
            })
        ));
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

//! The KS10 side of the CSL board: the console registers that the 8080
//! writes and reads with `OUT` and `IN`, the 4x4 memories and the 8646
//! latches of the bus, and the bus functions.
//!
//! The bit assignments are **[manual]** MP00540 (KS10 maintenance
//! schematics, March 1981), M8616 sheets CSL3 rev D, CSL4 rev C, CSL5 rev D,
//! CSL6 rev C, CSL7 rev B, CSL8 rev B and CSLA rev C (docs/csl-registers.md);
//! the element numbers are in the comments. The bus functions are those of
//! EK-OKS10-TM-002 Table 5-10, and the command/address word is that of
//! 5.3.4.
//!
//! Choices of the model, not statements of the drawings:
//! - A bus function completes at the write to register 210. On the path
//!   from power-up to the boot the KS10 clock is stopped (it starts only
//!   with the command CS, `CSCMD`), and the program reads the result at
//!   once (`EM`, `BUSRESP`).
//! - The levels of the error, parity and PI request inputs of the reads
//!   100 to 103 are those of a KS10 without errors and requests. The bits
//!   without a driver read as high (docs/unknowns.md).
//! - That a write to 101 also ends RESET (1) is derived from the decode
//!   (docs/csl-registers.md, U3).
//! - While the KS10 clock is stopped, the devices of the KS10 get their
//!   time from the board (`Ks10::idle_to`), and INT 10 waits in E171
//!   (docs/console-interface.md).
//!
//! A function or a register that the model does not have stops the run.

use pdp10::W36;

/// The processor inputs that the console drives: CSL4 RUN, EXECUTE and
/// CONTINUE (register 212), TRAP EN (205), CACHE EN and 1 MSEC EN (100).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub run: bool,
    pub execute: bool,
    pub cont: bool,
    pub trap_en: bool,
    pub cache_en: bool,
    pub one_ms_en: bool,
}

/// The answer to a read on the KS10 bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The data cycle with this word.
    Data(W36),
    /// No device answers.
    None,
}

/// The KS10 as the console sees it.
pub trait Ks10 {
    /// The board becomes the console of this KS10.
    fn attach(&mut self) {}
    /// A memory read at `address` (EK-OKS10-TM-002 5.3.5). `None` if no
    /// memory answers with MEM BUSY. `Err` if the KS10 has no model of it.
    fn memory_read(&mut self, address: u32) -> Result<Reply, &'static str>;
    /// An I/O register read at `address`, bits 14 to 35 of the
    /// command/address word (5.3.6); a byte read if `byte`. `None` if no
    /// device answers.
    /// A memory write. `false` if no memory takes it.
    fn memory_write(&mut self, address: u32, value: W36) -> Result<bool, &'static str>;
    fn io_read(&mut self, address: u32, byte: bool) -> Result<Reply, &'static str>;
    /// An I/O register write. `false` if no device takes it.
    fn io_write(&mut self, address: u32, value: W36, byte: bool) -> Result<bool, &'static str>;
    /// The 8080 time is `now` states since power-up, and the KS10 clock
    /// is stopped: the devices of the KS10 run to that time.
    fn idle_to(&mut self, now: u64);
    /// The inputs of the processor from the console.
    fn inputs(&self) -> Inputs;
    fn set_inputs(&mut self, i: Inputs);
    /// CSL4 HALT LOOP (1) (E161): the microcode sets and clears it.
    fn halt_loop(&self) -> bool;
}

/// No KS10: the board alone. A bus function stops the run.
#[derive(Clone, Copy, Debug, Default)]
pub struct Detached {
    inputs: Inputs,
}

impl Ks10 for Detached {
    fn memory_read(&mut self, _address: u32) -> Result<Reply, &'static str> {
        Err("the KS10")
    }
    fn memory_write(&mut self, _address: u32, _value: W36) -> Result<bool, &'static str> {
        Err("the KS10")
    }
    fn io_read(&mut self, _address: u32, _byte: bool) -> Result<Reply, &'static str> {
        Err("the KS10")
    }
    fn io_write(&mut self, _a: u32, _v: W36, _byte: bool) -> Result<bool, &'static str> {
        Err("the KS10")
    }
    fn idle_to(&mut self, _now: u64) {}
    fn inputs(&self) -> Inputs {
        self.inputs
    }
    fn set_inputs(&mut self, i: Inputs) {
        self.inputs = i;
    }
    fn halt_loop(&self) -> bool {
        false
    }
}

/// A bus function of the console, for the log of the tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusCycle {
    /// The time in 8080 states.
    pub at: u64,
    pub function: Function,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    /// A memory read: the command/address word and the reply.
    MemoryRead { com_adr: W36, reply: Reply },
    /// Examine bus: the latches take the bus without a transfer.
    ExamineBus { data: W36 },
    /// A memory write: the command/address word, the data and if a
    /// memory took it.
    MemoryWrite {
        com_adr: W36,
        data: W36,
        taken: bool,
    },
    /// An I/O register read: the command/address word and the reply.
    IoRead { com_adr: W36, reply: Reply },
    /// An I/O register write: the command/address word, the data and if
    /// a device took it.
    IoWrite {
        com_adr: W36,
        data: W36,
        taken: bool,
    },
}

/// Register 210 (CSL4 E121, E129; CSL3 E151).
const CLOSE_LATCHES: u8 = 1 << 0;
const LATCH_DATA: u8 = 1 << 1;
const CRA_R_CLK: u8 = 1 << 2;
const CRA_T_CLK: u8 = 1 << 3;
const XMIT_DATA: u8 = 1 << 4;
const XMIT_ADR: u8 = 1 << 5;
const BUS_REQ: u8 = 1 << 6;
const MEM: u8 = 1 << 7;

/// The state of the KS10 side of the board.
#[derive(Clone, Debug)]
pub struct Interface {
    /// CSL5 RESET (1) (E145): set at power-up, ended by the first write to
    /// register 100 or 101 (E145 PRE is CSL3 CSL REG WRT 100 L, which A0
    /// does not qualify). It also holds both USARTs in reset (CSL9 E65).
    pub power_up_reset: bool,
    /// Register 100 (CSL3 E138, E151; CSL4 E142): bit 7 CSL3 RESET, bits
    /// 6 to 4 PE DETECT, CRM DETECT, DP PE DETECT. Bits 3 (CACHE EN) and 2
    /// (1 MSEC EN) are processor inputs.
    pub reset_pe: u8,
    /// Register 101 (CSL3 E107): bit 0 FAULT, 1 REMOTE, 2 STATE, 3 DTR.
    pub lights: u8,
    /// The 4x4 memories (CSL6 to CSL8, 74LS170): the word and the control
    /// bits for the COM/ADR cycle (odd ports) and for the data cycle (even
    /// ports). Control bits (ports 114 and 115, CSL8 E62): 0 DATA CYCLE,
    /// 1 I/O DATA CYCLE, 2 COM/ADR CYCLE, 3 BAD DATA CYCLE.
    pub com_adr: (W36, u8),
    pub data: (W36, u8),
    /// Register 204 (CSL4 E156): 0 CRA/M RESET, 1 STACK RESET, 2 DP
    /// RESET, 3 SS MODE, 4 CRAM ADR LOAD, 5 CRAM WRITE.
    pub cram_ctl: u8,
    /// Register 205 (CSL4 E155): 0 DIAG 1, 1 DIAG 2, 2 DIAG 4, 3 DIAG 8.
    /// Bit 4 (TRAP EN) is a processor input.
    pub diag: u8,
    /// CSL4 10 INT (E161): register 205 bit 5, preset by DPMB CSL
    /// INTERRUPT.
    pub ten_int: bool,
    /// CSL4 E171: INT 10 to the KS10 (register 116 bit 0), held until the
    /// KS10 clock runs.
    pub int_10: bool,
    /// Register 210 bits 0 to 5 (CSL4 E121), BUS REQ (E129) and MEM (CSL3
    /// E151).
    pub bus_ctl: u8,
    pub bus_req: bool,
    pub mem: bool,
    /// CSL3 NEXM (E145).
    pub nexm: bool,
    /// CSL4 DATA ACK (E129).
    pub data_ack: bool,
    /// The receive latches of the 8646 transceivers: the data lines and
    /// the control lines (the bits of `com_adr.1`).
    pub latch: (W36, u8),
    pub cycles: Vec<BusCycle>,
}

impl Default for Interface {
    fn default() -> Interface {
        Interface {
            power_up_reset: true,
            reset_pe: 0,
            lights: 0,
            com_adr: (W36::ZERO, 0),
            data: (W36::ZERO, 0),
            cram_ctl: 0,
            diag: 0,
            ten_int: false,
            int_10: false,
            bus_ctl: 0,
            bus_req: false,
            mem: false,
            nexm: false,
            data_ack: false,
            latch: (W36::ZERO, 0),
            cycles: Vec::new(),
        }
    }
}

/// Bits `k` of `v` (8080 bit k) put into DEC bits `last - k` of `w`, for
/// `n` bits.
fn put(w: W36, last: u32, n: u32, v: u8) -> W36 {
    (0..n).fold(w, |w, k| {
        w.with_field(last - k, last - k, u64::from(v >> k & 1))
    })
}

/// DEC bits `last - k` of `w` as 8080 bits k, for 8 bits.
fn take(w: W36, last: u32, n: u32) -> u8 {
    (0..n).fold(0, |v, k| v | u8::from(w.bit(last - k)) << k)
}

impl Interface {
    /// BUS RESET on the KS10 bus: CSL5 RESET H (E131) from RESET (1) or
    /// CSL3 RESET (register 100 bit 7), driven by CSL8 E167.
    pub fn bus_reset(&self) -> bool {
        self.power_up_reset || self.reset_pe & 0o200 != 0
    }

    /// A write to a KS10 port. `Ok(false)` if the port is not one of them.
    pub fn write(
        &mut self,
        port: u8,
        v: u8,
        now: u64,
        ks10: &mut dyn Ks10,
    ) -> Result<bool, &'static str> {
        let mut inputs = ks10.inputs();
        match port {
            // CSL3 E143: A0 selects 100 or 101; both end RESET (1).
            0o100 => {
                self.power_up_reset = false;
                self.reset_pe = v & 0o360;
                inputs.cache_en = v & 0o10 != 0;
                inputs.one_ms_en = v & 0o4 != 0;
            }
            0o101 => {
                self.power_up_reset = false;
                self.lights = v & 0o17;
            }
            // 74LS170 WB = ADR 0 L: an odd port writes the COM/ADR word.
            0o102..=0o113 => {
                let p = u32::from(port - 0o102) / 2;
                let n = if p == 4 { 4 } else { 8 };
                let loc = if port & 1 == 1 {
                    &mut self.com_adr
                } else {
                    &mut self.data
                };
                loc.0 = put(loc.0, 35 - 8 * p, n, v);
            }
            0o114 | 0o115 => {
                let loc = if port & 1 == 1 {
                    &mut self.com_adr
                } else {
                    &mut self.data
                };
                loc.1 = v & 0o17;
                // CSL3 E145, E135: the write clears NEXM and the NXM timer.
                self.nexm = false;
            }
            // CSL4 E171: bit 0 sets INT 10. It reaches the KS10 through
            // E170 and E163, which clock on CSL5 ENABLE (0) H, and E171
            // clears when it arrives. The model holds the request while the
            // KS10 clock is stopped.
            0o116 => self.int_10 |= v & 1 != 0,
            0o204 => self.cram_ctl = v & 0o77,
            0o205 => {
                if v & 0o100 != 0 {
                    return Err("the maintenance clock of the KS10 (register 205 bit 6)");
                }
                self.diag = v & 0o17;
                inputs.trap_en = v & 0o20 != 0;
                self.ten_int = v & 0o40 != 0;
            }
            0o206 if v & 0o3 != 0 => return Err("the KS10 clock (register 206)"),
            0o206 => {}
            0o210 if self.bus_reset() => self.bus_ctl = 0,
            0o210 => {
                self.bus_ctl = v & 0o77;
                self.mem = v & MEM != 0;
                if v & BUS_REQ != 0 {
                    self.bus_req = true;
                }
                self.bus_function(now, ks10)?;
                return Ok(true);
            }
            0o212 => {
                inputs.cont = v & 1 != 0;
                inputs.execute = v & 2 != 0;
                inputs.run = v & 4 != 0;
            }
            _ => return Ok(false),
        }
        if self.bus_reset() {
            self.reset(&mut inputs);
        }
        ks10.set_inputs(inputs);
        Ok(true)
    }

    /// CSL5 RESET L and RESETB L clear these registers (MP00540 CSL4 rev C:
    /// E155, E156, E170, E163, E121, E129 through E114; CSL3 rev D: E151
    /// MEM). A write while the reset holds is lost. CACHE EN, 1 MSEC EN,
    /// 10 INT and register 101 have no reset (CSL3 E151, E107; CSL4 E142,
    /// E161).
    fn reset(&mut self, inputs: &mut Inputs) {
        (self.cram_ctl, self.diag, self.bus_ctl) = (0, 0, 0);
        (self.bus_req, self.mem) = (false, false);
        (inputs.run, inputs.execute, inputs.cont, inputs.trap_en) = (false, false, false, false);
    }

    /// The bus function that register 210 starts (EK-OKS10-TM-002 Table
    /// 5-10).
    fn bus_function(&mut self, now: u64, ks10: &mut dyn Ks10) -> Result<(), &'static str> {
        let c = self.bus_ctl;
        if c & (CRA_R_CLK | CRA_T_CLK) != 0 {
            return Err("a diagnostic function of the CRA (register 210)");
        }
        if self.bus_req {
            // The console is the arbitrator (CSL1); with the KS10 clock
            // stopped it grants the bus at once, and GRANT clears BUS REQ
            // (CSL4 E114, E129).
            self.bus_req = false;
            let com_adr = self.com_adr.0;
            // EK-OKS10-TM-002 5.3.4: bit 0 I/O, bit 1 read, bit 2 write,
            // bit 6 byte; the I/O address is bits 14 to 35, the memory
            // address bits 16 to 35.
            let (io, read, write) = (com_adr.bit(0), com_adr.bit(1), com_adr.bit(2));
            let byte = com_adr.bit(6);
            let function = match (io, read, write) {
                (false, true, false) if c == XMIT_ADR | LATCH_DATA | CLOSE_LATCHES => {
                    let reply = ks10.memory_read(com_adr.field(16, 35) as u32)?;
                    // CSL3 E135, E134, E145: with MEM, no MEM BUSY within
                    // four R clocks sets NEXM.
                    if reply == Reply::None {
                        self.nexm |= self.mem;
                    }
                    Function::MemoryRead { com_adr, reply }
                }
                (true, true, false) if c == XMIT_ADR | LATCH_DATA | CLOSE_LATCHES => {
                    let reply = ks10.io_read(com_adr.field(14, 35) as u32, byte)?;
                    Function::IoRead { com_adr, reply }
                }
                (true, false, true) if c & (XMIT_ADR | XMIT_DATA) == XMIT_ADR | XMIT_DATA => {
                    let data = self.data.0;
                    let taken = ks10.io_write(com_adr.field(14, 35) as u32, data, byte)?;
                    Function::IoWrite {
                        com_adr,
                        data,
                        taken,
                    }
                }
                (false, false, true) if c & (XMIT_ADR | XMIT_DATA) == XMIT_ADR | XMIT_DATA => {
                    let data = self.data.0;
                    let taken = ks10.memory_write(com_adr.field(16, 35) as u32, data)?;
                    if !taken {
                        self.nexm |= self.mem;
                    }
                    Function::MemoryWrite {
                        com_adr,
                        data,
                        taken,
                    }
                }
                _ => return Err("a bus function of this kind (register 210)"),
            };
            if let Function::MemoryRead { reply, .. } | Function::IoRead { reply, .. } = function
                && let Reply::Data(w) = reply
            {
                self.latch = (w, 1);
                self.data_ack = true;
            }
            self.cycles.push(BusCycle { at: now, function });
            return Ok(());
        }
        match c {
            0 => Ok(()),
            // Examine bus: the latches take the bus. No device drives it,
            // so every line is deasserted.
            CLOSE_LATCHES => {
                self.latch = (W36::ZERO, 0);
                self.cycles.push(BusCycle {
                    at: now,
                    function: Function::ExamineBus { data: W36::ZERO },
                });
                Ok(())
            }
            _ => Err("a bus function without BUS REQ (register 210)"),
        }
    }

    /// A read of a KS10 port: the byte that the 8080 gets. The multiplexers
    /// of CSLA (74LS353, 74LS258) invert, so each bit is the complement of
    /// the level of its signal. `None` if the port is not one of them.
    pub fn read(
        &mut self,
        port: u8,
        switches: u8,
        carrier: (bool, bool),
        ks10: &dyn Ks10,
    ) -> Option<u8> {
        let level: u8 = match port {
            // CSL3 E93 Y0, the 8646 receive latches; a read clears DATA
            // ACK (CSL4 E129).
            0o000..=0o077 => {
                self.data_ack = false;
                let p = u32::from(port & 3);
                take(self.latch.0, 35 - 8 * p, 8)
            }
            0o100..=0o177 => match port & 3 {
                // RECIEVE PE, -ADPT3 PARITY ERR (high), not connected
                // (high), CRAM PE, -MEM PARITY ERR (high), DP PE, CRA PE,
                // -ADPT2 PARITY ERR (high).
                0 => 0b0110_1001,
                // PI REQ 1 to 7, REF ERR B.
                1 => 0,
                // R AC LO, R RESET, R MEM BUSY, R I/O BUSY, R BAD DATA,
                // R COM/ADR, R I/O DATA, R DATA: the bus is idle.
                2 => u8::from(self.bus_reset()) << 6,
                // -ADPT1 and -ADPT4 PARITY ERR (high), R PAR RIGHT and
                // LEFT (the model has no parity), R DATA 00 to 03.
                _ => 0b1100_0000 | take(self.latch.0, 3, 4),
            },
            0o200..=0o277 => return None,
            _ => match port & 3 {
                // E173 switches, HALT LOOP (1), RUN (1), EXECUTE B,
                // CONTINUE.
                0 => {
                    let i = ks10.inputs();
                    return Some(
                        switches & 0o360
                            | !(u8::from(ks10.halt_loop()) << 3
                                | u8::from(i.run) << 2
                                | u8::from(i.execute) << 1
                                | u8::from(i.cont))
                                & 0o17,
                    );
                }
                // 10 INT, NEXM, SPARE (high), BUS REQ, PE (1), PANEL
                // CONSOLE ENABLE (high), the inverse of PANEL BOOT L (high,
                // not pressed), DATA ACK.
                1 => {
                    u8::from(self.ten_int) << 7
                        | u8::from(self.nexm) << 6
                        | 1 << 5
                        | u8::from(self.bus_req) << 4
                        | 1 << 2
                        | u8::from(self.data_ack)
                }
                // Bits 4 to 7 have no driver (docs/unknowns.md); REMOTE
                // DIAG PROT and ENB (high), TERMINAL CARRIER, CARRIER.
                2 => 0b1111_1100 | u8::from(carrier.0) << 1 | u8::from(carrier.1),
                // R CLK ENB (0), CRAM CLK ENB, -DPE/M CLK ENABLE,
                // -RAM ERROR, with the KS10 clock stopped and no error.
                _ => 0b1111_1011,
            },
        };
        Some(!level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A KS10 with a memory of `n` words.
    struct Memory(Vec<W36>, Inputs);

    impl Ks10 for Memory {
        fn memory_read(&mut self, a: u32) -> Result<Reply, &'static str> {
            Ok(self
                .0
                .get(a as usize)
                .map_or(Reply::None, |&w| Reply::Data(w)))
        }
        fn memory_write(&mut self, a: u32, v: W36) -> Result<bool, &'static str> {
            Ok(self.0.get_mut(a as usize).map(|w| *w = v).is_some())
        }
        fn io_read(&mut self, _a: u32, _byte: bool) -> Result<Reply, &'static str> {
            Ok(Reply::None)
        }
        fn io_write(&mut self, _a: u32, _v: W36, _byte: bool) -> Result<bool, &'static str> {
            Ok(false)
        }
        fn idle_to(&mut self, _now: u64) {}
        fn inputs(&self) -> Inputs {
            self.1
        }
        fn set_inputs(&mut self, i: Inputs) {
            self.1 = i;
        }
        fn halt_loop(&self) -> bool {
            false
        }
    }

    /// The board after the power-up reset.
    fn after_reset() -> Interface {
        Interface {
            power_up_reset: false,
            ..Interface::default()
        }
    }

    /// The writes of `EM1` and `ADATP` for a memory read of `address`
    /// (KNS10.PNT): the address bytes to 103 to 113, the function 4 to 113,
    /// COM/ADR CYCLE to 115, then 343 to 210.
    fn examine(i: &mut Interface, k: &mut Memory, address: u32) -> Result<(), &'static str> {
        let a = address.to_le_bytes();
        for (port, v) in [
            (0o103, a[0]),
            (0o105, a[1]),
            (0o107, a[2]),
            (0o111, 0),
            (0o113, 0),
        ] {
            i.write(port, v, 0, k)?;
        }
        i.write(0o113, 4, 0, k)?;
        i.write(0o115, 4, 0, k)?;
        i.write(0o210, 0o343, 0, k)?;
        Ok(())
    }

    #[test]
    fn a_memory_read_latches_the_word() {
        let w = W36::wrap(0o123456_701234);
        let mut k = Memory(vec![W36::ZERO, w], Inputs::default());
        let mut i = after_reset();
        examine(&mut i, &mut k, 1).unwrap();
        // The command/address: bit 1 (read) and the address.
        assert_eq!(
            i.cycles[0].function,
            Function::MemoryRead {
                com_adr: W36::wrap(0o200000_000001),
                reply: Reply::Data(w)
            }
        );
        let sw = 0;
        let read = |i: &mut Interface, p| !i.read(p, sw, (false, false), &k).unwrap();
        // BUS REQ cleared, NEXM clear, DATA ACK (301 after CMA).
        assert_eq!(read(&mut i, 0o301) & 0o121, 0o001);
        // Ports 0 to 3 and 103, after CMA: bits 28-35 first (RDATP).
        let bytes: Vec<u8> = [0, 1, 2, 3].iter().map(|&p| read(&mut i, p)).collect();
        assert_eq!(bytes, [0o234, 0o202, 0o273, 0o234]);
        assert_eq!(read(&mut i, 0o103) & 0o17, 0o2);
        // A read of port 0 cleared DATA ACK.
        assert_eq!(read(&mut i, 0o301) & 1, 0);
    }

    #[test]
    fn a_read_without_memory_sets_nexm() {
        let mut k = Memory(vec![], Inputs::default());
        let mut i = after_reset();
        examine(&mut i, &mut k, 0).unwrap();
        let s = !i.read(0o301, 0, (false, false), &k).unwrap();
        assert_eq!(s & 0o101, 0o100);
        // A write to 114 clears it (CSL3 E145).
        i.write(0o114, 0, 0, &mut k).unwrap();
        assert_eq!(!i.read(0o301, 0, (false, false), &k).unwrap() & 0o100, 0);
    }

    #[test]
    fn register_212_and_300_are_the_processor_inputs() {
        let mut k = Memory(vec![], Inputs::default());
        let mut i = after_reset();
        i.write(0o212, 0o5, 0, &mut k).unwrap();
        assert_eq!(
            k.1,
            Inputs {
                run: true,
                cont: true,
                ..Inputs::default()
            }
        );
        // Port 300 after CMA: bit 2 RUN and bit 0 CONTINUE are 1.
        assert_eq!(!i.read(0o300, 0o360, (false, false), &k).unwrap(), 0o5);
    }

    #[test]
    fn the_reset_clears_the_registers_of_the_cpu_control() {
        let mut k = Memory(vec![], Inputs::default());
        let mut i = Interface::default();
        // MRINT at power-up: 204/5 while RESET (1) holds is lost.
        i.write(0o204, 0o5, 0, &mut k).unwrap();
        assert_eq!(i.cram_ctl, 0);
        i.write(0o100, 0o174, 0, &mut k).unwrap();
        i.write(0o204, 0o5, 0, &mut k).unwrap();
        i.write(0o205, 0o20, 0, &mut k).unwrap();
        i.write(0o212, 0o7, 0, &mut k).unwrap();
        assert_eq!((i.cram_ctl, k.1.trap_en, k.1.run), (0o5, true, true));
        // Register 100 bit 7 (CSL3 RESET) clears them, but not CACHE EN.
        i.write(0o100, 0o210, 0, &mut k).unwrap();
        assert_eq!((i.cram_ctl, k.1.trap_en, k.1.run), (0, false, false));
        assert!(k.1.cache_en);
    }

    #[test]
    fn a_write_to_100_ends_the_bus_reset() {
        let mut k = Memory(vec![], Inputs::default());
        let mut i = Interface::default();
        assert!(i.bus_reset());
        i.write(0o100, 0o200, 0, &mut k).unwrap();
        assert!(i.bus_reset());
        i.write(0o100, 0o174, 0, &mut k).unwrap();
        assert!(!i.bus_reset());
        assert!(k.1.cache_en && k.1.one_ms_en);
    }
}

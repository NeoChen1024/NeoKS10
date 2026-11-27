//! The CH11 Chaosnet interface (spec/devices.toml: `ch11`; the instance
//! `chaos`).
//!
//! Source: **[its]** ch11.defs1, the comments of each register:
//!
//! - Transmit: the first word written to WBF resets the transmitter and
//!   clears TDN; the words of the message follow, the last is the
//!   destination. A read of XMT starts the transmission and gives the own
//!   host number. TDN is set when the transmitter is done.
//! - Receive: RDN says a message is in the buffer; RBF gives its words,
//!   then destination, source and check word; RBC is one less than the
//!   number of bits. RCL clears the receiver for the next message.
//! - Interrupts: receive done with REN, transmit done with TEN.
//!
//! In this version no network is connected: with loop back (LUP) a message
//! comes back to the receiver, and without it the message is dropped. The
//! check word is not computed (docs/unknowns.md).

use std::collections::VecDeque;

use crate::devregs::{Instance, ch11 as c};
use crate::unibus::Event;

/// The time of one transmission, in instructions. **[estimate]**: a
/// message of 512 bytes at 4 Mbit/s takes about 1 ms.
pub const XMT_TIME: u64 = 1000;

/// The words of the hardware buffers: 512 bytes of data and 3 more.
const BUFFER: usize = 256 + 3;

pub struct Ch11 {
    pub inst: Instance,
    /// The host number (ITS config.202: MYCHAD).
    pub address: u16,
    /// The bits LUP, SPY, REN, TEN of CSR.
    pub csr: u16,
    pub tdn: bool,
    pub rdn: bool,
    pub xmt: Vec<u16>,
    /// The first WBF write after a transmission starts a new message.
    xmt_new: bool,
    pub rcv: VecDeque<u16>,
    pub rcv_bits: u16,
    pub lost: u16,
    pub busy: bool,
    pub rx_irq: bool,
    pub tx_irq: bool,
}

impl Ch11 {
    pub fn new(inst: Instance, address: u16) -> Self {
        Ch11 {
            inst,
            address,
            csr: 0,
            tdn: true,
            rdn: false,
            xmt: Vec::new(),
            xmt_new: true,
            rcv: VecDeque::new(),
            rcv_bits: 0o7777,
            lost: 0,
            busy: false,
            rx_irq: false,
            tx_irq: false,
        }
    }

    pub fn offset(&self, a: u32, ub: u32) -> Option<u32> {
        let b = self.inst.base;
        (a == self.inst.adapter && (b..b + 0o20).contains(&ub)).then(|| (ub - b) & !1)
    }

    /// I/O reset (RST) and Unibus init.
    pub fn reset(&mut self) {
        self.csr = 0;
        self.tdn = true;
        self.rdn = false;
        self.xmt.clear();
        self.xmt_new = true;
        self.rcv.clear();
        self.rcv_bits = 0o7777;
        self.lost = 0;
        self.busy = false;
        self.rx_irq = false;
        self.tx_irq = false;
    }

    fn csr(&self) -> u16 {
        let mut v = self.csr;
        if self.busy {
            v |= c::csr::BSY as u16;
        }
        if self.tdn {
            v |= c::csr::TDN as u16;
        }
        if self.rdn {
            v |= c::csr::RDN as u16;
        }
        v | ((self.lost.min(0o17)) << c::csr::LOS_LSB)
    }

    fn update_irq(&mut self) {
        self.rx_irq = self.rdn && self.csr & c::csr::REN as u16 != 0;
        self.tx_irq = self.tdn && self.csr & c::csr::TEN as u16 != 0;
    }

    /// Put a message (data words and the destination) into the receiver.
    fn receive(&mut self, msg: &[u16], source: u16) {
        if self.rdn {
            self.lost = self.lost.saturating_add(1);
            return;
        }
        self.rcv = msg.iter().copied().collect();
        self.rcv.push_back(source);
        self.rcv.push_back(0);
        self.rcv_bits = (self.rcv.len() as u16 * 16).wrapping_sub(1) & 0o7777;
        self.rdn = true;
    }

    pub fn read(&mut self, off: u32, now: u64, events: &mut Vec<(u64, Event)>) -> u16 {
        match off {
            c::CSR => self.csr(),
            c::MYN => self.address,
            c::RBF => {
                let v = self.rcv.pop_front().unwrap_or(0);
                if self.rcv.is_empty() {
                    self.rcv_bits = 0o7777;
                }
                v
            }
            c::RBC => self.rcv_bits,
            c::XMT => {
                if !self.busy && !self.xmt.is_empty() {
                    self.busy = true;
                    self.tdn = false;
                    self.tx_irq = false;
                    events.push((now + XMT_TIME, Event::Chaos));
                }
                self.address
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, value: u16, mask: u16) {
        match off {
            c::CSR => {
                let v = value & mask;
                if v & c::csr::RST as u16 != 0 {
                    self.reset();
                    return;
                }
                let rw = (c::csr::LUP | c::csr::SPY | c::csr::REN | c::csr::TEN) as u16;
                self.csr = (self.csr & !(rw & mask)) | (v & rw);
                if v & c::csr::RCL as u16 != 0 {
                    self.rdn = false;
                    self.rcv.clear();
                    self.rcv_bits = 0o7777;
                    self.lost = 0;
                }
                if v & c::csr::TCL as u16 != 0 {
                    self.xmt.clear();
                    self.xmt_new = true;
                    self.busy = false;
                    self.tdn = true;
                }
                self.update_irq();
            }
            c::MYN => {
                // WBF
                if self.xmt_new {
                    self.xmt.clear();
                    self.xmt_new = false;
                    self.tdn = false;
                    self.tx_irq = false;
                }
                if self.xmt.len() < BUFFER {
                    self.xmt.push(value & mask);
                }
            }
            _ => {}
        }
    }

    /// The transmission ends.
    pub fn done(&mut self) {
        self.busy = false;
        self.tdn = true;
        self.xmt_new = true;
        let msg = std::mem::take(&mut self.xmt);
        if self.csr & c::csr::LUP as u16 != 0 {
            self.receive(&msg, self.address);
        }
        self.update_irq();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devregs;

    // ch11.defs1: words to WBF, the destination last, then a read of XMT;
    // with loop back the message comes back with source and check word.
    #[test]
    fn loop_back() {
        let mut ch = Ch11::new(devregs::CHAOS, 0o177002);
        let mut ev = Vec::new();
        ch.write(c::CSR, (c::csr::LUP | c::csr::REN) as u16, 0o177777);
        for w in [0o1, 0o2, 0o177002] {
            ch.write(c::MYN, w, 0o177777);
        }
        assert_eq!(ch.read(c::XMT, 0, &mut ev), 0o177002);
        assert_eq!(ch.read(c::CSR, 0, &mut ev) & c::csr::TDN as u16, 0);
        ch.done();
        assert!(ch.rx_irq);
        assert_eq!(ch.read(c::RBC, 0, &mut ev), 5 * 16 - 1);
        let words: Vec<u16> = (0..5).map(|_| ch.read(c::RBF, 0, &mut ev)).collect();
        assert_eq!(words, [0o1, 0o2, 0o177002, 0o177002, 0]);
        assert_eq!(ch.read(c::RBC, 0, &mut ev), 0o7777);
    }
}

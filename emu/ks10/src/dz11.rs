//! The DZ11 asynchronous multiplexer, 8 lines (spec/devices.toml: `dz11`;
//! the instance `dz0`).
//!
//! Sources: **[its]** dz11.10 and ts3tty.402 (DZINIT, DZSCAN, the DZ
//! interrupt routines); **[manual]** DZ11 user's guide and technical
//! manual (references/):
//!
//! - Receive: characters go into a silo of 64 entries. RBUF gives the
//!   oldest, with Data Valid; RDONE says the silo is not empty. With
//!   Silo Alarm Enable, the receiver interrupts after 16 characters (SA)
//!   instead of each one.
//! - Transmit: with Master Scan Enable, the scanner finds the next line
//!   whose bit is set in the low byte of TCR and whose transmitter is
//!   free. Then TRDY is set and TLINE names the line. A write of the low
//!   byte of TDR sends a character on that line.
//! - Interrupts: receive (vector) with RIE and RDONE (or SA); transmit
//!   (vector + 4) with TIE and TRDY.
//!
//! The host side of each line is a queue in each direction; the host
//! layer connects them to terminals. A line without a host has no carrier.

use std::collections::VecDeque;

use crate::devregs::{Instance, dz11 as d};
use crate::unibus::Event;

/// The time to send one character, in instructions. **[estimate]**: 9600
/// baud (about 1 ms for 10 bits) at 1 µs for each instruction.
pub const CHAR_TIME: u64 = 1000;

const SILO: usize = 64;
const ALARM: usize = 16;

#[derive(Clone, Debug, Default)]
pub struct Line {
    /// Characters from the host, not yet in the silo.
    pub input: VecDeque<u8>,
    /// Characters the line sent to the host.
    pub output: Vec<u8>,
    /// A host is connected (carrier detect).
    pub carrier: bool,
    /// The line parameters as written (LPR without the line number).
    pub lpr: u16,
    /// The transmitter sends a character until this instruction count.
    pub busy_until: u64,
}

pub struct Dz11 {
    pub inst: Instance,
    pub csr: u16,
    pub tcr: u16,
    pub silo: VecDeque<u16>,
    pub lines: [Line; 8],
    /// The interrupt requests: receive and transmit.
    pub rx_irq: bool,
    pub tx_irq: bool,
    /// The next line for the transmit scanner.
    scan: usize,
    /// An event for the scanner is pending.
    pub tick_pending: bool,
    /// The instruction count of the last receive poll.
    last_poll: u64,
    /// The receive interrupt condition at the last update.
    rx_cond: bool,
}

impl Dz11 {
    pub fn new(inst: Instance) -> Self {
        Dz11 {
            inst,
            csr: 0,
            tcr: 0,
            silo: VecDeque::new(),
            lines: Default::default(),
            rx_irq: false,
            tx_irq: false,
            scan: 0,
            tick_pending: false,
            last_poll: 0,
            rx_cond: false,
        }
    }

    pub fn offset(&self, a: u32, ub: u32) -> Option<u32> {
        let b = self.inst.base;
        (a == self.inst.adapter && (b..b + 0o10).contains(&ub)).then(|| (ub - b) & !1)
    }

    /// The Clear bit and Unibus init.
    pub fn reset(&mut self) {
        self.csr = 0;
        self.tcr = 0;
        self.silo.clear();
        self.rx_irq = false;
        self.tx_irq = false;
        self.rx_cond = false;
        self.scan = 0;
        for l in &mut self.lines {
            l.lpr = 0;
            l.busy_until = 0;
        }
    }

    fn msr(&self) -> u16 {
        let mut cd = 0;
        for (k, l) in self.lines.iter().enumerate() {
            if l.carrier {
                cd |= 1 << k;
            }
        }
        cd << 8
    }

    fn rdone(&self) -> bool {
        self.csr & d::csr::MSE as u16 != 0 && !self.silo.is_empty()
    }

    /// Update RDONE, SA and the receive request. The request starts when
    /// its condition becomes true and ends when it is false.
    fn update_rx(&mut self) {
        let mut c = self.csr & !((d::csr::RDONE | d::csr::SA) as u16);
        let done = self.rdone();
        let alarm = self.silo.len() >= ALARM;
        if done {
            c |= d::csr::RDONE as u16;
        }
        let sae = c & d::csr::SAE as u16 != 0;
        if alarm && sae {
            c |= d::csr::SA as u16;
        }
        self.csr = c;
        let want = c & d::csr::RIE as u16 != 0 && if sae { alarm } else { done };
        if want && !self.rx_cond {
            self.rx_irq = true;
        }
        if !want {
            self.rx_irq = false;
        }
        self.rx_cond = want;
    }

    /// The transmit scanner: find the next enabled, free line.
    fn update_tx(&mut self, now: u64, events: &mut Vec<(u64, Event)>) {
        if self.csr & d::csr::TRDY as u16 != 0 {
            return;
        }
        if self.csr & d::csr::MSE as u16 == 0 {
            return;
        }
        let mut wake = None;
        for k in 0..8 {
            let n = (self.scan + k) % 8;
            if self.tcr & (1 << n) == 0 {
                continue;
            }
            let l = &self.lines[n];
            if l.busy_until <= now {
                self.csr =
                    (self.csr & !(d::csr::TLINE as u16)) | ((n as u16) << 8) | d::csr::TRDY as u16;
                self.scan = (n + 1) % 8;
                if self.csr & d::csr::TIE as u16 != 0 {
                    self.tx_irq = true;
                }
                return;
            }
            wake = Some(wake.map_or(l.busy_until, |w: u64| w.min(l.busy_until)));
        }
        if let Some(t) = wake
            && !self.tick_pending
        {
            self.tick_pending = true;
            events.push((t, Event::Dz));
        }
    }

    /// Move host input into the silo, one character for each line and
    /// character time (a line cannot receive faster than its speed).
    pub fn poll(&mut self, now: u64, events: &mut Vec<(u64, Event)>) {
        if now >= self.last_poll + CHAR_TIME {
            self.last_poll = now;
            for (k, l) in self.lines.iter_mut().enumerate() {
                if l.lpr & d::rbuf::LPR_RXON as u16 == 0 {
                    continue;
                }
                if let Some(c) = l.input.pop_front() {
                    if self.silo.len() < SILO {
                        self.silo
                            .push_back(d::rbuf::VALID as u16 | ((k as u16) << 8) | u16::from(c));
                    } else if let Some(last) = self.silo.back_mut() {
                        *last |= d::rbuf::OVR as u16;
                    }
                }
            }
        }
        self.update_rx();
        self.update_tx(now, events);
    }

    /// The first instruction count at which [`Dz11::poll`] can change
    /// the state, if the processor does no I/O: the next receive poll,
    /// or the end of a busy transmitter that the scanner waits for.
    pub fn deadline(&self) -> u64 {
        let mut t = self.last_poll + CHAR_TIME;
        if self.csr & d::csr::TRDY as u16 == 0 && self.csr & d::csr::MSE as u16 != 0 {
            for (n, l) in self.lines.iter().enumerate() {
                if self.tcr & (1 << n) != 0 {
                    t = t.min(l.busy_until);
                }
            }
        }
        t
    }

    pub fn read(&mut self, off: u32) -> u16 {
        match off {
            d::CSR => self.csr,
            d::RBUF => {
                let v = self.silo.pop_front().unwrap_or(0);
                self.update_rx();
                v
            }
            d::TCR => self.tcr,
            _ => self.msr(),
        }
    }

    pub fn write(
        &mut self,
        off: u32,
        value: u16,
        mask: u16,
        now: u64,
        events: &mut Vec<(u64, Event)>,
    ) {
        match off {
            d::CSR => {
                let v = (self.csr & !mask) | (value & mask);
                if v & d::csr::CLR as u16 != 0 {
                    self.reset();
                    return;
                }
                let rw =
                    (d::csr::MAINT | d::csr::MSE | d::csr::RIE | d::csr::SAE | d::csr::TIE) as u16;
                let old = self.csr;
                self.csr = (self.csr & !rw) | (v & rw);
                if self.csr & d::csr::TIE as u16 == 0 {
                    self.tx_irq = false;
                } else if old & d::csr::TIE as u16 == 0 && self.csr & d::csr::TRDY as u16 != 0 {
                    self.tx_irq = true;
                }
                self.update_rx();
                self.update_tx(now, events);
            }
            d::RBUF => {
                // LPR: the parameters of one line.
                let v = value & mask;
                let n = (v & d::rbuf::LPR_LINE as u16) as usize;
                self.lines[n].lpr = v & !(d::rbuf::LPR_LINE as u16);
            }
            d::TCR => {
                self.tcr = (self.tcr & !mask) | (value & mask);
                // A line that is turned off while it holds TRDY releases
                // the scanner (DZ11 user's guide, "Transmitter Control
                // Register": the scanner looks only at enabled lines).
                let n = (self.csr & d::csr::TLINE as u16) >> 8;
                if self.csr & d::csr::TRDY as u16 != 0 && self.tcr & (1 << n) == 0 {
                    self.csr &= !(d::csr::TRDY as u16);
                    self.tx_irq = false;
                }
                self.update_tx(now, events);
            }
            _ => {
                // TDR: a character for the line that TLINE names.
                if mask & 0o377 != 0 && self.csr & d::csr::TRDY as u16 != 0 {
                    let n = ((self.csr & d::csr::TLINE as u16) >> 8) as usize;
                    let l = &mut self.lines[n];
                    l.output.push((value & 0o377) as u8);
                    l.busy_until = now + CHAR_TIME;
                    self.csr &= !(d::csr::TRDY as u16);
                    self.tx_irq = false;
                    self.update_tx(now, events);
                }
            }
        }
    }

    /// The scanner event: a transmitter is free again.
    pub fn tick(&mut self, now: u64, events: &mut Vec<(u64, Event)>) {
        self.tick_pending = false;
        self.update_tx(now, events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devregs;

    // ts3tty.402, DZINIT: enable the board with TIE, RIE, SAE and MSE, set
    // all DTR bits. A line enabled in TCR then gets TRDY with its number
    // in TLINE; a TDR write sends a character.
    #[test]
    fn transmit_scan() {
        let mut z = Dz11::new(devregs::DZ0);
        let mut ev = Vec::new();
        let on = (d::csr::TIE | d::csr::RIE | d::csr::SAE | d::csr::MSE) as u16;
        z.write(d::CSR, on, 0o177777, 0, &mut ev);
        z.write(d::TCR, (0o377 << 8) | (1 << 3), 0o177777, 0, &mut ev);
        let c = z.read(d::CSR);
        assert_ne!(c & d::csr::TRDY as u16, 0);
        assert_eq!((c & d::csr::TLINE as u16) >> 8, 3);
        assert!(z.tx_irq);
        z.write(d::MSR, u16::from(b'A'), 0o377, 0, &mut ev);
        assert_eq!(z.lines[3].output, b"A");
        assert_eq!(z.read(d::CSR) & d::csr::TRDY as u16, 0);
        // The line is busy for one character time.
        assert_eq!(ev, vec![(CHAR_TIME, Event::Dz)]);
        z.tick(CHAR_TIME, &mut ev);
        assert_ne!(z.read(d::CSR) & d::csr::TRDY as u16, 0);
        // Turning the line off releases TRDY (ts3tty.402: the transmit
        // interrupt routine clears the TCR bit of a line with no more
        // output).
        z.write(d::TCR, 0o377 << 8, 0o177777, CHAR_TIME, &mut ev);
        assert_eq!(z.read(d::CSR) & d::csr::TRDY as u16, 0);
        assert!(!z.tx_irq);
    }

    // dz11.10: RDR has Data Valid (100000), the line in bits 8-10 and the
    // character.
    #[test]
    fn receive() {
        let mut z = Dz11::new(devregs::DZ0);
        let mut ev = Vec::new();
        z.write(
            d::CSR,
            (d::csr::MSE | d::csr::RIE) as u16,
            0o177777,
            0,
            &mut ev,
        );
        z.write(d::RBUF, 5 | d::rbuf::LPR_RXON as u16, 0o177777, 0, &mut ev);
        z.lines[5].input.push_back(b'x');
        z.poll(CHAR_TIME, &mut ev);
        assert!(z.rx_irq);
        assert_ne!(z.read(d::CSR) & d::csr::RDONE as u16, 0);
        assert_eq!(z.read(d::RBUF), 0o100000 | (5 << 8) | u16::from(b'x'));
        assert_eq!(z.read(d::RBUF) & d::rbuf::VALID as u16, 0);
        assert!(!z.rx_irq);
    }
}

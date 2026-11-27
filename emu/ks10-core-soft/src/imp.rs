//! The ACC LH/DH IMP interface (spec/devices.toml: `lhdh`; the instance
//! `imp`), without an IMP for now.
//!
//! Source: **[its]** lhdh.defs5: two Unibus devices, input and output, with
//! DMA; the reset bits of the two sides are wired together; RDY says that
//! the registers can change; the input side reports "IMP not ready" and
//! "host ready" (the relay that HRC closes).
//!
//! In this version no IMP is connected: the input side always has INR,
//! an input transfer does not end, and an output transfer ends at once
//! and drops its data. The IMP of the simulated simple IMP (SSIMP) comes
//! with the host network layer.

use crate::devregs::{Instance, lhdh as l};

pub struct Imp {
    pub inst: Instance,
    /// The program bits of the two CSRs (IE, A17, A16, and SE and HRC on
    /// the input side, BB and ELB on the output side).
    pub ics: u16,
    pub ocs: u16,
    pub iwc: u16,
    pub ica: u16,
    pub owc: u16,
    pub oca: u16,
    /// An input transfer waits for data.
    pub input_busy: bool,
    pub rx_irq: bool,
    pub tx_irq: bool,
}

impl Imp {
    pub fn new(inst: Instance) -> Self {
        Imp {
            inst,
            ics: 0,
            ocs: 0,
            iwc: 0,
            ica: 0,
            owc: 0,
            oca: 0,
            input_busy: false,
            rx_irq: false,
            tx_irq: false,
        }
    }

    pub fn offset(&self, a: u32, ub: u32) -> Option<u32> {
        let b = self.inst.base;
        (a == self.inst.adapter && (b..b + 0o20).contains(&ub)).then(|| (ub - b) & !1)
    }

    pub fn reset(&mut self) {
        let inst = self.inst;
        *self = Imp::new(inst);
    }

    pub fn read(&self, off: u32) -> u16 {
        match off {
            l::ICS => {
                let mut v = self.ics | l::ics::INR as u16;
                if !self.input_busy {
                    v |= l::ics::RDY as u16;
                }
                if self.ics & l::ics::HRC as u16 != 0 {
                    v |= l::ics::HR as u16;
                }
                v
            }
            l::IWC => self.iwc,
            l::ICA => self.ica,
            l::OCS => {
                let mut v = self.ocs | (l::ocs::RDY | l::ocs::OBE) as u16;
                if self.owc == 0 {
                    v |= l::ocs::WC0 as u16;
                }
                v
            }
            l::OWC => self.owc,
            l::OCA => self.oca,
            _ => 0,
        }
    }

    pub fn write(&mut self, off: u32, value: u16, mask: u16) {
        let v = value & mask;
        match off {
            l::ICS | l::OCS => {
                if v & l::ics::RST as u16 != 0 {
                    self.reset();
                    return;
                }
                if off == l::ICS {
                    let rw =
                        (l::ics::IE | l::ics::A17 | l::ics::A16 | l::ics::SE | l::ics::HRC) as u16;
                    self.ics = (self.ics & !(rw & mask)) | (v & rw);
                    if v & l::ics::GO as u16 != 0 {
                        self.input_busy = true;
                    }
                    self.rx_irq = false;
                } else {
                    let rw =
                        (l::ocs::IE | l::ocs::A17 | l::ocs::A16 | l::ocs::BB | l::ocs::ELB) as u16;
                    self.ocs = (self.ocs & !(rw & mask)) | (v & rw);
                    self.tx_irq = false;
                    if v & l::ocs::GO as u16 != 0 {
                        // No IMP: the data go nowhere. The address
                        // register stays: how it counts in 32-bit mode is
                        // not known (docs/unknowns.md).
                        self.owc = 0;
                        if self.ocs & l::ocs::IE as u16 != 0 {
                            self.tx_irq = true;
                        }
                    }
                }
            }
            l::IWC => self.iwc = v,
            l::ICA => self.ica = v,
            l::OWC => self.owc = v,
            l::OCA => self.oca = v,
            _ => {}
        }
    }
}

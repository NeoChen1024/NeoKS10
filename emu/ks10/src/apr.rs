//! The APR and the PI system as I/O registers: APRID, WRAPR, RDAPR, WRPI,
//! RDPI, CONSZ and CONSO of APR and PI (opcode 700, dispatch on AC;
//! spec/instructions.toml).
//!
//! The formats come from SYSTEM;KSDEFS **[its]** (`ksdefs.194`, RDAPR,
//! WRAPR, RDPI, WRPI). KSDEFS numbers bits as "q.b": quarter q (4 = bits 0
//! to 8, ..., 1 = bits 27 to 35) and bit b of it from the right, so q.b is
//! word bit 9 * (4 - q) + 9 - b. This version keeps the state; the delivery
//! of interrupts comes later.

use pdp10::{Half, W36};

use crate::machine::{Fault, Inst, Machine, Next};

type R = Result<Next, Fault>;

/// APRID of the ITS microcode **[microcode]**: options 020 and version 262
/// (decimal) in the left half, hardware options 0 and serial number 1729
/// (decimal) in the right half (docs/discrepancies.md, "APRID").
pub const APRID: W36 = W36::from_halves(Half::wrap(0o020406), Half::wrap(0o003301));

/// The APR flags, in the order of KSDEFS bits 2.3 to 1.5 (word bits 24 to
/// 31): bit 7 of the byte is word bit 24.
pub mod flag {
    pub const FLAG24: u8 = 1 << 7;
    pub const KS_TO_8080: u8 = 1 << 6;
    pub const POWER_FAIL: u8 = 1 << 5;
    pub const NO_MEMORY: u8 = 1 << 4;
    pub const BAD_MEMORY: u8 = 1 << 3;
    pub const CORRECTED: u8 = 1 << 2;
    pub const INTERVAL: u8 = 1 << 1;
    pub const FROM_8080: u8 = 1 << 0;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Apr {
    pub enabled: u8,
    pub flags: u8,
    /// PI level of the APR interrupt, 0 to 7.
    pub level: u8,
}

impl Apr {
    /// An enabled flag is set: the APR requests an interrupt (RDAPR 1.4).
    pub fn requesting(&self) -> bool {
        self.enabled & self.flags != 0
    }

    /// The RDAPR word: enabled flags in 4.3 to 3.5 (word bits 6 to 13),
    /// flags in 2.3 to 1.5 (24 to 31), 1.4 (32) if requesting, the level in
    /// 1.3 to 1.1 (33 to 35).
    pub fn word(&self) -> W36 {
        let lh = u64::from(self.enabled) << 4;
        let rh = (u64::from(self.flags) << 4)
            | (u64::from(self.requesting()) << 3)
            | u64::from(self.level);
        W36::from_halves(Half::wrap(lh), Half::wrap(rh))
    }

    /// WRAPR with E: functions 2.7 enable, 2.6 disable, 2.5 clear, 2.4 set
    /// (word bits 20 to 23) on the flags selected by 2.3 to 1.5; the level
    /// from 1.3 to 1.1.
    pub fn write(&mut self, e: Half) {
        let sel = e.field(6, 13) as u8;
        if e.bit(2) {
            self.enabled |= sel;
        }
        if e.bit(3) {
            self.enabled &= !sel;
        }
        if e.bit(4) {
            self.flags &= !sel;
        }
        if e.bit(5) {
            self.flags |= sel;
        }
        self.level = e.field(15, 17) as u8;
    }
}

/// The PI system. Levels 1 to 7 are bits 6 to 0 of the masks (level 1 is
/// word bit 29 in the KSDEFS formats).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pi {
    pub on: bool,
    pub active: u8,
    pub requested: u8,
    pub in_progress: u8,
}

impl Pi {
    /// The RDPI word: requests made by WRPI in 3.7 to 3.1 (word bits 11 to
    /// 17), interrupts in progress in 2.6 to 1.9 (21 to 27), the system on
    /// in 1.8 (28), the active levels in 1.7 to 1.1 (29 to 35).
    pub fn word(&self) -> W36 {
        let lh = u64::from(self.requested);
        let rh =
            (u64::from(self.in_progress) << 8) | (u64::from(self.on) << 7) | u64::from(self.active);
        W36::from_halves(Half::wrap(lh), Half::wrap(rh))
    }

    /// WRPI with E: 2.5 drop requests, 2.4 clear the PI system, 2.3
    /// request interrupts, 2.2 turn on levels, 2.1 turn off levels (word
    /// bits 22 to 26), 1.9 turn off the system, 1.8 turn it on (27, 28), on
    /// the levels selected by 1.7 to 1.1. The order of the functions is the
    /// one of the microcode **[microcode]** (inout.50, WRPI): clear, drop,
    /// request, system on, system off, levels on, levels off. Returns false
    /// if one of word bits 18 to 21 is set (PI.MBZ, ks10.47): the
    /// microcode then does a UUO after the clear.
    pub fn write(&mut self, e: Half) -> bool {
        let sel = e.field(11, 17) as u8;
        if e.bit(5) {
            *self = Pi::default();
        }
        if e.field(0, 3) != 0 {
            return false;
        }
        if e.bit(4) {
            self.requested &= !sel;
        }
        if e.bit(6) {
            self.requested |= sel;
        }
        if e.bit(10) {
            self.on = true;
        }
        if e.bit(9) {
            self.on = false;
        }
        if e.bit(7) {
            self.active |= sel;
        }
        if e.bit(8) {
            self.active &= !sel;
        }
        true
    }

    /// The mask bit of level n (1 to 7).
    pub fn level_bit(n: u8) -> u8 {
        1 << (7 - n)
    }

    /// The level that the processor accepts now, if any (PRM 4.1,
    /// "Processing an Interrupt"): the system is on, and the highest
    /// priority level with a recognized request is above every level in
    /// progress. A request is recognized on an active level only; for the
    /// program requests this is the rule of the microcode
    /// **[microcode]** (inout.50, LOADPI: "ONLY REQUEST CHANS THAT ARE ON";
    /// docs/discrepancies.md). `other` holds the requests of the APR and
    /// the devices.
    pub fn accept(&self, other: u8) -> Option<u8> {
        if !self.on {
            return None;
        }
        let recognized = (self.requested | other) & self.active;
        let n = (1..=7).find(|&n| recognized & Self::level_bit(n) != 0)?;
        let busy = (1..=7).find(|&k| self.in_progress & Self::level_bit(k) != 0);
        match busy {
            Some(k) if k <= n => None,
            _ => Some(n),
        }
    }

    /// Dismiss the highest priority interrupt in progress (inout.50,
    /// DISMISS).
    pub fn dismiss(&mut self) {
        if let Some(k) = (1..=7).find(|&k| self.in_progress & Self::level_bit(k) != 0) {
            self.in_progress &= !Self::level_bit(k);
        }
    }
}

/// Opcode 700 by AC (spec/instructions.toml, acdisp 700).
pub fn op700(m: &mut Machine, i: Inst, e: Half) -> R {
    if m.flags.is_user() && !m.flags.has(crate::flags::Flags::USERIO) {
        // IOT: I/O in user mode needs User In-out (ks10.47, "SKIP IO
        // LEGAL"; A/IOT in the DRAM entry); else a monitor UUO.
        return Err(Fault::Uuo);
    }
    match i.ac {
        0 => m.write(e, APRID)?,
        4 => m.apr.write(e),
        5 => m.write(e, m.apr.word())?,
        // CONSZ, CONSO APR: [microcode] inout.50, APRSZ and APRSO test the
        // right half of the APR word with the mask 7770.
        6 | 7 => return Ok(test(m.apr.word().rh().get() & 0o7770, e, i.ac == 7)),
        0o14 => {
            if !m.pi.write(e) {
                return Err(Fault::Uuo);
            }
        }
        0o15 => m.write(e, m.pi.word())?,
        // CONSZ, CONSO PI: the RDPI word (inout.50, [BR]_[PI]).
        0o16 | 0o17 => return Ok(test(m.pi.word().get(), e, i.ac == 0o17)),
        _ => return Err(Fault::Unimplemented),
    }
    Ok(Next::Step)
}

/// CONSZ (skip if all bits of value AND E are 0) or CONSO (skip if not).
fn test(value: u64, e: Half, conso: bool) -> Next {
    let zero = value & e.get() == 0;
    if zero != conso {
        Next::Skip
    } else {
        Next::Step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Values from the KSDEFS texts: CONO APR with 2.4 (set, word bit 23)
    // and 1.6 (interval done, bit 30) sets the flag; RDAPR shows it at 1.6.
    #[test]
    fn apr_set_enable_read() {
        let mut a = Apr::default();
        a.write(Half::wrap(0o010040 | 3)); // set interval done, level 3
        assert_eq!(a.flags, flag::INTERVAL);
        assert_eq!(a.word().rh().get(), 0o000040 | 3);
        a.write(Half::wrap(0o100040 | 3)); // enable interval done
        assert!(a.requesting());
        assert_eq!(
            a.word(),
            W36::from_halves(Half::wrap(0o000040), Half::wrap(0o000053))
        );
        a.write(Half::wrap(0o020040)); // clear it
        assert_eq!(a.flags, 0);
    }

    // CONO PI,2200+177 (ITS: turn on the system and all levels).
    #[test]
    fn pi_on() {
        let mut p = Pi::default();
        assert!(p.write(Half::wrap(0o002377)));
        assert!(p.on);
        assert_eq!(p.active, 0o177);
        assert_eq!(p.word().rh().get(), 0o000377);
        assert!(p.write(Half::wrap(0o010000))); // clear the PI system
        assert_eq!(p, Pi::default());
    }

    // PRM 4.1: the highest priority recognized request is accepted if it is
    // above every level in progress; requests count on active levels only.
    #[test]
    fn pi_accept() {
        let mut p = Pi {
            on: true,
            active: 0o177,
            ..Pi::default()
        };
        assert_eq!(p.accept(Pi::level_bit(3) | Pi::level_bit(5)), Some(3));
        p.in_progress = Pi::level_bit(3);
        assert_eq!(p.accept(Pi::level_bit(5)), None);
        assert_eq!(p.accept(Pi::level_bit(2)), Some(2));
        p.active = 0o177 & !Pi::level_bit(2);
        assert_eq!(p.accept(Pi::level_bit(2)), None);
        p.on = false;
        p.active = 0o177;
        assert_eq!(p.accept(Pi::level_bit(1)), None);
        p.on = true;
        p.in_progress = Pi::level_bit(2) | Pi::level_bit(6);
        p.dismiss();
        assert_eq!(p.in_progress, Pi::level_bit(6));
    }

    // Both "system on" and "system off" in one WRPI: off wins, as in the
    // order of inout.50. A bit in 18 to 21 makes WRPI fail after the clear.
    #[test]
    fn wrpi_order() {
        let mut p = Pi::default();
        assert!(p.write(Half::wrap(0o000600)));
        assert!(!p.on);
        p.on = true;
        assert!(!p.write(Half::wrap(0o210000)));
        assert_eq!(p, Pi::default());
    }
}

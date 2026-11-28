//! The I/O bus of the KS10: the memory status register, the two Unibus
//! adapters and the devices behind them (docs/emulator.md, "Unibus
//! devices"; the registers are in spec/devices.toml, `devregs`).
//!
//! **[its]** ksdefs.194 (UBAPAG, UBASTA, UBAMNT, KSECCS) and its.1652
//! (UBAQIT, UBAIIT, the UBASTA setup) give the formats and what ITS
//! expects; **[microcode]** inout.50 (PI, VECINT) gives the interrupt
//! cycle:
//!
//! - The processor asks the bus which adapters request an interrupt on
//!   the PI level it takes (the WRU cycle). The first adapter that answers
//!   gives the vector of its device (the vector cycle).
//! - An adapter requests on the PI level of its status register: PIH for
//!   devices on BR7 and BR6, PIL for BR5 and BR4.
//! - DMA: Unibus byte address A uses paging RAM entry A / 2048. The entry
//!   gives a DEC page; the KS10 word is (page * 512) + (A / 4) mod 512.
//!   Two 18-bit Unibus words fill one KS10 word: A mod 4 = 0 is the left
//!   half (simple.42, UBABLT: "Unibus format").

use pdp10::W36;

use crate::ch11::Ch11;
use crate::devregs::{self, Instance, uba};
use crate::dz11::Dz11;
use crate::imp::Imp;
use crate::rh11::Rh11;
use crate::tm03::Tm03;

/// One Unibus adapter.
#[derive(Clone, Debug)]
pub struct Uba {
    /// The paging RAM entries as written (`devregs::uba::page::W_*`).
    pub page: [u64; 64],
    /// The status bits that the program sets or that the adapter latches
    /// (not HIG and LOW, which come from the devices).
    pub status: u64,
}

impl Default for Uba {
    fn default() -> Self {
        Uba {
            page: [0; 64],
            status: 0,
        }
    }
}

impl Uba {
    /// The paging RAM entry as read (ksdefs.194, %UP): the flags move to
    /// the left half (bits 4.4 to 4.1), the page to bits 3.2 to 2.1. The
    /// parity bits are not modelled (docs/unknowns.md).
    fn read_page(&self, k: usize) -> u64 {
        let w = self.page[k];
        let flags = (w >> 14) & 0o17;
        let page = w & uba::page::W_PAGE;
        (flags << 27) | (page << 9)
    }

    /// The KS10 word and half for Unibus byte address `a`, or `None` if
    /// the entry is not valid or `a` is above the mapped space.
    pub fn map(&self, a: u32) -> Option<(usize, bool)> {
        let k = (a >> 11) as usize;
        let w = *self.page.get(k)?;
        if w & uba::page::W_VAL == 0 {
            return None;
        }
        let page = (w & uba::page::W_PAGE) as usize;
        Some(((page << 9) | ((a as usize >> 2) & 0o777), a & 2 != 0))
    }

    /// PI level of a request on bus request level `br`.
    fn level(&self, br: u32) -> u8 {
        let s = self.status;
        if br >= 6 {
            ((s & uba::sta::PIH) >> uba::sta::PIH_LSB) as u8
        } else {
            (s & uba::sta::PIL) as u8
        }
    }
}

/// A device event, due at an instruction count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// Disk drive `unit` finishes the command with this number.
    Disk(u8, u32),
    /// A DZ11 transmitter is free again.
    Dz,
    /// The CH11 transmission ends.
    Chaos,
    /// The tape formatter finishes the command with this number.
    Tape(u32),
    /// Tape transport `n` finishes the rewind with this number.
    Rewind(u8, u32),
}

/// The mask bit of PI level n (1 to 7): level 1 is the left bit of 8.
/// The processor takes the requests of the bus in this form.
pub fn level_bit(n: u8) -> u8 {
    1 << (7 - n)
}

/// The I/O bus: adapters 1 and 3 (index 0 and 2 are not present).
pub struct Io {
    pub uba: [Uba; 4],
    pub disk: Rh11,
    pub tape: Tm03,
    pub dz: Dz11,
    pub chaos: Ch11,
    pub imp: Imp,
    /// Pending events, unordered.
    pub events: Vec<(u64, Event)>,
    /// The memory status register.
    pub memstat: MemStatus,
}

/// The memory status register of the memory controller **[manual]**
/// (EK-OKS10-TM-002 5.7.8 and figure 5-30; M8618-0-MMC3, MMC4, MMC5, MMC8).
/// The model has no memory errors, so HOLD, BAD DATA and REF ERR are 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemStatus {
    /// PAR ERR: a write loads it from its bit.
    pub par: bool,
    /// MMC3 E131 loads ECC ON from the complement of bit 35 at each status
    /// write, and at RESET.
    pub ecc_off: bool,
    /// The force check bits CP to C01 (MMC3 E144, E131). The model has no
    /// check bits, so they have no effect (docs/emulator.md).
    pub force: u8,
    /// POWER FAILED (MMC5 E58): the power-up clears E58, which sets the
    /// flag. A write with bit 12 set clears it **[its]** (ksdefs,
    /// `%KEPWR` "[R/C]"; docs/discrepancies.md).
    pub power_failed: bool,
}

impl Default for MemStatus {
    /// The state at power on.
    fn default() -> Self {
        MemStatus {
            par: false,
            ecc_off: false,
            force: 0,
            power_failed: true,
        }
    }
}

impl MemStatus {
    /// The bus RESET: STATUS LOAD EN with the data lines 0 (MMC3 E70):
    /// ECC ON, no force bits. It clears PAR ERR **[manual]** (KS10
    /// reference card, memory status register, bit 3). POWER FAILED stays.
    fn reset(&mut self) {
        *self = MemStatus {
            power_failed: self.power_failed,
            ..MemStatus::default()
        };
    }

    /// A status read. ERR ADD LOAD loads the error address at each
    /// command/address cycle while no error is held (MMC5 E48; MMCA E53;
    /// MMC8 E100, E149, E135), so it has the address of this read. The
    /// correction code is the syndrome of the last data strobe (MMC4
    /// E104): 0 for a correct word **[estimate]** (docs/unknowns.md).
    pub fn read(&self) -> u64 {
        use devregs::memctl::status as s;
        let bit = |on: bool, m: u64| if on { m } else { 0 };
        bit(self.par, s::PAR)
            | bit(!self.ecc_off, s::ENA)
            | bit(self.power_failed, s::PWR)
            | u64::from(devregs::memctl::STATUS)
    }

    /// A status write (figure 5-30).
    pub fn write(&mut self, v: u64) {
        use devregs::memctl::status as s;
        self.par = v & s::PAR != 0;
        self.ecc_off = v & s::OFF != 0;
        self.force = ((v & s::FRC) >> s::FRC_LSB) as u8;
        if v & s::PWR != 0 {
            self.power_failed = false;
        }
    }
}

impl Default for Io {
    fn default() -> Self {
        Io {
            uba: Default::default(),
            disk: Rh11::new(devregs::DISK),
            tape: Tm03::new(devregs::TAPE),
            dz: Dz11::new(devregs::DZ0),
            // ITS config.202, configuration DB: MYCHAD==177002.
            chaos: Ch11::new(devregs::CHAOS, 0o177002),
            imp: Imp::new(devregs::IMP),
            events: Vec::new(),
            memstat: MemStatus::default(),
        }
    }
}

/// The reply of a bus reference.
pub enum Reply {
    Value(u64),
    /// No device answered (a page failure with "no IO register").
    NoDevice,
}

impl Io {
    /// The interrupt requests of the devices: adapter, bus request
    /// level, vector, and which request (for the acknowledge).
    fn sources(&self) -> [(bool, Instance, u32, u8); 8] {
        let (d, z, c, i) = (&self.disk, &self.dz, &self.chaos, &self.imp);
        let t = &self.tape;
        [
            (d.irq, d.inst, d.inst.vector, 0),
            (t.irq, t.inst, t.inst.vector, 7),
            (i.rx_irq, i.inst, i.inst.vector, 5),
            (i.tx_irq, i.inst, i.inst.vector + 4, 6),
            (z.rx_irq, z.inst, z.inst.vector, 1),
            (z.tx_irq, z.inst, z.inst.vector + 4, 2),
            (c.rx_irq, c.inst, c.inst.vector, 3),
            (c.tx_irq, c.inst, c.inst.vector, 4),
        ]
    }

    /// The PI levels that the adapters request, as a mask ([`level_bit`]).
    pub fn requests(&self) -> u8 {
        // No device requests: the usual case, without the scan of the
        // sources. The list is that of `Io::sources`.
        let (d, z, c, i, t) = (&self.disk, &self.dz, &self.chaos, &self.imp, &self.tape);
        if !(d.irq || t.irq || i.rx_irq || i.tx_irq || z.rx_irq || z.tx_irq || c.rx_irq || c.tx_irq)
        {
            return 0;
        }
        let mut m = 0;
        for (on, inst, _, _) in self.sources() {
            if on {
                let l = self.uba[inst.adapter as usize].level(inst.br);
                if l != 0 {
                    m |= level_bit(l);
                }
            }
        }
        m
    }

    /// The answer of the adapters to a WRU cycle for PI level `n`: bit
    /// 18 + a of the word for each adapter a that requests on level `n`
    /// (EK-OKS10-TM-002 5.3.7; inout.50, VECINT: "BIT 18 FOR DEVICE 0").
    pub fn wru(&self, n: u8) -> u64 {
        let mut w = 0;
        for (on, inst, _, _) in self.sources() {
            if on && self.uba[inst.adapter as usize].level(inst.br) == n {
                w |= 1 << (17 - inst.adapter);
            }
        }
        w
    }

    /// The WRU and vector cycles for PI level `n`: the adapter and the
    /// vector, and the device drops its request. The adapters answer in
    /// the order of the WRU bits (VECINT: bit 18 first).
    pub fn acknowledge(&mut self, n: u8) -> Option<(u32, u32)> {
        let w = self.wru(n);
        let a = (0..4).find(|a| w & 1 << (17 - a) != 0)?;
        self.vector(a, n).map(|v| (a, v))
    }

    /// The vector cycle on adapter `a` for PI level `n`: the vector, and
    /// the device drops its request. The highest bus request level wins,
    /// then the order of `sources`.
    pub fn vector(&mut self, a: u32, n: u8) -> Option<u32> {
        let mut best: Option<(u32, u32, u8)> = None;
        for (on, inst, vec, which) in self.sources() {
            if !on || inst.adapter != a || self.uba[a as usize].level(inst.br) != n {
                continue;
            }
            if best.is_none_or(|(br, _, _)| inst.br > br) {
                best = Some((inst.br, vec, which));
            }
        }
        let (_, vec, which) = best?;
        match which {
            0 => self.disk.irq = false,
            7 => self.tape.irq = false,
            1 => self.dz.rx_irq = false,
            2 => self.dz.tx_irq = false,
            5 => self.imp.rx_irq = false,
            6 => self.imp.tx_irq = false,
            // The CH11 requests while its condition holds (ch11.defs1:
            // the program clears RDN or TDN).
            _ => {}
        }
        Some(vec)
    }

    /// An I/O read cycle of the processor. A byte read reads the word at
    /// the even address.
    pub fn read_cycle(&mut self, a: u32, byte: bool, now: u64) -> Reply {
        self.read(if byte { a & !1 } else { a }, now)
    }

    /// An I/O write cycle of the processor. A byte write changes the byte
    /// that the address selects: the high byte for an odd address.
    pub fn write_cycle(
        &mut self,
        mem: &mut [W36],
        now: u64,
        a: u32,
        value: u64,
        byte: bool,
    ) -> bool {
        let mask = match (byte, a & 1) {
            (false, _) => 0o177777,
            (true, 0) => 0o377,
            (true, _) => 0o177400,
        };
        let addr = if byte { a & !1 } else { a };
        self.write(mem, now, addr, value, mask)
    }

    /// The bus RESET of the console (hcore.m80, MRINT): the adapters and
    /// the devices return to their state at power on.
    pub fn reset(&mut self) {
        self.uba = Default::default();
        self.disk.reset();
        self.dz.reset();
        self.chaos.reset();
        self.imp.reset();
        self.events.clear();
        self.memstat.reset();
    }

    /// A read of I/O address `addr` (adapter in bits 14 to 17).
    pub fn read(&mut self, addr: u32, now: u64) -> Reply {
        let a = addr >> 18;
        let ub = addr & 0o777777;
        if a == 0 {
            return if ub == devregs::memctl::STATUS {
                Reply::Value(self.memstat.read())
            } else {
                Reply::NoDevice
            };
        }
        if a != 1 && a != 3 {
            return Reply::NoDevice;
        }
        let u = &self.uba[a as usize];
        if (uba::PAGE..uba::PAGE + uba::PAGE_COUNT).contains(&ub) {
            return Reply::Value(u.read_page((ub - uba::PAGE) as usize));
        }
        if ub == uba::STA {
            return Reply::Value(self.status(a));
        }
        if ub == uba::MNT {
            return Reply::Value(0);
        }
        if let Some(off) = self.disk.offset(a, ub) {
            return Reply::Value(u64::from(self.disk.read(off)));
        }
        if let Some(off) = self.tape.offset(a, ub) {
            return Reply::Value(u64::from(self.tape.read(off)));
        }
        if let Some(off) = self.dz.offset(a, ub) {
            return Reply::Value(u64::from(self.dz.read(off)));
        }
        if let Some(off) = self.chaos.offset(a, ub) {
            return Reply::Value(u64::from(self.chaos.read(off, now, &mut self.events)));
        }
        if let Some(off) = self.imp.offset(a, ub) {
            return Reply::Value(u64::from(self.imp.read(off)));
        }
        self.uba[a as usize].status |= uba::sta::NXD;
        Reply::NoDevice
    }

    /// A write of I/O address `addr`. `mask` selects the bits of a 16-bit
    /// Unibus register that change (a byte write changes one byte).
    pub fn write(&mut self, mem: &mut [W36], now: u64, addr: u32, value: u64, mask: u16) -> bool {
        let a = addr >> 18;
        let ub = addr & 0o777777;
        if a == 0 {
            if ub == devregs::memctl::STATUS {
                self.memstat.write(value);
                return true;
            }
            return false;
        }
        if a != 1 && a != 3 {
            return false;
        }
        if (uba::PAGE..uba::PAGE + uba::PAGE_COUNT).contains(&ub) {
            let w = value
                & (uba::page::W_RPW
                    | uba::page::W_16B
                    | uba::page::W_FST
                    | uba::page::W_VAL
                    | uba::page::W_PAGE);
            self.uba[a as usize].page[(ub - uba::PAGE) as usize] = w;
            return true;
        }
        if ub == uba::STA {
            self.write_status(a, value);
            return true;
        }
        if ub == uba::MNT {
            return true;
        }
        if let Some(off) = self.disk.offset(a, ub) {
            let Io {
                uba, disk, events, ..
            } = self;
            disk.write(off, value as u16, mask, &uba[a as usize], mem, now, events);
            return true;
        }
        if let Some(off) = self.tape.offset(a, ub) {
            let Io {
                uba, tape, events, ..
            } = self;
            tape.write(off, value as u16, mask, &uba[a as usize], mem, now, events);
            return true;
        }
        if let Some(off) = self.dz.offset(a, ub) {
            self.dz
                .write(off, value as u16, mask, now, &mut self.events);
            return true;
        }
        if let Some(off) = self.chaos.offset(a, ub) {
            self.chaos.write(off, value as u16, mask);
            return true;
        }
        if let Some(off) = self.imp.offset(a, ub) {
            self.imp.write(off, value as u16, mask);
            return true;
        }
        self.uba[a as usize].status |= uba::sta::NXD;
        false
    }

    /// UBASTA as read: the latched bits, the request bits HIG and LOW.
    fn status(&self, a: u32) -> u64 {
        let mut s = self.uba[a as usize].status;
        for (on, inst, _, _) in self.sources() {
            if on && inst.adapter == a {
                s |= if inst.br >= 6 {
                    uba::sta::HIG
                } else {
                    uba::sta::LOW
                };
            }
        }
        s
    }

    /// UBASTA as written (ksdefs.194, UBASTA): 1s clear TIM, BAD, PAR and
    /// NXD; any write clears PWR; DXF and the PI levels are set; INI
    /// resets the devices of the adapter.
    fn write_status(&mut self, a: u32, v: u64) {
        let u = &mut self.uba[a as usize];
        let rc = uba::sta::TIM | uba::sta::BAD | uba::sta::PAR | uba::sta::NXD;
        let keep = u.status & rc & !v;
        let set = v & (uba::sta::DXF | uba::sta::PIH | uba::sta::PIL);
        u.status = keep | set;
        if v & uba::sta::INI != 0 {
            if self.disk.inst.adapter == a {
                self.disk.reset();
                self.events.retain(|e| !matches!(e.1, Event::Disk(..)));
            }
            if self.tape.inst.adapter == a {
                self.tape.reset();
                self.events.retain(|e| !matches!(e.1, Event::Tape(_)));
            }
            if self.dz.inst.adapter == a {
                self.dz.reset();
            }
            if self.chaos.inst.adapter == a {
                self.chaos.reset();
            }
            if self.imp.inst.adapter == a {
                self.imp.reset();
            }
        }
    }

    /// The first instruction count at which [`Io::poll`] or
    /// [`Io::run_events`] can change the state, if the processor does no
    /// I/O.
    pub fn deadline(&self) -> u64 {
        self.events
            .iter()
            .map(|e| e.0)
            .fold(self.dz.deadline(), u64::min)
    }

    /// Device work between instructions: the DZ11 receivers.
    pub fn poll(&mut self, now: u64) {
        self.dz.poll(now, &mut self.events);
    }

    /// Run the events that are due at `now`.
    pub fn run_events(&mut self, mem: &mut [W36], now: u64) {
        while let Some(k) = self.events.iter().position(|e| e.0 <= now) {
            let (_, ev) = self.events.swap_remove(k);
            match ev {
                Event::Disk(unit, serial) => {
                    let Io { uba, disk, .. } = self;
                    let a = disk.inst.adapter as usize;
                    disk.finish(unit, serial, &uba[a], mem);
                }
                Event::Dz => self.dz.tick(now, &mut self.events),
                Event::Chaos => self.chaos.done(),
                Event::Tape(serial) => {
                    let Io { uba, tape, .. } = self;
                    let a = tape.inst.adapter as usize;
                    tape.finish(serial, &uba[a], mem);
                }
                Event::Rewind(n, serial) => {
                    let Io {
                        uba, tape, events, ..
                    } = self;
                    let a = tape.inst.adapter as usize;
                    tape.rewound(n, serial, &uba[a], mem, now, events);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // EK-OKS10-TM-002 figure 5-30, with bits from the left: 3 PAR ERR, 4
    // ECC ON, 12 POWER FAIL, 14-35 the error address (here the address
    // of the read, 0,,100000); written 3 PAR ERR, 12 POWER FAIL, 35 ECC
    // OFF. ksdefs: %KEPWR is "[R/C]".
    #[test]
    fn memory_status_register() {
        let mut io = Io::default();
        let mut mem = vec![W36::ZERO; 16];
        let a = 0o100000;
        let read = |io: &mut Io| match io.read(a, 0) {
            Reply::Value(v) => format!("{v:o}"),
            _ => panic!(),
        };
        assert_eq!(read(&mut io), "20040100000", "power on");
        assert!(io.write(&mut mem, 0, a, 0o040_000_000_001, 0o177777));
        assert_eq!(read(&mut io), "40040100000", "PAR ERR, ECC OFF");
        io.reset();
        assert_eq!(read(&mut io), "20040100000", "after RESET");
        assert!(io.write(&mut mem, 0, a, 0o000_040_000_000, 0o177777));
        assert_eq!(read(&mut io), "20000100000", "POWER FAIL cleared");
        assert!(io.write(&mut mem, 0, a, 0o000_000_000_376, 0o177777));
        assert_eq!(io.memstat.force, 0o177, "force bits");
    }

    // ksdefs.194, UBAPAG: written 2.6 valid, 2.7 fast, 2.2-1.1 the page;
    // read back as 4.1 valid, 4.2 fast, 3.2-2.1 the page.
    #[test]
    fn paging_ram() {
        let mut io = Io::default();
        let mut mem = vec![W36::ZERO; 16];
        let a = (1 << 18) | 0o763005;
        assert!(io.write(
            &mut mem,
            0,
            a,
            uba::page::W_VAL | uba::page::W_FST | 0o123,
            0o177777
        ));
        let Reply::Value(v) = io.read(a, 0) else {
            panic!()
        };
        assert_eq!(v, uba::page::R_VAL | uba::page::R_FST | (0o123 << 9));
        // Entry 5 maps Unibus 5 * 2048 = 24000 to DEC page 123; byte 24006
        // is the right half of word 1.
        let u = &io.uba[1];
        assert_eq!(u.map(0o24006), Some(((0o123 << 9) | 1, true)));
        assert_eq!(u.map(0o14000), None);
    }

    // its.1652: ITS writes DSKCHN_3 into UBASTA of adapter 1; a status
    // write with 1s in the error bits clears them.
    #[test]
    fn status_register() {
        let mut io = Io::default();
        let mut mem = vec![W36::ZERO; 16];
        let a = (1 << 18) | uba::STA;
        io.uba[1].status = uba::sta::NXD;
        io.write(&mut mem, 0, a, uba::sta::NXD | 0o50, 0o177777);
        let Reply::Value(v) = io.read(a, 0) else {
            panic!()
        };
        assert_eq!(v, 0o50);
        // A reference to a missing Unibus address sets NXD.
        assert!(matches!(io.read((1 << 18) | 0o760000, 0), Reply::NoDevice));
        assert_ne!(io.uba[1].status & uba::sta::NXD, 0);
    }
}

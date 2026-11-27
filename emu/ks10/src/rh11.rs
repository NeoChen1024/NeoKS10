//! The RH11 Massbus controller with RP06 disk drives (spec/devices.toml:
//! `rh11`, `rp06`; the instance `disk`).
//!
//! Sources: **[its]** rh11.defs48 and disk.1228 (what ITS writes and
//! expects); **[manual]** RH11-AB option description and the RP04/05/06
//! maintenance manual (references/); **[emulator]** KLH10 6d733f2
//! (dvrh11.c, dvrpxx.c) and sims 9556eca0 (kx10_rh.c, kx10_rp.c) for the
//! details that the documents leave open:
//!
//! - Interrupts need IE. The controller requests at the end of a data
//!   transfer, and when a drive sets attention. Writing CS1 with GO, or with
//!   IE clear, drops the request; writing IE with an attention pending
//!   requests (sims, rh_write).
//! - The word count counts 18-bit Unibus words, two for each KS10 word
//!   (disk.1228: -4000 for a block of 2000 words); the address counts
//!   bytes, 2 for each 18-bit word.
//! - A drive command with an error present only sets attention, except
//!   Drive Clear (KLH10, rp_cmdxct). A register write while GO is set is
//!   refused with RMR.
//! - Seek, Search and Recalibrate set attention when they end; a data
//!   transfer sets controller ready; neither sets the other.
//! - A data transfer moves its data when it ends, not when it starts
//!   (KLH10, rp_delayop: a boot program can read over its own code).
//!
//! The disk image has 8 bytes for each word, little-endian (AGENTS.md;
//! docs/emulator.md, "Image formats"), sector after sector in the order
//! (cylinder, track, sector).

use std::fs::File;
use std::os::unix::fs::FileExt;

use pdp10::W36;

use crate::devregs::{Instance, rh11 as r, rp06};
use crate::unibus::{Event, Uba};

/// The time of a positioning command or a data transfer, in instructions.
/// **[estimate]**: about 0.1 ms at 1 µs for each instruction; the real
/// seek and rotation times are longer (docs/unknowns.md).
pub const DELAY: u64 = 100;

/// The drive type word of an RP06: moving head (2.5) and type 22
/// (rh11.defs48, %HRTYP).
const DT_RP06: u16 = 0o20022;

const WORDS: usize = rp06::WORDS_PER_SECTOR as usize;
const SECTORS: u32 = rp06::CYLINDERS * rp06::TRACKS * rp06::SECTORS;

/// One RP06 drive.
pub struct Rp06 {
    /// The function and GO of CS1 (bits 0 to 5).
    pub cmd: u16,
    pub da: u16,
    pub er1: u16,
    pub er2: u16,
    pub er3: u16,
    pub of: u16,
    pub dc: u16,
    pub cc: u16,
    pub mr: u16,
    pub attn: bool,
    pub vv: bool,
    /// A positioning command is running (PIP).
    pub pip: bool,
    pub file: Option<File>,
    pub read_only: bool,
    /// The number of the last command that waits for an event; an event
    /// with another number is stale (after a clear) and does nothing.
    pub serial: u32,
}

impl Rp06 {
    pub fn new() -> Self {
        Rp06 {
            cmd: 0,
            da: 0,
            er1: 0,
            er2: 0,
            er3: 0,
            of: 0,
            dc: 0,
            cc: 0,
            mr: 0,
            attn: false,
            vv: false,
            pip: false,
            file: None,
            read_only: false,
            serial: 0,
        }
    }

    fn error(&self) -> bool {
        self.er1 | self.er2 | self.er3 != 0
    }

    fn busy(&self) -> bool {
        self.cmd & r::cs1::GO as u16 != 0
    }

    /// The drive status register.
    fn ds(&self) -> u16 {
        let mut s = r::ds::DPR as u16;
        if self.attn {
            s |= r::ds::ATA as u16;
        }
        if self.error() {
            s |= r::ds::ERR as u16;
        }
        if self.pip {
            s |= r::ds::PIP as u16;
        }
        if self.file.is_some() {
            s |= r::ds::MOL as u16;
        }
        if self.read_only {
            s |= r::ds::WRL as u16;
        }
        if !self.busy() {
            s |= r::ds::DRY as u16;
        }
        if self.vv && self.file.is_some() {
            s |= r::ds::VV as u16;
        }
        s
    }

    /// Drive Clear (rh11.defs48, %HMCLR): errors and attention off.
    fn clear(&mut self) {
        self.er1 = 0;
        self.er2 = 0;
        self.er3 = 0;
        self.attn = false;
        self.cmd = 0;
        self.pip = false;
        self.serial = self.serial.wrapping_add(1);
    }

    /// The sector number of DC and DA, if valid.
    fn sector(&self) -> Option<u32> {
        let cyl = u32::from(self.dc);
        let trk = u32::from((self.da >> 8) & 0o77);
        let sec = u32::from(self.da & 0o177);
        (cyl < rp06::CYLINDERS && trk < rp06::TRACKS && sec < rp06::SECTORS)
            .then_some((cyl * rp06::TRACKS + trk) * rp06::SECTORS + sec)
    }

    /// Set DC, DA and CC to sector `s`.
    fn set_sector(&mut self, s: u32) {
        let s = s.min(SECTORS - 1);
        let per_cyl = rp06::TRACKS * rp06::SECTORS;
        self.dc = (s / per_cyl) as u16;
        self.cc = self.dc;
        let rest = s % per_cyl;
        self.da = (((rest / rp06::SECTORS) as u16) << 8) | (rest % rp06::SECTORS) as u16;
    }

    fn read_sector(&self, s: u32, buf: &mut [W36; WORDS]) -> bool {
        let Some(f) = &self.file else { return false };
        let mut b = [0u8; WORDS * 8];
        if f.read_at(&mut b, u64::from(s) * (WORDS as u64 * 8))
            .is_err()
        {
            return false;
        }
        for (k, w) in buf.iter_mut().enumerate() {
            let x = u64::from_le_bytes(b[k * 8..k * 8 + 8].try_into().unwrap_or([0; 8]));
            *w = W36::wrap(x & 0o777777_777777);
        }
        true
    }

    fn write_sector(&self, s: u32, buf: &[W36; WORDS]) -> bool {
        let Some(f) = &self.file else { return false };
        let mut b = [0u8; WORDS * 8];
        for (k, w) in buf.iter().enumerate() {
            b[k * 8..k * 8 + 8].copy_from_slice(&w.get().to_le_bytes());
        }
        f.write_at(&b, u64::from(s) * (WORDS as u64 * 8)).is_ok()
    }
}

impl Default for Rp06 {
    fn default() -> Self {
        Self::new()
    }
}

/// The RH11 controller.
pub struct Rh11 {
    pub inst: Instance,
    /// The controller bits of CS1: TRE, MCPE, A17, A16, RDY, IE.
    pub cs1: u16,
    pub wc: u16,
    pub ba: u16,
    pub cs2: u16,
    pub drives: [Option<Rp06>; 8],
    /// The interrupt request (BR).
    pub irq: bool,
}

impl Rh11 {
    pub fn new(inst: Instance) -> Self {
        Rh11 {
            inst,
            cs1: r::cs1::RDY as u16,
            wc: 0,
            ba: 0,
            cs2: 0,
            drives: Default::default(),
            irq: false,
        }
    }

    /// The register offset of Unibus address `ub` on adapter `a`.
    pub fn offset(&self, a: u32, ub: u32) -> Option<u32> {
        let b = self.inst.base;
        (a == self.inst.adapter && (b..b + 0o50).contains(&ub)).then(|| (ub - b) & !1)
    }

    fn unit(&self) -> usize {
        (self.cs2 & r::cs2::UNIT as u16) as usize
    }

    fn attention(&self) -> bool {
        self.drives.iter().flatten().any(|d| d.attn)
    }

    /// Controller clear (CS2 CLR) and Unibus init.
    pub fn reset(&mut self) {
        self.cs1 = r::cs1::RDY as u16;
        // Drive clear also makes pending events stale.
        self.cs2 = 0;
        self.wc = 0;
        self.ba = 0;
        self.irq = false;
        for d in self.drives.iter_mut().flatten() {
            d.clear();
        }
    }

    fn request(&mut self) {
        if self.cs1 & r::cs1::IE as u16 != 0 {
            self.irq = true;
        }
    }

    pub fn read(&mut self, off: u32) -> u16 {
        let unit = self.unit();
        match off {
            r::CS1 => {
                let mut v = self.cs1;
                if self.attention() || v & (r::cs1::TRE | r::cs1::MCPE) as u16 != 0 {
                    v |= r::cs1::SC as u16;
                }
                match &self.drives[unit] {
                    Some(d) => v | d.cmd | r::cs1::DVA as u16,
                    None => {
                        self.cs2 |= r::cs2::NED as u16;
                        v
                    }
                }
            }
            r::WC => self.wc,
            r::BA => self.ba,
            r::CS2 => self.cs2,
            r::AS => {
                let mut m = 0;
                for (k, d) in self.drives.iter().enumerate() {
                    if d.as_ref().is_some_and(|d| d.attn) {
                        m |= 1 << k;
                    }
                }
                m
            }
            _ => {
                let Some(d) = &self.drives[unit] else {
                    self.cs2 |= r::cs2::NED as u16;
                    return 0;
                };
                match off {
                    r::DA => d.da,
                    r::DS => d.ds(),
                    r::ER1 => d.er1,
                    r::LA => 0,
                    r::DB => 0,
                    r::MR => d.mr,
                    r::DT => DT_RP06,
                    // The serial number is not known (docs/unknowns.md).
                    r::SN => 0,
                    r::OF => d.of,
                    r::DC => d.dc,
                    r::CC => d.cc,
                    r::ER2 => d.er2,
                    r::ER3 => d.er3,
                    _ => 0,
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write(
        &mut self,
        off: u32,
        value: u16,
        mask: u16,
        uba: &Uba,
        mem: &mut [W36],
        now: u64,
        events: &mut Vec<(u64, Event)>,
    ) {
        let old = self.read_quiet(off);
        let v = (old & !mask) | (value & mask);
        let unit = self.unit();
        match off {
            r::CS1 => {
                if v & r::cs1::TRE as u16 != 0 {
                    self.cs1 &= !(r::cs1::TRE as u16);
                }
                let set = (r::cs1::A17 | r::cs1::A16 | r::cs1::IE) as u16;
                self.cs1 = (self.cs1 & !set) | (v & set);
                if v & r::cs1::GO as u16 != 0 || v & r::cs1::IE as u16 == 0 {
                    self.irq = false;
                }
                match self.drives[unit].as_mut() {
                    None => {
                        self.cs2 |= r::cs2::NED as u16;
                        self.cs1 |= r::cs1::MCPE as u16;
                    }
                    Some(d) if d.busy() => d.er1 |= r::er1::RMR as u16,
                    Some(_) => {
                        if v & r::cs1::GO as u16 != 0 {
                            self.command(unit, v & r::cs1::CMD as u16, uba, mem, now, events);
                        }
                    }
                }
                if v & r::cs1::GO as u16 == 0
                    && self.cs1 & r::cs1::IE as u16 != 0
                    && self.attention()
                {
                    self.irq = true;
                }
            }
            r::WC => self.wc = v,
            r::BA => self.ba = v & !1,
            r::CS2 => {
                if v & r::cs2::CLR as u16 != 0 {
                    self.reset();
                }
                let keep = self.cs2 & !(r::cs2::UNIT | r::cs2::BAI | r::cs2::PAT) as u16;
                self.cs2 = keep | (v & (r::cs2::UNIT | r::cs2::BAI | r::cs2::PAT) as u16);
                if self.drives[self.unit()].is_some() {
                    self.cs2 &= !(r::cs2::NED as u16);
                }
            }
            r::AS => {
                for k in 0..8 {
                    if v & (1 << k) != 0
                        && let Some(d) = self.drives[k].as_mut()
                    {
                        d.attn = false;
                    }
                }
            }
            _ => {
                let Some(d) = self.drives[unit].as_mut() else {
                    self.cs2 |= r::cs2::NED as u16;
                    self.cs1 |= r::cs1::MCPE as u16;
                    return;
                };
                if d.busy() {
                    d.er1 |= r::er1::RMR as u16;
                    return;
                }
                match off {
                    r::DA => d.da = v,
                    r::ER1 => d.er1 = v,
                    r::MR => d.mr = v,
                    r::OF => d.of = v,
                    r::DC => d.dc = v,
                    r::ER2 => d.er2 = v,
                    r::ER3 => d.er3 = v,
                    // DS, LA, DB, DT, SN, CC are read only.
                    _ => {}
                }
            }
        }
    }

    /// A register value for a byte write, without side effects.
    fn read_quiet(&self, off: u32) -> u16 {
        let unit = self.unit();
        let d = self.drives[unit].as_ref();
        match off {
            r::CS1 => self.cs1 | d.map_or(0, |d| d.cmd),
            r::WC => self.wc,
            r::BA => self.ba,
            r::CS2 => self.cs2,
            r::DA => d.map_or(0, |d| d.da),
            r::OF => d.map_or(0, |d| d.of),
            r::DC => d.map_or(0, |d| d.dc),
            _ => 0,
        }
    }

    fn command(
        &mut self,
        unit: usize,
        cmd: u16,
        _uba: &Uba,
        _mem: &mut [W36],
        now: u64,
        events: &mut Vec<(u64, Event)>,
    ) {
        use crate::devregs::rh11::cs1_cmd as c;
        let cmd = u64::from(cmd);
        let transfer = cmd >= 0o51;
        let Some(d) = self.drives[unit].as_mut() else {
            return;
        };
        if cmd == c::DCLR {
            d.clear();
            return;
        }
        if d.error() {
            d.attn = true;
            self.request();
            return;
        }
        if d.file.is_none() {
            d.er1 |= r::er1::UNS as u16;
            d.attn = true;
            if transfer {
                self.cs1 |= r::cs1::TRE as u16;
            }
            self.request();
            return;
        }
        match cmd {
            c::NOP | c::UNLOAD | c::RELEASE | c::OFFSET | c::CENTER => {}
            c::PRESET => {
                d.dc = 0;
                d.da = 0;
                d.of = 0;
                d.vv = true;
            }
            c::PACK_ACK => d.vv = true,
            c::RECAL | c::SEEK | c::SEARCH => {
                if cmd == c::SEEK && u32::from(d.dc) >= rp06::CYLINDERS
                    || cmd == c::SEARCH && d.sector().is_none()
                {
                    d.er1 |= r::er1::IAE as u16;
                    d.attn = true;
                    self.request();
                    return;
                }
                d.cmd = cmd as u16;
                d.pip = true;
                d.serial = d.serial.wrapping_add(1);
                events.push((now + DELAY, Event::Disk(unit as u8, d.serial)));
            }
            c::WRITE | c::READ | c::WCHK | c::READ_HD => {
                if cmd == c::WRITE && d.read_only {
                    d.er1 |= r::er1::WLE as u16;
                    d.attn = true;
                    self.cs1 |= r::cs1::TRE as u16;
                    self.request();
                    return;
                }
                d.cmd = cmd as u16;
                self.cs1 &= !(r::cs1::RDY as u16);
                d.serial = d.serial.wrapping_add(1);
                events.push((now + DELAY, Event::Disk(unit as u8, d.serial)));
            }
            _ => {
                d.er1 |= r::er1::ILF as u16;
                d.attn = true;
                self.request();
            }
        }
    }

    /// The end of the command of drive `unit` (event number `serial`).
    pub fn finish(&mut self, unit: u8, serial: u32, uba: &Uba, mem: &mut [W36]) {
        use crate::devregs::rh11::cs1_cmd as c;
        let unit = unit as usize;
        let Some(d) = self.drives[unit].as_mut() else {
            return;
        };
        if d.serial != serial || !d.busy() {
            return;
        }
        // The function values include GO (rh11.defs48).
        let cmd = u64::from(d.cmd & r::cs1::CMD as u16);
        d.cmd &= !(r::cs1::GO as u16);
        match cmd {
            c::RECAL => {
                d.cc = 0;
                d.dc = 0;
                d.pip = false;
                d.attn = true;
                self.request();
            }
            c::SEEK | c::SEARCH => {
                d.cc = d.dc;
                d.pip = false;
                d.attn = true;
                self.request();
            }
            _ => self.transfer(unit, cmd == c::WRITE, cmd == c::WCHK, uba, mem),
        }
    }

    /// A data transfer: the 18-bit words of the sectors from DC and DA,
    /// through the adapter map, until WC reaches 0.
    fn transfer(&mut self, unit: usize, write: bool, check: bool, uba: &Uba, mem: &mut [W36]) {
        let Some(d) = self.drives[unit].as_mut() else {
            return;
        };
        let mut ba =
            (u32::from(self.cs1 & (r::cs1::A17 | r::cs1::A16) as u16) << 8) | u32::from(self.ba);
        let inc = if self.cs2 & r::cs2::BAI as u16 != 0 {
            0
        } else {
            2
        };
        let mut wc = self.wc;
        let mut error = false;
        match d.sector() {
            None => {
                d.er1 |= r::er1::IAE as u16;
                d.attn = true;
                error = true;
            }
            Some(mut s) => {
                let mut buf = [W36::ZERO; WORDS];
                'outer: while wc != 0 {
                    if s >= SECTORS {
                        d.er1 |= r::er1::AOE as u16;
                        d.attn = true;
                        error = true;
                        break;
                    }
                    if !write && !d.read_sector(s, &mut buf) {
                        d.er1 |= r::er1::UNS as u16;
                        error = true;
                        break;
                    }
                    if write {
                        buf = [W36::ZERO; WORDS];
                    }
                    for h in 0..2 * WORDS {
                        if wc == 0 {
                            break;
                        }
                        let Some((addr, right)) = uba.map(ba) else {
                            self.cs2 |= r::cs2::NEM as u16;
                            error = true;
                            break 'outer;
                        };
                        let Some(m) = mem.get_mut(addr) else {
                            self.cs2 |= r::cs2::NEM as u16;
                            error = true;
                            break 'outer;
                        };
                        let w = &mut buf[h / 2];
                        let src = if right { m.rh() } else { m.lh() };
                        if write {
                            *w = if h % 2 == 0 {
                                pdp10::W36::from_halves(src, w.rh())
                            } else {
                                pdp10::W36::from_halves(w.lh(), src)
                            };
                        } else if !check {
                            let half = if h % 2 == 0 { w.lh() } else { w.rh() };
                            *m = if right {
                                pdp10::W36::from_halves(m.lh(), half)
                            } else {
                                pdp10::W36::from_halves(half, m.rh())
                            };
                        }
                        ba = (ba + inc) & 0o777777;
                        wc = wc.wrapping_add(1);
                    }
                    if write && !d.write_sector(s, &buf) {
                        d.er1 |= r::er1::UNS as u16;
                        error = true;
                        break;
                    }
                    s += 1;
                }
                d.set_sector(s);
            }
        }
        self.wc = wc;
        self.ba = (ba & 0o177777) as u16;
        self.cs1 =
            (self.cs1 & !((r::cs1::A17 | r::cs1::A16) as u16)) | (((ba >> 8) & 0o1400) as u16);
        if error {
            self.cs1 |= r::cs1::TRE as u16;
        }
        self.cs1 |= r::cs1::RDY as u16;
        self.request();
    }

    /// Attach an image to drive `unit`.
    pub fn attach(&mut self, unit: usize, file: File, read_only: bool) {
        let mut d = Rp06::new();
        d.file = Some(file);
        d.read_only = read_only;
        d.vv = true;
        self.drives[unit] = Some(d);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devregs;

    fn disk() -> (Rh11, Uba, Vec<W36>, Vec<(u64, Event)>) {
        let mut rh = Rh11::new(devregs::DISK);
        let f = File::open("/dev/null").unwrap();
        rh.attach(0, f, true);
        (rh, Uba::default(), vec![W36::ZERO; 1024], Vec::new())
    }

    fn run(rh: &mut Rh11, uba: &Uba, mem: &mut [W36], ev: &mut Vec<(u64, Event)>) {
        for (_, e) in std::mem::take(ev) {
            if let Event::Disk(u, s) = e {
                rh.finish(u, s, uba, mem);
            }
        }
    }

    // rh11.defs48: a Seek (%HMSEK) sets PIP while it runs; at its end the
    // drive has attention (ATA, the bit in AS) and the current cylinder is
    // the desired one; with IE the controller requests an interrupt.
    #[test]
    fn seek_ends_with_attention() {
        let (mut rh, uba, mut mem, mut ev) = disk();
        rh.write(r::DC, 0o123, 0o177777, &uba, &mut mem, 0, &mut ev);
        let go = (r::cs1_cmd::SEEK | r::cs1::IE) as u16;
        rh.write(r::CS1, go, 0o177777, &uba, &mut mem, 0, &mut ev);
        assert_ne!(rh.read(r::DS) & r::ds::PIP as u16, 0);
        run(&mut rh, &uba, &mut mem, &mut ev);
        let ds = rh.read(r::DS);
        assert_eq!(ds & r::ds::PIP as u16, 0);
        assert_ne!(ds & r::ds::ATA as u16, 0);
        assert_eq!(rh.read(r::AS), 1);
        assert_eq!(rh.read(r::CC), 0o123);
        assert!(rh.irq);
    }

    // A controller clear (CS2 CLR) clears the drives and ends the command:
    // its event must not end a later command early. (A Drive Clear while
    // GO is set is refused like any register write, KLH10 rpxx_wrreg.)
    #[test]
    fn stale_event_after_clear() {
        let (mut rh, uba, mut mem, mut ev) = disk();
        rh.write(
            r::CS1,
            r::cs1_cmd::SEEK as u16,
            0o177777,
            &uba,
            &mut mem,
            0,
            &mut ev,
        );
        let old = std::mem::take(&mut ev);
        rh.write(
            r::CS2,
            r::cs2::CLR as u16,
            0o177777,
            &uba,
            &mut mem,
            0,
            &mut ev,
        );
        rh.write(
            r::CS1,
            r::cs1_cmd::SEEK as u16,
            0o177777,
            &uba,
            &mut mem,
            0,
            &mut ev,
        );
        let mut stale = old;
        run(&mut rh, &uba, &mut mem, &mut stale);
        assert_ne!(
            rh.read(r::DS) & r::ds::PIP as u16,
            0,
            "the new seek still runs"
        );
        run(&mut rh, &uba, &mut mem, &mut ev);
        assert_eq!(rh.read(r::DS) & r::ds::PIP as u16, 0);
    }
}

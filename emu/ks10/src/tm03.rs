//! The RH11 Massbus controller with a TM03 formatter and TU45 transports
//! (spec/devices.toml: `tm03`; the instance `tape`).
//!
//! Sources: **[its]** tm03s.defs5 and nmtape.30 (what ITS writes and
//! expects), dr.m80 and cmds.m80 (the 8080 tape boot); **[emulator]**
//! KLH10 6d733f2 (dvtm03.c) and sims 9556eca0 (kx10_tu.c) for the details
//! that the ITS sources leave open:
//!
//! - The controller part is the RH11 of `rh11.rs`: IE, RDY, TRE, the word
//!   count of 18-bit words, the byte address, controller clear. The
//!   formatter is drive 0 of the controller.
//! - The tape control register (TC) selects the transport, the density and
//!   the data format. A write of the frame count (FC) sets FCS.
//! - Space, Erase, Write Tape Mark and Rewind set attention when they end.
//!   A data transfer sets controller ready; it sets attention only with an
//!   error (KLH10, tm_attn: the TM03 manual, p. 4-44).
//! - Space counts records until FC reaches 0, and stops after a tape mark.
//!   If it stops at a tape mark before the count ends, it sets the frame
//!   count error (cmds.m80, MBINT: "we expect there to be a frame count
//!   error from the space forward"; docs/discrepancies.md).
//! - A read sets FC to the number of frames in the record (nmtape.30,
//!   MGRD1A); a read of a tape mark gives FC 0 and TM, without an error.
//! - Rewind turns GO off at once and sets PIP until the tape is at the
//!   load point; then the formatter sets SSC and attention. A command
//!   other than NOP waits with GO set until the rewind ends (KLH10,
//!   tm_cmdxct: ITS gives NOP during a rewind).
//! - A command with an error present only sets attention, except Clear.
//!   A register write while GO is set is refused with RMR.
//!
//! Data formats: core dump puts a word into 5 frames (4 bytes of 8 bits,
//! then the low 4 bits); industry compatible puts bits 0 to 31 into 4
//! frames (tm03s.defs5, %TMFCD, %TMFIC; KLH10, wdstofcd, wdstofic).
//!
//! The tape image is the format that the ITS build tools write
//! (docs/emulator.md, "Image formats"): each record is a 32-bit
//! little-endian frame count, the frames, one pad byte if the count is
//! odd, and the count again. A count of 0 is a tape mark.

use std::fs::File;
use std::os::unix::fs::FileExt;

use pdp10::{Half, W36};

use crate::devregs::{Instance, tm03 as t};
use crate::unibus::{Event, Uba};

/// The time of a command, in instructions. **[estimate]**: as the disk
/// (`rh11::DELAY`); a real record takes milliseconds (docs/unknowns.md).
pub const DELAY: u64 = 100;

/// The time of a rewind, in instructions. **[estimate]**: long enough
/// that ITS sees PIP; a real rewind takes seconds (docs/unknowns.md).
pub const REWIND_TIME: u64 = 1000;

/// The drive type word with a transport: not sector addressed (2.7), tape
/// (2.6), slave present (2.2), TM03 (050) with a TU45 (012). Without a
/// transport: no slave present, type 050. **[emulator]** KLH10 6d733f2,
/// dvtm03.h (TM_DTNS, TM_DTTA, TM_DTSS, TM_DTTM03, TM_DT45).
const DT_TU45: u16 = 0o142052;
const DT_NONE: u16 = 0o140050;

/// The record mark of a tape mark in the image.
const MARK: u32 = 0;
/// The largest record of the image that the emulator accepts, in frames.
const MAX_RECORD: u32 = 0o777777;

/// One TU45 transport with its tape image.
pub struct Transport {
    pub file: Option<File>,
    pub read_only: bool,
    /// The byte position in the image.
    pub pos: u64,
    /// The last motion stopped at a tape mark.
    pub tm: bool,
    /// The tape is rewinding (PIP).
    pub rewinding: bool,
    /// The number of the last rewind; a rewind event with another number
    /// is stale.
    pub serial: u32,
}

impl Transport {
    fn new() -> Self {
        Transport {
            file: None,
            read_only: true,
            pos: 0,
            tm: false,
            rewinding: false,
            serial: 0,
        }
    }

    fn online(&self) -> bool {
        self.file.is_some()
    }

    fn word(&self, at: u64) -> Option<u32> {
        let f = self.file.as_ref()?;
        let mut b = [0u8; 4];
        f.read_exact_at(&mut b, at).ok()?;
        Some(u32::from_le_bytes(b))
    }

    /// The record at the position, forward.
    fn next(&self) -> Rec {
        match self.word(self.pos) {
            None => Rec::End,
            Some(MARK) => Rec::Mark,
            Some(n) if n > MAX_RECORD => Rec::Bad,
            Some(n) => Rec::Data(n),
        }
    }

    /// Move over the record before the position, backward.
    fn back(&mut self) -> Rec {
        if self.pos < 4 {
            return Rec::Start;
        }
        match self.word(self.pos - 4) {
            None => Rec::Bad,
            Some(MARK) => {
                self.pos -= 4;
                Rec::Mark
            }
            Some(n) if n > MAX_RECORD => Rec::Bad,
            Some(n) => {
                let size = 8 + u64::from(n) + u64::from(n & 1);
                if self.pos < size {
                    return Rec::Bad;
                }
                self.pos -= size;
                Rec::Data(n)
            }
        }
    }

    fn skip(&mut self, n: u32) {
        self.pos += 8 + u64::from(n) + u64::from(n & 1);
    }

    /// Read up to `want` frames of the data record of `n` frames at the
    /// position.
    fn data(&self, n: u32, want: usize) -> Option<Vec<u8>> {
        let f = self.file.as_ref()?;
        let mut b = vec![0u8; want.min(n as usize)];
        f.read_exact_at(&mut b, self.pos + 4).ok()?;
        Some(b)
    }

    /// Write a record (or a tape mark if `data` is empty) at the position;
    /// the tape ends after it.
    fn put(&mut self, data: &[u8]) -> bool {
        let Some(f) = self.file.as_ref() else {
            return false;
        };
        let n = data.len() as u32;
        let mut b = n.to_le_bytes().to_vec();
        if n != 0 {
            b.extend_from_slice(data);
            if n & 1 != 0 {
                b.push(0);
            }
            b.extend_from_slice(&n.to_le_bytes());
        }
        let ok = f.write_all_at(&b, self.pos).is_ok();
        self.pos += b.len() as u64;
        ok && f.set_len(self.pos).is_ok()
    }
}

enum Rec {
    Data(u32),
    Mark,
    /// No more records (the end of the image).
    End,
    /// The load point (backward).
    Start,
    /// The image is not in the record format.
    Bad,
}

/// The RH11 controller with its TM03 formatter.
pub struct Tm03 {
    pub inst: Instance,
    /// The controller bits of CS1: TRE, MCPE, A17, A16, RDY, IE.
    pub cs1: u16,
    pub wc: u16,
    pub ba: u16,
    pub cs2: u16,
    /// The interrupt request (BR).
    pub irq: bool,
    /// The function and GO of the formatter (CS1 bits 0 to 5).
    pub cmd: u16,
    pub er: u16,
    pub fc: u16,
    /// FC holds a count that the program wrote (TC FCS).
    pub fcs: bool,
    pub tc: u16,
    pub mr: u16,
    pub attn: bool,
    pub ssc: bool,
    /// A command that waits for the end of a rewind.
    pub pending: Option<u16>,
    /// The number of the last command; an event with another number is
    /// stale.
    pub serial: u32,
    pub slaves: [Transport; 8],
}

impl Tm03 {
    pub fn new(inst: Instance) -> Self {
        Tm03 {
            inst,
            cs1: t::cs1::RDY as u16,
            wc: 0,
            ba: 0,
            cs2: 0,
            irq: false,
            cmd: 0,
            er: 0,
            fc: 0,
            fcs: false,
            tc: 0,
            mr: 0,
            attn: false,
            ssc: false,
            pending: None,
            serial: 0,
            slaves: std::array::from_fn(|_| Transport::new()),
        }
    }

    /// The register offset of Unibus address `ub` on adapter `a`.
    pub fn offset(&self, a: u32, ub: u32) -> Option<u32> {
        let b = self.inst.base;
        (a == self.inst.adapter && (b..b + 0o40).contains(&ub)).then(|| (ub - b) & !1)
    }

    /// The formatter is selected: it is drive 0.
    fn formatter(&self) -> bool {
        self.cs2 & t::cs2::UNIT as u16 == 0
    }

    fn slave(&self) -> usize {
        (self.tc & t::tc::SLAVE as u16) as usize
    }

    fn busy(&self) -> bool {
        self.cmd & t::cs1::GO as u16 != 0
    }

    /// Formatter clear (the Clear command, controller clear, Unibus init).
    fn clear(&mut self) {
        self.er = 0;
        self.attn = false;
        self.ssc = false;
        self.cmd = 0;
        self.pending = None;
        self.serial = self.serial.wrapping_add(1);
    }

    /// Controller clear (CS2 CLR) and Unibus init. A rewind goes on.
    pub fn reset(&mut self) {
        self.cs1 = t::cs1::RDY as u16;
        self.cs2 = 0;
        self.wc = 0;
        self.ba = 0;
        self.irq = false;
        self.clear();
    }

    fn request(&mut self) {
        if self.cs1 & t::cs1::IE as u16 != 0 {
            self.irq = true;
        }
    }

    fn attention(&mut self) {
        self.attn = true;
        self.request();
    }

    /// The formatter status register for the selected transport.
    fn fs(&self) -> u16 {
        let s = &self.slaves[self.slave()];
        let mut v = t::fs::DPR as u16;
        let mut bit = |on: bool, b: u64| {
            if on {
                v |= b as u16;
            }
        };
        bit(self.attn, t::fs::ATA);
        bit(self.er != 0, t::fs::ERR);
        bit(s.rewinding, t::fs::PIP);
        bit(s.online(), t::fs::MOL);
        bit(s.online() && s.read_only, t::fs::WRL);
        bit(!self.busy(), t::fs::DRY);
        bit(self.ssc, t::fs::SSC);
        let den = u64::from(self.tc & t::tc::DEN as u16) >> t::tc::DEN_LSB;
        bit(den == t::tc_den::D1600, t::fs::PES);
        bit(s.tm, t::fs::TM);
        bit(s.online() && s.pos == 0 && !s.rewinding, t::fs::BOT);
        v
    }

    pub fn read(&mut self, off: u32) -> u16 {
        match off {
            t::CS1 => {
                let mut v = self.cs1;
                if self.attn || v & (t::cs1::TRE | t::cs1::MCPE) as u16 != 0 {
                    v |= t::cs1::SC as u16;
                }
                if self.formatter() {
                    v | self.cmd | t::cs1::DVA as u16
                } else {
                    self.cs2 |= t::cs2::NEF as u16;
                    v
                }
            }
            t::WC => self.wc,
            t::BA => self.ba,
            t::CS2 => self.cs2,
            t::AS => u16::from(self.attn),
            _ if !self.formatter() => {
                self.cs2 |= t::cs2::NEF as u16;
                0
            }
            t::FC => self.fc,
            t::FS => self.fs(),
            t::ER => self.er,
            t::MR => self.mr,
            t::DT => {
                if self.slaves[self.slave()].online() {
                    DT_TU45
                } else {
                    DT_NONE
                }
            }
            t::TC => {
                let mut v = self.tc;
                if self.fcs {
                    v |= t::tc::FCS as u16;
                }
                v
            }
            // CC, DB and SN: no check character, no data buffer, the
            // serial number is not known (docs/unknowns.md).
            _ => 0,
        }
    }

    /// A register value for a byte write, without side effects.
    fn read_quiet(&self, off: u32) -> u16 {
        match off {
            t::CS1 => self.cs1 | self.cmd,
            t::WC => self.wc,
            t::BA => self.ba,
            t::CS2 => self.cs2,
            t::FC => self.fc,
            t::MR => self.mr,
            t::TC => self.tc,
            _ => 0,
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
        let v = (self.read_quiet(off) & !mask) | (value & mask);
        match off {
            t::CS1 => {
                if v & t::cs1::TRE as u16 != 0 {
                    self.cs1 &= !(t::cs1::TRE as u16);
                }
                let set = (t::cs1::A17 | t::cs1::A16 | t::cs1::IE) as u16;
                self.cs1 = (self.cs1 & !set) | (v & set);
                if v & t::cs1::GO as u16 != 0 || v & t::cs1::IE as u16 == 0 {
                    self.irq = false;
                }
                if !self.formatter() {
                    self.cs2 |= t::cs2::NEF as u16;
                    self.cs1 |= t::cs1::MCPE as u16;
                } else if self.busy() {
                    self.er |= t::er::RMR as u16;
                } else if v & t::cs1::GO as u16 != 0 {
                    self.command(v & t::cs1::CMD as u16, uba, mem, now, events);
                }
                if v & t::cs1::GO as u16 == 0 && self.cs1 & t::cs1::IE as u16 != 0 && self.attn {
                    self.irq = true;
                }
            }
            t::WC => self.wc = v,
            t::BA => self.ba = v & !1,
            t::CS2 => {
                if v & t::cs2::CLR as u16 != 0 {
                    self.reset();
                }
                let rw = (t::cs2::UNIT | t::cs2::BAI | t::cs2::PAT) as u16;
                self.cs2 = (self.cs2 & !rw) | (v & rw);
                if self.formatter() {
                    self.cs2 &= !(t::cs2::NEF as u16);
                }
            }
            t::AS => {
                if v & 1 != 0 {
                    self.attn = false;
                }
            }
            _ if !self.formatter() => {
                self.cs2 |= t::cs2::NEF as u16;
                self.cs1 |= t::cs1::MCPE as u16;
            }
            t::MR => self.mr = v,
            _ if self.busy() => self.er |= t::er::RMR as u16,
            t::FC => {
                self.fc = v;
                self.fcs = true;
            }
            t::TC => {
                let rw =
                    (t::tc::EAODTE | t::tc::DEN | t::tc::FMT | t::tc::EVPAR | t::tc::SLAVE) as u16;
                self.tc = v & rw;
            }
            // FS, ER, CC, DB, DT and SN are read only.
            _ => {}
        }
    }

    /// The frames of one word in the selected format, or `None` for a
    /// format that the emulator does not have.
    fn frames_per_word(&self) -> Option<usize> {
        let f = u64::from(self.tc & t::tc::FMT as u16) >> t::tc::FMT_LSB;
        match f {
            t::tc_fmt::CORE_DUMP => Some(5),
            t::tc_fmt::INDUSTRY => Some(4),
            _ => None,
        }
    }

    fn command(
        &mut self,
        cmd: u16,
        _uba: &Uba,
        _mem: &mut [W36],
        now: u64,
        events: &mut Vec<(u64, Event)>,
    ) {
        use t::cs1_cmd as c;
        let f = u64::from(cmd);
        let transfer = f >= c::WCHKF;
        if f == c::CLEAR {
            self.clear();
            return;
        }
        if self.er != 0 {
            self.attention();
            return;
        }
        let n = self.slave();
        if self.slaves[n].rewinding {
            if f != c::NOP {
                self.cmd = cmd;
                self.pending = Some(cmd);
            }
            return;
        }
        if f == c::NOP {
            return;
        }
        let s = &self.slaves[n];
        let known = matches!(
            f,
            c::UNLOAD
                | c::REWIND
                | c::ERASE
                | c::WTM
                | c::SPACEF
                | c::SPACER
                | c::WCHKF
                | c::WCHKR
                | c::WRITE
                | c::READF
                | c::READR
        );
        // A write on a write-locked tape, or a write or space without a
        // frame count: non-executable function (KLH10, tm_cmdxct).
        let locked = (f == c::WRITE || f == c::WTM || f == c::ERASE) && s.read_only;
        let no_count = (f == c::WRITE || f == c::SPACEF || f == c::SPACER) && !self.fcs;
        let error = if !known {
            Some(t::er::ILF)
        } else if !s.online() {
            Some(t::er::UNS)
        } else if locked || no_count {
            Some(t::er::NEF)
        } else if transfer && self.frames_per_word().is_none() {
            Some(t::er::FMT)
        } else {
            None
        };
        if let Some(e) = error {
            self.er |= e as u16;
            if transfer {
                self.cs1 |= t::cs1::TRE as u16;
            }
            self.attention();
            return;
        }
        if f == c::REWIND || f == c::UNLOAD {
            let s = &mut self.slaves[n];
            s.rewinding = true;
            s.tm = false;
            s.serial = s.serial.wrapping_add(1);
            events.push((now + REWIND_TIME, Event::Rewind(n as u8, s.serial)));
            if f == c::UNLOAD {
                // The tape goes off line when the rewind ends.
                s.read_only = true;
            }
            return;
        }
        self.cmd = cmd;
        if transfer {
            self.cs1 &= !(t::cs1::RDY as u16);
        }
        self.serial = self.serial.wrapping_add(1);
        events.push((now + DELAY, Event::Tape(self.serial)));
    }

    /// The end of a rewind of transport `n` (event number `serial`).
    pub fn rewound(
        &mut self,
        n: u8,
        serial: u32,
        uba: &Uba,
        mem: &mut [W36],
        now: u64,
        events: &mut Vec<(u64, Event)>,
    ) {
        let s = &mut self.slaves[n as usize & 7];
        if !s.rewinding || s.serial != serial {
            return;
        }
        s.rewinding = false;
        s.pos = 0;
        self.ssc = true;
        self.attention();
        if n as usize == self.slave()
            && let Some(cmd) = self.pending.take()
        {
            self.cmd = 0;
            self.command(cmd, uba, mem, now, events);
        }
    }

    /// The end of the formatter command with number `serial`.
    pub fn finish(&mut self, serial: u32, uba: &Uba, mem: &mut [W36]) {
        use t::cs1_cmd as c;
        if self.serial != serial || !self.busy() {
            return;
        }
        let f = u64::from(self.cmd & t::cs1::CMD as u16);
        self.cmd &= !(t::cs1::GO as u16);
        let n = self.slave();
        match f {
            c::SPACEF | c::SPACER => {
                self.space(n, f == c::SPACER);
                self.attention();
            }
            c::ERASE => self.attention(),
            c::WTM => {
                let s = &mut self.slaves[n];
                s.tm = false;
                if !s.put(&[]) {
                    self.er |= t::er::UNS as u16;
                }
                self.attention();
            }
            c::WRITE => self.write_record(n, uba, mem),
            _ => self.read_record(n, f, uba, mem),
        }
    }

    /// Space over records until FC counts to 0 or a tape mark.
    fn space(&mut self, n: usize, reverse: bool) {
        let s = &mut self.slaves[n];
        s.tm = false;
        loop {
            let r = if reverse {
                s.back()
            } else {
                let r = s.next();
                match r {
                    Rec::Data(k) => s.skip(k),
                    Rec::Mark => s.pos += 4,
                    _ => {}
                }
                r
            };
            match r {
                Rec::Data(_) => {}
                Rec::Mark => s.tm = true,
                Rec::Start => {
                    self.er |= t::er::NEF as u16;
                    break;
                }
                Rec::End => {
                    self.er |= t::er::OPI as u16;
                    break;
                }
                Rec::Bad => {
                    self.er |= t::er::INC as u16;
                    break;
                }
            }
            self.fc = self.fc.wrapping_add(1);
            if s.tm {
                break;
            }
            if self.fc == 0 {
                break;
            }
        }
        if self.fc == 0 {
            self.fcs = false;
        } else if s.tm {
            self.er |= t::er::FCE as u16;
        }
    }

    /// The Unibus address with A17 and A16, and its increment.
    fn address(&self) -> (u32, u32) {
        let ba =
            (u32::from(self.cs1 & (t::cs1::A17 | t::cs1::A16) as u16) << 8) | u32::from(self.ba);
        let inc = if self.cs2 & t::cs2::BAI as u16 != 0 {
            0
        } else {
            2
        };
        (ba, inc)
    }

    /// The 18-bit words that WC allows: 0 means 200000.
    fn halfwords(&self) -> usize {
        if self.wc == 0 {
            0x10000
        } else {
            0x10000 - usize::from(self.wc)
        }
    }

    fn set_address(&mut self, ba: u32) {
        self.ba = (ba & 0o177777) as u16;
        self.cs1 =
            (self.cs1 & !((t::cs1::A17 | t::cs1::A16) as u16)) | (((ba >> 8) & 0o1400) as u16);
    }

    /// End a data transfer: controller ready, attention with an error.
    fn end_transfer(&mut self, dma_error: bool) {
        if dma_error {
            self.cs1 |= t::cs1::TRE as u16;
        }
        if self.er != 0 {
            self.cs1 |= t::cs1::TRE as u16;
            self.attn = true;
        }
        self.cs1 |= t::cs1::RDY as u16;
        self.request();
    }

    /// Read Forward, Read Reverse and the Write Checks (they read and do
    /// not store, as the disk; the compare is not modelled).
    fn read_record(&mut self, n: usize, f: u64, uba: &Uba, mem: &mut [W36]) {
        use t::cs1_cmd as c;
        let fpw = self.frames_per_word().unwrap_or(5);
        let reverse = f == c::READR || f == c::WCHKR;
        let store = f == c::READF || f == c::READR;
        self.fc = 0;
        self.fcs = false;
        let mut left = self.halfwords();
        let s = &mut self.slaves[n];
        s.tm = false;
        // Backward, the position moves to the start of the record first.
        let rec = if reverse { s.back() } else { s.next() };
        let frames = match rec {
            Rec::Data(k) => k,
            Rec::Mark => {
                if !reverse {
                    s.pos += 4;
                }
                s.tm = true;
                self.end_transfer(false);
                return;
            }
            Rec::Start => {
                self.er |= t::er::NEF as u16;
                self.end_transfer(false);
                return;
            }
            Rec::End => {
                self.er |= t::er::OPI as u16;
                self.end_transfer(false);
                return;
            }
            Rec::Bad => {
                self.er |= t::er::INC as u16;
                self.end_transfer(false);
                return;
            }
        };
        let words = (frames as usize).div_ceil(fpw);
        let take = words.min(left.div_ceil(2));
        let Some(mut data) = s.data(frames, take * fpw) else {
            self.er |= t::er::INC as u16;
            self.end_transfer(false);
            return;
        };
        if !reverse {
            s.skip(frames);
        }
        self.fc = (frames & 0o177777) as u16;
        data.resize(take * fpw, 0);
        let mut ws: Vec<W36> = data.chunks(fpw).map(|b| unpack(b, fpw)).collect();
        if reverse {
            ws.reverse();
        }
        let (mut ba, inc) = self.address();
        let mut wc = self.wc;
        let mut dma_error = false;
        'words: for w in ws {
            for half in [w.lh(), w.rh()] {
                if left == 0 {
                    break 'words;
                }
                left -= 1;
                let Some((addr, right)) = uba.map(ba) else {
                    self.cs2 |= t::cs2::NEM as u16;
                    dma_error = true;
                    break 'words;
                };
                let Some(m) = mem.get_mut(addr) else {
                    self.cs2 |= t::cs2::NEM as u16;
                    dma_error = true;
                    break 'words;
                };
                if store {
                    *m = if right {
                        W36::from_halves(m.lh(), half)
                    } else {
                        W36::from_halves(half, m.rh())
                    };
                }
                ba = (ba + inc) & 0o777777;
                wc = wc.wrapping_add(1);
            }
        }
        self.wc = wc;
        self.set_address(ba);
        self.end_transfer(dma_error);
    }

    /// Write Forward: -FC frames from memory as one record.
    fn write_record(&mut self, n: usize, uba: &Uba, mem: &mut [W36]) {
        let fpw = self.frames_per_word().unwrap_or(5);
        let frames = 0x10000 - u32::from(self.fc);
        let frames = if self.fc == 0 { 0x10000 } else { frames } as usize;
        let words = frames.div_ceil(fpw);
        let (mut ba, inc) = self.address();
        let mut wc = self.wc;
        let mut left = self.halfwords();
        let mut dma_error = false;
        let mut data = Vec::with_capacity(words * fpw);
        // If the word count ends first, the rest of the record is 0.
        'words: for _ in 0..words {
            let mut h = [Half::ZERO; 2];
            for x in &mut h {
                if left == 0 {
                    pack(W36::from_halves(h[0], h[1]), fpw, &mut data);
                    break 'words;
                }
                left -= 1;
                let Some((addr, right)) = uba.map(ba) else {
                    self.cs2 |= t::cs2::NEM as u16;
                    dma_error = true;
                    break 'words;
                };
                let Some(m) = mem.get(addr) else {
                    self.cs2 |= t::cs2::NEM as u16;
                    dma_error = true;
                    break 'words;
                };
                *x = if right { m.rh() } else { m.lh() };
                ba = (ba + inc) & 0o777777;
                wc = wc.wrapping_add(1);
            }
            pack(W36::from_halves(h[0], h[1]), fpw, &mut data);
        }
        data.resize(frames, 0);
        let s = &mut self.slaves[n];
        s.tm = false;
        if !s.put(&data) {
            self.er |= t::er::UNS as u16;
        }
        self.fc = 0;
        self.fcs = false;
        self.wc = wc;
        self.set_address(ba);
        self.end_transfer(dma_error);
    }

    /// Mount an image on transport `n` at the load point.
    pub fn attach(&mut self, n: usize, file: File, read_only: bool) {
        let s = &mut self.slaves[n & 7];
        s.file = Some(file);
        s.read_only = read_only;
        s.pos = 0;
        s.tm = false;
        s.rewinding = false;
    }
}

/// A word from its frames (`fpw` = 5: core dump, 4: industry compatible).
fn unpack(b: &[u8], fpw: usize) -> W36 {
    let mut w = 0u64;
    for &x in &b[..4] {
        w = (w << 8) | u64::from(x);
    }
    w <<= 4;
    if fpw == 5 {
        w |= u64::from(b[4] & 0o17);
    }
    W36::wrap(w)
}

/// The frames of a word.
fn pack(w: W36, fpw: usize, out: &mut Vec<u8>) {
    let v = w.get();
    for k in 0..4 {
        out.push((v >> (28 - 8 * k)) as u8);
    }
    if fpw == 5 {
        out.push((v & 0o17) as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devregs::{self, uba};

    struct Rig {
        tm: Tm03,
        uba: Uba,
        mem: Vec<W36>,
        ev: Vec<(u64, Event)>,
        path: std::path::PathBuf,
    }

    impl Rig {
        /// A tape image with these bytes on transport 0, Unibus page 1
        /// mapped to memory page 1 (Unibus address 4000 is word 1000).
        fn new(name: &str, image: &[u8], read_only: bool) -> Rig {
            let path =
                std::env::temp_dir().join(format!("ks10-tm03-{}-{name}", std::process::id()));
            std::fs::write(&path, image).unwrap();
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            let mut tm = Tm03::new(devregs::TAPE);
            tm.attach(0, f, read_only);
            let mut u = Uba::default();
            u.page[1] = uba::page::W_VAL | 1;
            Rig {
                tm,
                uba: u,
                mem: vec![W36::ZERO; 0o2000],
                ev: Vec::new(),
                path,
            }
        }

        fn wr(&mut self, off: u32, v: u16) {
            let Rig {
                tm, uba, mem, ev, ..
            } = self;
            tm.write(off, v, 0o177777, uba, mem, 0, ev);
        }

        fn run(&mut self) {
            while !self.ev.is_empty() {
                for (_, e) in std::mem::take(&mut self.ev) {
                    let Rig {
                        tm, uba, mem, ev, ..
                    } = self;
                    match e {
                        Event::Tape(s) => tm.finish(s, uba, mem),
                        Event::Rewind(n, s) => tm.rewound(n, s, uba, mem, 0, ev),
                        _ => {}
                    }
                }
            }
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn record(frames: &[u8]) -> Vec<u8> {
        let n = (frames.len() as u32).to_le_bytes();
        let mut b = n.to_vec();
        b.extend_from_slice(frames);
        if frames.len() % 2 == 1 {
            b.push(0);
        }
        b.extend_from_slice(&n);
        b
    }

    // nmtape.30, MGRD1: WC = -2 * words, FC = 0, Read Forward with IE.
    // At the end FC is the frame count of the record (MGRD1A), the
    // controller is ready and requests an interrupt. Core dump format
    // (KLH10 fcdtowds): 377 0 0 0 17 is the word 776000,,000017.
    #[test]
    fn read_forward() {
        let mut r = Rig::new("read", &record(&[0o377, 0, 0, 0, 0o17, 1, 2, 3]), true);
        r.wr(t::TC, (t::tc_den::D1600 << t::tc::DEN_LSB) as u16);
        r.wr(t::WC, (-4i16) as u16);
        r.wr(t::BA, 0o4000);
        r.wr(t::FC, 0);
        r.wr(t::CS1, (t::cs1_cmd::READF | t::cs1::IE) as u16);
        assert_eq!(r.tm.read(t::CS1) & t::cs1::RDY as u16, 0);
        r.run();
        assert_eq!(r.mem[0o1000], W36::wrap(0o776000_000017));
        // The second word has 3 frames; the rest is 0.
        assert_eq!(r.mem[0o1001], W36::wrap((1 << 28) | (2 << 20) | (3 << 12)));
        assert_eq!(r.tm.read(t::FC), 8);
        assert_eq!(r.tm.read(t::WC), 0);
        assert_ne!(r.tm.read(t::CS1) & t::cs1::RDY as u16, 0);
        assert_eq!(r.tm.read(t::FS) & (t::fs::ERR | t::fs::ATA) as u16, 0);
        assert!(r.tm.irq);
    }

    // cmds.m80, MBINT: a space forward with FC 0 stops after the tape mark
    // of the first file with a frame count error; FS has TM; the space
    // sets attention. FC counts the records and the tape mark.
    #[test]
    fn space_to_tape_mark() {
        let mut img = record(&[1; 10]);
        img.extend(record(&[2; 10]));
        img.extend([0; 4]);
        img.extend(record(&[3; 10]));
        let mut r = Rig::new("space", &img, true);
        r.wr(t::FC, 0);
        r.wr(t::CS1, t::cs1_cmd::SPACEF as u16);
        r.run();
        let fs = r.tm.read(t::FS);
        assert_ne!(fs & t::fs::TM as u16, 0);
        assert_ne!(fs & t::fs::ATA as u16, 0);
        assert_eq!(r.tm.read(t::ER), t::er::FCE as u16);
        assert_eq!(r.tm.read(t::FC), 3);
        // The tape is at the record of the second file.
        // Each record of 10 frames takes 4 + 10 + 4 bytes.
        assert_eq!(r.tm.slaves[0].pos, 2 * 18 + 4);
        // A space reverse of one record (nmtape.30: FC -1, SPR) moves
        // back over the tape mark.
        r.wr(t::CS1, t::cs1_cmd::CLEAR as u16);
        r.wr(t::FC, 0o177777);
        r.wr(t::CS1, t::cs1_cmd::SPACER as u16);
        r.run();
        assert_eq!(r.tm.read(t::ER), 0);
        assert_ne!(r.tm.read(t::FS) & t::fs::TM as u16, 0);
        assert_eq!(r.tm.slaves[0].pos, 2 * 18);
        assert_eq!(r.tm.read(t::TC) & t::tc::FCS as u16, 0);
    }

    // nmtape.30, MGWRIT: WC, FC = -frames, Write Forward. The record has
    // the frames of the words (core dump: 123456,,701234 is 51 313 270 51
    // 14 in octal), and the image format of the ITS tools.
    #[test]
    fn write_record() {
        let mut r = Rig::new("write", &[], false);
        r.mem[0o1000] = W36::wrap(0o123456_701234);
        r.wr(t::WC, (-2i16) as u16);
        r.wr(t::BA, 0o4000);
        r.wr(t::FC, (-5i16) as u16);
        r.wr(t::CS1, t::cs1_cmd::WRITE as u16);
        r.run();
        r.wr(t::CS1, t::cs1_cmd::WTM as u16);
        r.run();
        let img = std::fs::read(&r.path).unwrap();
        let mut want = record(&[0o51, 0o313, 0o270, 0o51, 0o14]);
        want.extend([0; 4]);
        assert_eq!(img, want);
        assert_eq!(r.tm.read(t::ER), 0);
    }

    // KLH10 tm_cmdxct: Rewind turns GO off at once and sets PIP; NOP runs
    // during the rewind; another command waits with GO set. At the end the
    // tape is at BOT, with SSC and attention, and the waiting command runs.
    #[test]
    fn rewind_then_read() {
        let mut r = Rig::new("rewind", &record(&[0o377, 0, 0, 0, 0o17]), true);
        r.tm.slaves[0].pos = 14;
        r.wr(t::CS1, t::cs1_cmd::REWIND as u16);
        assert_eq!(r.tm.read(t::CS1) & t::cs1::GO as u16, 0);
        assert_ne!(r.tm.read(t::FS) & t::fs::PIP as u16, 0);
        r.wr(t::CS1, t::cs1_cmd::NOP as u16);
        assert_eq!(r.tm.read(t::CS1) & t::cs1::GO as u16, 0);
        r.wr(t::WC, (-2i16) as u16);
        r.wr(t::BA, 0o4000);
        r.wr(t::FC, 0);
        r.wr(t::CS1, t::cs1_cmd::READF as u16);
        assert_ne!(r.tm.read(t::CS1) & t::cs1::GO as u16, 0);
        assert_eq!(r.tm.read(t::FS) & t::fs::DRY as u16, 0);
        r.run();
        let fs = r.tm.read(t::FS);
        assert_ne!(fs & t::fs::SSC as u16, 0);
        assert_eq!(fs & t::fs::PIP as u16, 0);
        assert_eq!(r.mem[0o1000], W36::wrap(0o776000_000017));
        assert_eq!(r.tm.read(t::FC), 5);
    }

    // A transport without a tape: Unsafe, attention; a register write
    // while GO is set is refused (RMR).
    #[test]
    fn errors() {
        let mut r = Rig::new("errors", &record(&[1; 5]), true);
        r.wr(t::TC, 1);
        r.wr(t::CS1, t::cs1_cmd::REWIND as u16);
        assert_eq!(r.tm.read(t::ER), t::er::UNS as u16);
        assert_ne!(r.tm.read(t::CS1) & t::cs1::SC as u16, 0);
        assert_eq!(r.tm.read(t::DT), DT_NONE);
        r.wr(t::CS1, t::cs1_cmd::CLEAR as u16);
        r.wr(t::TC, 0);
        assert_eq!(r.tm.read(t::DT), DT_TU45);
        r.wr(t::FC, 0);
        r.wr(t::CS1, t::cs1_cmd::SPACEF as u16);
        r.wr(t::FC, 7);
        assert_eq!(r.tm.read(t::ER), t::er::RMR as u16);
        // Write on a write-locked tape: non-executable function.
        r.run();
        r.wr(t::CS1, t::cs1_cmd::CLEAR as u16);
        r.wr(t::FC, (-5i16) as u16);
        r.wr(t::CS1, t::cs1_cmd::WRITE as u16);
        assert_eq!(r.tm.read(t::ER), t::er::NEF as u16);
        assert_ne!(r.tm.read(t::CS1) & t::cs1::TRE as u16, 0);
    }
}

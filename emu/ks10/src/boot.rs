//! The boot commands of the 8080 console, as a model of what they do
//! **[its]** (src/kshack/cmds.m80: BTCMD, LBINT, FILEINIT, CHKHOM, BLKRDR,
//! PASSSRC; dr.m80: DSKSEQ).
//!
//! `BT` (disk boot):
//!
//! 1. Master reset (BTINT, MRCMD).
//! 2. Read the home block, disk block 1 (else block 10), to 1000, and
//!    check `SIXBIT /HOM/` in its word 0 (CHKHOM, HOMEWD).
//! 3. Word 103 of the home block points to the page of file pointers; read
//!    it to 1000 (GOODPK, BLKRDR).
//! 4. Word 2 points to the microcode, which this emulator does not need
//!    (MICROP, DMEM2CRAM). Word 4 points to the monitor pre-boot; read its
//!    first page to 1000 (LBINT with RM100 = 4).
//! 5. Words 36 to 40: the RH11 base address with the adapter number, the
//!    unit, the density and slave (PASSSRC). Start at 1000.
//!
//! A pointer has the cylinder in bits 0 to 11 and the track and sector in
//! bits 20 to 35, as the drive registers DC and DA take them (BLKRDR).
//!
//! Each page read is the channel command list DSKSEQ, done on the
//! emulated RH11: map Unibus page 1 to memory page 1 (valid, 36-bit
//! transfers), select the unit, controller clear, drive clear, read-in
//! preset, the address, word count -2000 (512 words), Unibus address
//! 4000, read.

use pdp10::{Half, W36};

use crate::devregs::{self, rh11 as r, uba};
use crate::machine::Machine;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootError {
    /// The drive does not exist or has no image.
    NoDrive,
    /// The drive or the controller reported an error.
    Disk { cs1: u16, ds: u16, er1: u16 },
    /// No home block in block 1 or block 10.
    NoHomeBlock,
    /// A file pointer is 0.
    NoFile(u32),
}

/// SIXBIT /HOM/ (cmds.m80, HOMEWD).
const HOM: u64 = 0o505755_000000;

/// Write a disk RH11 register through the I/O bus, as the 8080 does.
fn wr(m: &mut Machine, off: u32, v: u16) {
    let d = devregs::DISK;
    let a = (d.adapter << 18) | (d.base + off);
    let Machine { io, mem, count, .. } = m;
    io.write(mem, *count, a, u64::from(v), 0o177777);
}

fn rd(m: &mut Machine, off: u32) -> u16 {
    let d = devregs::DISK;
    let a = (d.adapter << 18) | (d.base + off);
    match m.io.read(a, m.count) {
        crate::unibus::Reply::Value(v) => v as u16,
        crate::unibus::Reply::NoDevice => 0,
    }
}

/// Finish all device work (the 8080 waits for ready).
fn wait(m: &mut Machine) {
    let Machine { io, mem, .. } = m;
    io.run_events(mem, u64::MAX);
}

/// Read one page (512 words) from cylinder `cyl`, track and sector `da`
/// of drive `unit` to memory 1000 to 1777 (DSKSEQ).
fn read_page(m: &mut Machine, unit: u16, cyl: u16, da: u16) -> Result<(), BootError> {
    let d = devregs::DISK;
    let page1 = (d.adapter << 18) | (uba::PAGE + 1);
    {
        let Machine { io, mem, count, .. } = m;
        io.write(
            mem,
            *count,
            page1,
            uba::page::W_VAL | uba::page::W_FST | 1,
            0o177777,
        );
    }
    wr(m, r::CS2, unit);
    if rd(m, r::DS) & r::ds::DPR as u16 == 0 {
        return Err(BootError::NoDrive);
    }
    wait(m);
    wr(m, r::CS2, r::cs2::CLR as u16);
    wr(m, r::CS2, unit);
    wr(m, r::CS1, r::cs1_cmd::DCLR as u16);
    wr(m, r::CS1, r::cs1_cmd::PRESET as u16);
    wait(m);
    if rd(m, r::DS) & r::ds::VV as u16 == 0 {
        return Err(BootError::NoDrive);
    }
    wr(m, r::DA, da);
    wr(m, r::DC, cyl);
    wr(m, r::WC, 0o176000);
    wr(m, r::BA, 0o4000);
    wr(m, r::CS1, r::cs1_cmd::READ as u16);
    wait(m);
    let (cs1, ds, er1) = (rd(m, r::CS1), rd(m, r::DS), rd(m, r::ER1));
    if ds & r::ds::ERR as u16 != 0 || cs1 & (r::cs1::TRE | r::cs1::MCPE) as u16 != 0 {
        return Err(BootError::Disk { cs1, ds, er1 });
    }
    Ok(())
}

/// Read the page that the pointer word at `loc` gives.
fn read_pointer(m: &mut Machine, unit: u16, loc: u32) -> Result<(), BootError> {
    let p = m.mem[loc as usize].get();
    if p == 0 {
        return Err(BootError::NoFile(loc));
    }
    let cyl = ((p >> 24) & 0o7777) as u16;
    let da = (p & 0o177777) as u16;
    read_page(m, unit, cyl, da)
}

/// The `BT` command for disk unit `unit`.
pub fn boot_disk(m: &mut Machine, unit: u16) -> Result<(), BootError> {
    m.reset();
    read_page(m, unit, 0, 1)?;
    if m.mem[0o1000].get() != HOM {
        read_page(m, unit, 0, 0o10)?;
        if m.mem[0o1000].get() != HOM {
            return Err(BootError::NoHomeBlock);
        }
    }
    read_pointer(m, unit, 0o1103)?;
    read_pointer(m, unit, 0o1004)?;
    let d = devregs::DISK;
    m.mem[0o36] = W36::wrap(u64::from((d.adapter << 18) | d.base));
    m.mem[0o37] = W36::wrap(u64::from(unit));
    m.mem[0o40] = W36::ZERO;
    m.pc = Half::wrap(0o1000);
    Ok(())
}

//! The ITS I/O instructions **[microcode]** (inout.50, "ITS IO
//! INSTRUCTIONS", .IF/ITSIO): IORDI, IORDQ, IORD, IOWR, IOWRI, IOWRQ (710
//! to 715) and their byte forms IORDBI to IOWRBQ (720 to 725).
//!
//! - The address: 3,,E for the "I" forms (Unibus adapter 3), 1,,E for the
//!   "Q" forms (adapter 1), C(E) for IORD and IOWR (IORDI, IOWRI: "Unibus
//!   I", "Unibus Q").
//! - Word forms move a word between AC and the I/O register. Byte forms
//!   move 8 bits: an odd address is the left byte of the 16-bit word
//!   (IORD2, IORD3, IOWR2, IOWR3).
//! - I/O must be legal (A/IOT).
//!
//! The addresses go to the I/O bus (`unibus`). An address without a
//! device is a page failure with bits "no IO register" and "IO cycle"
//! (itspag.98, HARD: "I/O failures come here").

use pdp10::{Half, W36};

use crate::machine::{Fault, Inst, Machine, Next};
use crate::pager::pfw;
use ks10_machine::unibus::Reply;

type R = Result<Next, Fault>;

/// A reference to a nonexistent I/O register.
fn no_register(m: &Machine, a: u32, byte: bool) -> Fault {
    let mut w = pfw::NO_IO | pfw::IO_CYCLE | u64::from(a);
    if byte {
        w |= pfw::IO_BYTE;
    }
    if m.flags.is_user() {
        w |= pfw::USER;
    }
    Fault::PageFail(W36::wrap(w))
}

/// Read an I/O register.
pub fn io_read(m: &mut Machine, a: u32, byte: bool) -> Result<u64, Fault> {
    m.wake();
    let now = m.count;
    match m.io.read_cycle(a, byte, now) {
        Reply::Value(v) => Ok(v),
        Reply::NoDevice => Err(no_register(m, a, byte)),
    }
}

/// Write an I/O register. A byte write changes the byte that the address
/// selects: the high byte for an odd address.
pub fn io_write(m: &mut Machine, a: u32, value: u64, byte: bool) -> Result<(), Fault> {
    m.wake();
    let Machine { io, mem, count, .. } = m;
    if io.write_cycle(mem, *count, a, value, byte) {
        Ok(())
    } else {
        Err(no_register(m, a, byte))
    }
}

pub fn execute(m: &mut Machine, i: Inst, e: Half) -> R {
    if !m.flags.io_legal() {
        return Err(Fault::Uuo);
    }
    // 710 to 715: words; 720 to 725: bytes. 710, 711, 712, 720, 721, 722
    // read.
    let byte = i.op & 0o20 != 0;
    let read = matches!(i.op & 0o7, 0..=2);
    let a = match i.op & 0o7 {
        0 | 4 => 3 << 18 | e.get() as u32,
        1 | 5 => 1 << 18 | e.get() as u32,
        // The 22-bit I/O address in C(E): adapter in bits 14 to 17.
        _ => (m.read(e)?.get() & 0o17777777) as u32,
    };
    let odd = a & 1 != 0;
    if read {
        let v = io_read(m, a, byte)?;
        let v = if byte {
            if odd { (v >> 8) & 0o377 } else { v & 0o377 }
        } else {
            v
        };
        m.ac[i.ac] = W36::wrap(v);
    } else {
        let v = m.ac[i.ac].get();
        let v = if byte && odd { v << 8 } else { v };
        io_write(m, a, v, byte)?;
    }
    Ok(Next::Step)
}

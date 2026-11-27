//! The ITS pager **[microcode]** (itspag.98, "ITS PAGE REFILL CODE").
//!
//! The hardware has a page table of 512 entries, one for each DEC page
//! (512 words) of the virtual address space, with a valid bit, a user bit,
//! a writable bit and a physical DEC page number. A reference that finds no
//! valid entry, or an entry for the other mode, or a write to an entry that
//! is not writable, makes the microcode refill the entry (PAGE-FAIL, PFD:
//! cases 10, 12 and 13, PFMAP):
//!
//! - The DBR: DBR1 for user addresses below 400000, DBR2 for user addresses
//!   from 400000, DBR4 and DBR3 for executive addresses (USER-DBR,
//!   EXEC-DBR).
//! - The page table word: DBR + bits 19 to 24 of the address (an ITS page
//!   is 1024 words; each word has the entries of two pages). Bit 25 of the
//!   address selects the half: the left half for an even page (GOTDBR).
//! - The entry (a half word): bits 0 and 1 are the access, 00 none, 01 read
//!   only, 10 read/write/first, 11 read/write; bit 4 is the age bit; bits
//!   8 to 17 are the physical ITS page (PTE fields).
//! - No access, or a write to a page that is not read/write, is a page
//!   failure. Else the microcode writes the page table word back with the
//!   age bit of the entry cleared, and loads the hardware entries of both
//!   DEC pages (PAGE-REFILL).
//!
//! The page fail word (the VMA flags and the VMA; itspag.98, "VMA flags"
//! and "Page fail word flags", PFMAP and HARD): bit 0 user, bit 1 no IO
//! register, bit 2 no memory, bit 5 write, bits 6 and 7 the access bits of
//! the entry, bit 8 physical, bit 10 IO cycle, bit 13 IO byte, bits 14 to
//! 35 the address.

use pdp10::{Half, W36};

use crate::machine::{Fault, Machine};

/// Page fail word bits.
pub mod pfw {
    pub const USER: u64 = 1 << 35;
    pub const NO_IO: u64 = 1 << 34;
    pub const NO_MEMORY: u64 = 1 << 33;
    pub const WRITE: u64 = 1 << 30;
    pub const ACCESS_29: u64 = 1 << 29;
    pub const ACCESS_28: u64 = 1 << 28;
    pub const PHYSICAL: u64 = 1 << 27;
    pub const IO_CYCLE: u64 = 1 << 25;
    pub const IO_BYTE: u64 = 1 << 22;
    pub const ADDRESS: u64 = (1 << 22) - 1;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub valid: bool,
    pub user: bool,
    pub writable: bool,
    /// The physical DEC page.
    pub page: u32,
}

#[derive(Clone, Debug)]
pub struct Pager {
    pub table: [Entry; 512],
}

impl Default for Pager {
    fn default() -> Self {
        Pager {
            table: [Entry::default(); 512],
        }
    }
}

impl Pager {
    /// A sweep (WREBR, WRUBR, LDBRn, LPMR, CLRCSH: "J/SWEEP").
    pub fn clear_all(&mut self) {
        self.table = [Entry::default(); 512];
    }

    /// CLRPT: the entry of the DEC page of `a` (ksdefs.194, CLRPT: "only
    /// invalidate the page table entry for half page referenced by E").
    pub fn clear(&mut self, a: Half) {
        self.table[(a.get() >> 9) as usize] = Entry::default();
    }
}

/// The page fail word of a virtual reference.
pub fn fail_word(a: Half, user: bool, write: bool, extra: u64) -> W36 {
    let mut w = a.get() | extra;
    if user {
        w |= pfw::USER;
    }
    if write {
        w |= pfw::WRITE;
    }
    W36::wrap(w)
}

/// The physical address of the virtual address `a`, for a reference in
/// user (`user`) or executive mode.
pub fn translate(m: &mut Machine, a: Half, user: bool, write: bool) -> Result<u32, Fault> {
    if !m.paging {
        return Ok(a.get() as u32);
    }
    let idx = (a.get() >> 9) as usize;
    let e = m.pager.table[idx];
    if !(e.valid && e.user == user && (e.writable || !write)) {
        refill(m, a, user, write)?;
    }
    let e = m.pager.table[idx];
    Ok((e.page << 9) | (a.get() as u32 & 0o777))
}

fn refill(m: &mut Machine, a: Half, user: bool, write: bool) -> Result<(), Fault> {
    let v = a.get();
    let high = v & 0o400000 != 0;
    let dbr = match (user, high) {
        (true, false) => m.dbr[0],
        (true, true) => m.dbr[1],
        (false, true) => m.dbr[2],
        (false, false) => m.dbr[3],
    };
    let ptw_addr = (dbr.get().wrapping_add((v >> 11) & 0o77) & 0o3777777) as u32;
    let ptw = m.phys_read(ptw_addr)?;
    let odd = v & 0o2000 != 0;
    let h = if odd { ptw.rh() } else { ptw.lh() }.get();
    let mut extra = 0;
    let writable = match h >> 16 {
        0 => return Err(Fault::PageFail(fail_word(a, user, write, 0))),
        1 => {
            extra = pfw::ACCESS_28;
            false
        }
        2 => {
            extra = pfw::ACCESS_29;
            false
        }
        _ => true,
    };
    if write && !writable {
        return Err(Fault::PageFail(fail_word(a, user, write, extra)));
    }
    // Write the word back with the age bit (2.5) of this entry cleared.
    let age = if odd { 1u64 << 13 } else { 1u64 << 31 };
    m.phys_write(ptw_addr, W36::wrap(ptw.get() & !age))?;
    let its_page = (h & 0o1777) as u32;
    let even = (v >> 9) as usize & !1;
    for k in 0..2 {
        m.pager.table[even + k] = Entry {
            valid: true,
            user,
            writable,
            page: its_page * 2 + k as u32,
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A page table word: the left half for an even page, the right half
    // for an odd page (itspag.98, GOTDBR). 620123 is read/write with the
    // age bit (020000), ITS page 123; 210000 is read only, ITS page 0.
    #[test]
    fn walk_and_age() {
        let mut m = Machine::new();
        m.paging = true;
        m.dbr[3] = W36::wrap(0o1000); // DBR4: executive addresses below 400000
        // ITS pages 4 and 5 are in word 2 of the table.
        m.mem[0o1002] = W36::from_halves(Half::wrap(0o620123), Half::wrap(0o210000));
        let p = translate(&mut m, Half::wrap(0o10005), false, true).unwrap();
        assert_eq!(p, (0o123 << 10) | 5);
        // The age bit (2.5) of the left half is cleared.
        assert_eq!(m.mem[0o1002].lh().get(), 0o600123);
        // Page 5 is read only: a read works, a write fails with the access
        // bits 01 in bits 6 and 7 of the page fail word.
        let p = translate(&mut m, Half::wrap(0o12000), false, false).unwrap();
        assert_eq!(p, 0); // ITS page 0, the even DEC page
        let f = translate(&mut m, Half::wrap(0o12000), false, true).unwrap_err();
        assert_eq!(
            f,
            Fault::PageFail(W36::wrap(pfw::WRITE | pfw::ACCESS_28 | 0o12000))
        );
        // No access: a page fail for user mode as well (DBR1 is 0, so the
        // table word is location 0 + 2).
        let f = translate(&mut m, Half::wrap(0o10000), true, false).unwrap_err();
        assert_eq!(f, Fault::PageFail(W36::wrap(pfw::USER | 0o10000)));
    }
}

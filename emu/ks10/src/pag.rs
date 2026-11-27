//! Opcode 701 (dispatch on AC; spec/instructions.toml, acdisp 701): the
//! EBR and UBR registers, the cache and page table sweeps, and the PC
//! sample table pointer.
//!
//! All of it follows the ITS version of the microcode **[microcode]**
//! (inout.50, "IO -- INTERNAL DEVICES -- EBR & UBR", CLRPT, CLRCSH, the
//! .IF/ITS and .IF/PCST parts). This version has no page table and no
//! cache, so CLRPT, CLRCSH and the sweeps of WREBR and WRUBR have no
//! effect.

use pdp10::{Half, W36};

use crate::machine::{Fault, Inst, Machine, Next, context_word};

type R = Result<Next, Fault>;

pub fn op701(m: &mut Machine, i: Inst, e: Half) -> R {
    if !m.flags.io_legal() {
        return Err(Fault::Uuo);
    }
    match i.ac {
        // CLRCSH, CLRPT.
        0 | 2 => {}
        1 => m.write(e, context_word(m))?,
        3 => {
            let c = m.read(e)?;
            m.ubr = wrubr(m.ubr, c);
            m.load_ac_blocks();
        }
        4 => wrebr(m, e),
        5 => m.write(e, rdebr(m.ebr))?,
        // RDPCST, WRPCST (.IF/PCST): the AOBJN pointer of the PC sample
        // table. The samples come with the 1 ms timer.
        0o11 => m.write(e, m.pcst)?,
        0o13 => m.pcst = m.read(e)?,
        _ => return Err(Fault::Uuo),
    }
    Ok(Next::Step)
}

/// WRUBR, ITS version (inout.50, WRUBR to WRUBR2). The new word `c` sets
/// the AC blocks (bits 6 to 11) if bit 0 is set, and the UBR address
/// (bits 16 to 35) if bit 2 is set. Bits 0 and 2 themselves go into UBR
/// with the fields they select.
fn wrubr(ubr: W36, c: W36) -> W36 {
    const KEEP_BASE: u64 = 0o100003_777777;
    const KEEP_BLOCKS: u64 = 0o407700_000000;
    let (mut ar, mut ubr) = (c.get(), ubr.get());
    if c.bit(0) {
        ubr &= KEEP_BASE;
    } else {
        ar &= KEEP_BASE;
    }
    if ar & (1 << 33) != 0 {
        ubr &= KEEP_BLOCKS;
    } else {
        ar &= KEEP_BLOCKS;
    }
    W36::wrap(ubr | ar)
}

/// WREBR (inout.50, WREBR): EBR gets E shifted left 9 places, with bit 0
/// set if E bit 21 ("Tops-20 style") is set. E bit 22 enables or disables
/// the pager and the traps.
fn wrebr(m: &mut Machine, e: Half) {
    let ar = W36::wrap(e.get() << 9);
    m.paging = ar.bit(13);
    m.ebr = if ar.bit(12) {
        W36::wrap(ar.get() | 1 << 35)
    } else {
        ar
    };
}

/// RDEBR (inout.50, RDEBR): EBR shifted right 9 places, masked with
/// 0,,63777.
fn rdebr(ebr: W36) -> W36 {
    W36::wrap((ebr.get() >> 9) & 0o63777)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values computed by hand from the ksdefs.194 text of WRUBR
    // (4.9 set AC blocks, 4.7 set UBR, 4.3-4.1 current ACs, 3.9-3.7
    // previous ACs, 3.2-1.1 the address).
    #[test]
    fn wrubr_fields() {
        let base = W36::from_halves(Half::wrap(0o100001), Half::wrap(0o234000));
        let u = wrubr(W36::ZERO, base);
        assert_eq!(u, base);
        // Set the AC blocks only: the address stays.
        let blocks = W36::from_halves(Half::wrap(0o403200), Half::ZERO);
        let u = wrubr(u, blocks);
        assert_eq!(u.get(), 0o503201_234000);
        // Neither 4.9 nor 4.7: no change.
        assert_eq!(wrubr(u, W36::wrap(0o007777_777777)), u);
    }

    // ksdefs.194, WREBR: 2.5 enables the pager, 2.2-1.1 is the EPT page
    // number; RDEBR gives them back.
    #[test]
    fn wrebr_rdebr() {
        let mut m = Machine::new();
        wrebr(&mut m, Half::wrap(0o000003));
        assert_eq!(m.ept_addr(0o40), 0o3040);
        assert!(!m.paging);
        assert_eq!(rdebr(m.ebr).get(), 3);
        wrebr(&mut m, Half::wrap(0o020000 | 0o3777));
        assert!(m.paging);
        assert_eq!(m.ept_addr(0), 0o3777000);
        assert_eq!(rdebr(m.ebr).get(), 0o023777);
        // Bits 23 and 24 are not kept.
        wrebr(&mut m, Half::wrap(0o014000));
        assert_eq!(rdebr(m.ebr).get(), 0);
    }

    #[test]
    fn ac_blocks() {
        let mut m = Machine::new();
        m.ac[1] = W36::wrap(0o111);
        m.ubr = wrubr(m.ubr, W36::from_halves(Half::wrap(0o403000), Half::ZERO));
        m.load_ac_blocks();
        assert_eq!(m.cur_block, 3);
        assert_eq!(m.ac[1], W36::ZERO);
        m.ac[1] = W36::wrap(0o333);
        m.ubr = wrubr(m.ubr, W36::from_halves(Half::wrap(0o400000), Half::ZERO));
        m.load_ac_blocks();
        assert_eq!(m.ac[1].get(), 0o111);
        assert_eq!(m.blocks[3][1].get(), 0o333);
    }
}

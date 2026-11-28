//! The PDP-10 parts of the emulator that do not depend on the machine.
//!
//! - [`word`]: words of 1 to 63 bits with the carries of the PDP-10; the
//!   36-bit guest word [`W36`] and the half word [`Half`].
//! - [`instructions`]: the opcodes of the KS10 with the ITS microcode, from
//!   `spec/instructions.toml` through `make gen`.
//! - [`impldef`]: the implementation differences of `spec/impldef.toml`
//!   for the differential runs, and the predicates of their operands.

pub mod impldef;
#[rustfmt::skip]
pub mod instructions;
pub mod word;

pub use word::{Half, Sum, W36, Word};

#[cfg(test)]
mod tests {
    use super::instructions::{AC_OPS, Class, OPS, a, ac_op, b};

    // The expected values come from the ITS microcode, not from the
    // generator: simple.42, DRAM entry 200 "R-PF, AC, J/STAC" (MOVE), and
    // ks10.47, "DISPATCH ROM MACROS": R-PF is A/RD-PF and READ/1; AC is B/AC.
    #[test]
    fn move_entry() {
        let m = &OPS[0o200];
        assert_eq!((m.code, m.name, m.class), (0o200, "MOVE", Class::Inst));
        assert_eq!((m.dram.a, m.dram.b, m.dram.j), (a::RD_PF, b::AC, "STAC"));
        assert!(m.dram.read && !m.dram.test && !m.dram.acdisp);
    }

    #[test]
    fn tables_are_ordered() {
        for (i, op) in OPS.iter().enumerate() {
            assert_eq!(usize::from(op.code), i);
        }
        let acdisp = OPS.iter().filter(|o| o.class == Class::AcDisp).count();
        assert_eq!(AC_OPS.len(), 16 * acdisp);
        for op in OPS.iter().filter(|o| o.class == Class::AcDisp) {
            for ac in 0..16 {
                assert!(ac_op(op.code, ac).is_some());
            }
        }
        assert!(ac_op(0o200, 0).is_none());
    }

    // inout.50: 1700 (APRID, the ksdefs name), 1701 is UUO; simple.42: 1524
    // is HALT, JRST with AC 4.
    #[test]
    fn ac_dispatch() {
        assert_eq!(
            ac_op(0o700, 0).map(|a| (a.name, a.class)),
            Some(("APRID", Class::Inst))
        );
        assert_eq!(ac_op(0o700, 1).map(|a| a.class), Some(Class::Uuo));
        assert_eq!(ac_op(0o254, 4).map(|a| a.name), Some("HALT"));
    }
}

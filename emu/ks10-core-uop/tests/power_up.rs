//! The power-up sequence of the ITS microcode on the model.
//!
//! The sequence computes a table of the powers of ten in the workspace of
//! the RAM file and checks 10^21 itself before it enters the halt loop
//! (simple, "POWER UP SEQUENCE"). The test compares the table with powers
//! of ten that it computes itself.

mod common;

use common::{FILLS, label, machine, run_until, symbol};
use ks10_core_uop::alu::W40;
use pdp10::W36;

#[test]
fn the_power_up_sequence_computes_the_powers_of_ten() {
    let pwron = label("PWRON");
    let (declo, dechi) = (symbol("WORK", "DECLO"), symbol("WORK", "DECHI"));
    for fill in FILLS {
        let mut m = machine(fill);
        let r = run_until(&mut m, 100_000, |m| m.upc == pwron);
        println!("fill {:o}: {} cycles, {r:?}", fill.get(), m.cycles);
        println!("{}", m.coverage);
        // The table: 10^n as two words, the high word 10^n >> 35 and the
        // low word with bit 0 clear (extend, DBSLOW and ADDCRY: "KEEP LOW
        // WORD POSITIVE").
        for n in 0..=21u32 {
            let p = 10u128.pow(n);
            let lo = W36::wrap((p & ((1 << 35) - 1)) as u64);
            let hi = W36::wrap((p >> 35) as u64);
            let i = n as usize;
            assert_eq!(
                (m.ram_file[dechi + i], m.ram_file[declo + i]),
                (hi, lo),
                "10^{n} with fill {:o}",
                fill.get()
            );
        }
        assert_eq!(r, Ok(()));
        // The constants (simple, "POWER UP SEQUENCE"): MASK has "A ONE IN
        // 36-BITS AND 0 IN BITS -2,-1,36,37"; XWD1 has "1 IN EACH HALF
        // WORD"; ONE is "THE CONSTANT 1". The register numbers are the
        // values of the A field.
        let reg = |name: &str| m.alu.reg[symbol("A", name)];
        assert_eq!(reg("MASK").get(), 0o777777_777777 << 2);
        assert_eq!(
            ks10_core_uop::alu::word(reg("XWD1")),
            W36::wrap(0o000001_000001)
        );
        assert_eq!(ks10_core_uop::alu::word(reg("ONE")), W36::wrap(1));
        assert_eq!(ks10_core_uop::alu::word(reg("PC")), W36::ZERO);
        // "ENTER EXEC MODE", "CLEAR TRAP FLAGS".
        assert!(!m.flags.user && !m.flags.trap1 && !m.flags.trap2);
    }
}

/// After PWRON the microcode stores the halt code in location 0 and PC in
/// location 1, sets HALT LOOP and waits for CONTINUE (inout, PWRON, STOPC,
/// H1, HALTLP). The power-up halt code is 0 (ks10, `HALT/POWER=0`), and
/// the power-up sequence clears PC.
///
/// The write to location 0 depends on the cycle type in DPM5 E33 at power
/// on, which no reset clears. The loop for the powers of ten leaves VMA
/// at an address that is not an AC. If E33 holds READ and WRITE, RPW
/// CYCLE is 1 at PWRON: START CYCLE is 0 (DPM5 E32, E5), so there is no
/// bus request (DPM5 E46) and no MEMORY CYCLE, and DPMA WRITE CYCLE needs
/// MEMORY CYCLE (DPMA E17, E21). Location 0 then keeps its content. The
/// fill with all ones has READ and WRITE in E33.
#[test]
fn power_up_ends_in_the_halt_loop() {
    let haltlp = label("HALTLP");
    for fill in FILLS {
        let mut m = machine(fill);
        let r = run_until(&mut m, 100_000, |m| m.upc == haltlp && m.csl.halt_loop);
        println!("fill {:o}: {} cycles, {r:?}", fill.get(), m.cycles);
        assert_eq!(r, Ok(()));
        let location_0 = if fill == W40::ZERO {
            W36::ZERO
        } else {
            ks10_core_uop::alu::word(fill)
        };
        assert_eq!(m.memory[0], location_0);
        assert_eq!(m.memory[1], W36::ZERO);
        // The loop stays in HALTLP while CONTINUE is 0.
        run_until(&mut m, 20, |_| false).unwrap_err();
        assert_eq!(m.upc & !1, haltlp & !1, "upc {:04o}", m.upc);
    }
}

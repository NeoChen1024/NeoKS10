//! The power-up sequence of the ITS microcode on the model.
//!
//! The sequence computes a table of the powers of ten in the workspace of
//! the RAM file and checks 10^21 itself before it enters the halt loop
//! (simple, "POWER UP SEQUENCE"). The test compares the table with powers
//! of ten that it computes itself.
//!
//! The test needs the listing of the ITS build: `KS10_MCR_LISTING` (`make
//! microcode` sets it). Without it the test prints SKIPPED.

use ks10uop::alu::W40;
use ks10uop::cram::{CRAM_WORDS, Microword};
use ks10uop::listing::{self, DROM_WORDS};
use ks10uop::machine::{Machine, PcFlags, Stop};
use pdp10::W36;

/// The value of a workspace name in the listing, for example `DECLO=344`
/// (ks10, "# REDEFINED WHEN USED AS WORSPACE ADDRESS").
fn work(text: &str, name: &str) -> usize {
    let key = format!("\t{name}=");
    let line = text.lines().find(|l| l.contains(&key)).expect(name);
    let v = line.split(&key).nth(1).unwrap();
    let digits: String = v.chars().take_while(|c| c.is_ascii_digit()).collect();
    usize::from_str_radix(&digits, 8).unwrap()
}

fn machine(text: &str, fill: W40) -> Machine {
    let l = listing::parse(text).expect("listing");
    let mut cram = vec![Microword::from_bits(0); CRAM_WORDS];
    for (a, w) in &l.cram {
        cram[usize::from(*a)] = *w;
    }
    let mut drom = vec![0; DROM_WORDS];
    for (a, w) in &l.drom {
        drom[usize::from(*a)] = *w;
    }
    Machine::new(cram, drom, fill)
}

#[test]
fn the_power_up_sequence_computes_the_powers_of_ten() {
    let Ok(path) = std::env::var("KS10_MCR_LISTING") else {
        println!("SKIPPED: KS10_MCR_LISTING is not set");
        return;
    };
    let text = String::from_utf8_lossy(&std::fs::read(path).expect("listing")).into_owned();
    let pwron = listing::label(&text, "PWRON").expect("PWRON");
    let (declo, dechi) = (work(&text, "DECLO"), work(&text, "DECHI"));
    for fill in [W40::ZERO, W40::wrap(0xA5_A5A5_A5A5)] {
        let mut m = machine(&text, fill);
        if fill != W40::ZERO {
            m.flags = PcFlags {
                ov: true,
                cry0: true,
                cry1: true,
                fov: true,
                fpd: true,
                user: true,
                pcu: true,
                trap2: true,
                trap1: true,
                fxu: true,
                nodiv: true,
            };
        }
        let mut stop = None;
        while m.upc != pwron && m.cycles < 100_000 {
            if let Err(e) = m.step() {
                stop = Some(e);
                break;
            }
        }
        println!(
            "fill {:o}: {} cycles, upc {:04o}, {stop:?}",
            fill.get(),
            m.cycles,
            m.upc
        );
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
        assert_eq!(stop, None::<Stop>);
        assert_eq!(m.upc, pwron);
        // The constants (simple, "POWER UP SEQUENCE"): MASK has "A ONE IN
        // 36-BITS AND 0 IN BITS -2,-1,36,37"; XWD1 has "1 IN EACH HALF
        // WORD"; ONE is "THE CONSTANT 1". The register numbers are the
        // values of the A field in the listing.
        let reg = |name: &str| m.alu.reg[work(&text, name)];
        assert_eq!(reg("MASK").get(), 0o777777_777777 << 2);
        assert_eq!(ks10uop::alu::word(reg("XWD1")), W36::wrap(0o000001_000001));
        assert_eq!(ks10uop::alu::word(reg("ONE")), W36::wrap(1));
        assert_eq!(ks10uop::alu::word(reg("PC")), W36::ZERO);
        // "ENTER EXEC MODE", "CLEAR TRAP FLAGS".
        assert!(!m.flags.user && !m.flags.trap1 && !m.flags.trap2);
    }
}

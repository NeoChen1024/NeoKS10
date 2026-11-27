//! Helpers for the tests that need the listing of the ITS build.

#![allow(dead_code)]

use ks10uop::alu::W40;
use ks10uop::cram::{CRAM_WORDS, Microword};
use ks10uop::listing::{self, DROM_WORDS};
use ks10uop::machine::{Machine, Stop};

/// The text of the listing (`KS10_MCR_LISTING`), or None after a SKIPPED
/// line.
pub fn listing_text(test: &str) -> Option<String> {
    let Ok(path) = std::env::var("KS10_MCR_LISTING") else {
        println!("SKIPPED: {test}: KS10_MCR_LISTING is not set");
        return None;
    };
    Some(String::from_utf8_lossy(&std::fs::read(path).expect("listing")).into_owned())
}

/// The value of a workspace or register name in the listing, for example
/// `DECLO=344` (ks10, "# REDEFINED WHEN USED AS WORSPACE ADDRESS").
pub fn symbol(text: &str, name: &str) -> usize {
    let key = format!("\t{name}=");
    let line = text.lines().find(|l| l.contains(&key)).expect(name);
    let v = line.split(&key).nth(1).unwrap();
    let digits: String = v.chars().take_while(|c| c.is_ascii_digit()).collect();
    usize::from_str_radix(&digits, 8).unwrap()
}

pub fn label(text: &str, name: &str) -> u16 {
    listing::label(text, name).expect(name)
}

pub fn machine(text: &str, fill: W40) -> Machine {
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

/// The two fills of each test: 0 and a pattern of alternate bits.
pub const FILLS: [W40; 2] = [W40::ZERO, W40::wrap(0xA5_A5A5_A5A5)];

/// Step until `done` or a stop; at most `limit` cycles.
pub fn run_until(
    m: &mut Machine,
    limit: u64,
    done: impl Fn(&Machine) -> bool,
) -> Result<(), String> {
    let start = m.cycles;
    while !done(m) {
        if m.cycles - start >= limit {
            return Err(format!("no end after {limit} cycles, upc {:04o}", m.upc));
        }
        m.step()
            .map_err(|Stop::Unmodelled { addr, what }| format!("stop at {addr:04o}: {what}"))?;
    }
    Ok(())
}

//! The control RAM and the dispatch ROM of the ITS build.
//!
//! The RAM file comes from the disk of the ITS build, read as the 8080 reads
//! it before it starts the microcode (`ks10::boot::read_microcode`). The
//! listing `KSHACK; MCR 262` comes from the dump tape of the same build
//! (`make microcode`). Both are third-party files and are not in git: the
//! test runs only if KS10_ITS_DISK names `rp0.dsk` and KS10_MCR_LISTING
//! names the listing. Without them it prints SKIPPED.
//!
//! The test decodes the 4000 RAM file entries with the layout of
//! `.RAMFILE` and compares each field with the U words of the listing. The
//! two files come from different steps of the build (MICRO writes both;
//! KSFEDR copies the RAM file to the disk), and the decoding uses the
//! generated field tables, so a wrong field position or a wrong parity rule
//! makes the test fail. It also compares the D words with the instruction
//! table, which comes from the source text of the microcode.

use ks10::{Machine, boot};
use ks10uop::cram::{self, CRAM_WORDS, Microword};
use ks10uop::fields::d;
use ks10uop::listing::{self, DROM_WORDS, drom_field};
use pdp10::W36;

/// The SHA-256 of the RAM file of microcode 262, as 8-byte little-endian
/// words. [measured] 2026-11-27 from the disk of the ITS build; the
/// listing of the same build agrees with the historical listing in
/// KS10FPGA (microcode/ITS/mcr.262, commit 5090e7a1) on all U and D words.
const RAM_FILE_SHA256: &str = "fa2396254dd99ecc62f67c0b499bd232453dfa6e102bd0fae24dacbe3f160844";

fn inputs() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let disk = std::env::var_os("KS10_ITS_DISK");
    let list = std::env::var_os("KS10_MCR_LISTING");
    match (disk, list) {
        (Some(d), Some(l)) => Some((d.into(), l.into())),
        _ => {
            eprintln!("SKIPPED: KS10_ITS_DISK and KS10_MCR_LISTING must name rp0.dsk and MCR 262");
            None
        }
    }
}

fn sha256(words: &[W36]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for w in words {
        h.update(w.get().to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_ram_file_agrees_with_the_listing() {
    let Some((disk, list)) = inputs() else {
        return;
    };
    // The 8080 path reads only; a read-only attachment keeps the image as it is.
    let f = std::fs::File::open(&disk).expect("open the disk image");
    let mut m = Machine::new();
    m.io.disk.attach(0, f, true);
    let words = boot::read_microcode(&mut m, 0).expect("read the microcode");
    assert_eq!(words.len(), 3 * CRAM_WORDS);
    let hash = sha256(&words);
    eprintln!("RAM file SHA-256 {hash}");
    if !RAM_FILE_SHA256.is_empty() {
        assert_eq!(hash, RAM_FILE_SHA256, "a different microcode");
    }

    let cram = cram::cram_from_ram_file(&words).expect("pads and parity of every entry");
    let text = std::fs::read(&list).expect("read the listing");
    let l = listing::parse(&String::from_utf8_lossy(&text)).expect("parse the listing");
    assert_eq!(l.drom.len(), DROM_WORDS);
    // The dispatch ROM words against the instruction table, which comes
    // from the microcode source text (spec/instructions.toml), not from
    // the listing.
    for op in &pdp10::instructions::OPS {
        let w = l.drom[&op.code];
        let flag = |f| drom_field(w, f) != 0;
        let t = op.dram;
        let got = (
            drom_field(w, d::A) as u8,
            drom_field(w, d::B) as u8,
            [flag(d::ACDISP), flag(d::I), flag(d::READ), flag(d::TEST)],
            [flag(d::COND_FUNC), flag(d::VMA), flag(d::WRITE)],
        );
        let want = (
            t.a,
            t.b,
            [t.acdisp, t.i, t.read, t.test],
            [t.cond_func, t.vma, t.write],
        );
        assert_eq!(got, want, "DROM {:03o}", op.code);
    }

    let mask = cram::ram_mask();
    let mut outside = 0u128;
    for (addr, mw) in cram.iter().enumerate() {
        match l.cram.get(&(addr as u16)) {
            Some(want) => {
                assert_eq!(mw.bits(), want.bits() & mask, "CRAM {addr:04o}");
                outside |= want.bits() & !mask;
            }
            None => assert_eq!(
                *mw,
                Microword::default(),
                "CRAM {addr:04o} is not in the listing"
            ),
        }
    }
    let used = cram
        .iter()
        .filter(|mw| **mw != Microword::default())
        .count();
    eprintln!(
        "{} U words in the listing, {used} nonzero CRAM words; listing bits outside the RAM file: {outside:o}",
        l.cram.len()
    );
    assert!(l.cram.len() <= CRAM_WORDS);
}

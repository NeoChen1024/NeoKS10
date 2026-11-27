//! The control RAM and the dispatch ROM of the ITS microcode.
//!
//! `ucode` has the words that spec/micro.py assembles from the ITS
//! microcode sources. These tests compare them with data from other
//! sources:
//!
//! - The RAM file that `ucode::CRAM` gives with the layout of `.RAMFILE`
//!   has the SHA-256 of the RAM file on the disk of the ITS build.
//! - The D words agree with the instruction table, which comes from the
//!   source text of the microcode by another program (spec/instructions.toml).
//! - The RAM file on the disk, read as the 8080 reads it before it starts
//!   the microcode (`ks10_machine::boot::read_microcode`), has the right
//!   pads and parity bits, and decodes to `ucode::CRAM`. This test needs
//!   KS10_ITS_DISK (`make microcode` sets it); without it, it prints
//!   SKIPPED.
//!
//! The decoding uses the generated field tables, so a wrong field position
//! or a wrong parity rule makes the tests fail.

use ks10_core_soft::Machine;
use ks10_core_uop::cram::{self, CRAM_WORDS, Microword};
use ks10_core_uop::drom::drom_field;
use ks10_core_uop::fields::d;
use ks10_core_uop::ucode;
use ks10_machine::boot;
use pdp10::W36;

/// The SHA-256 of the RAM file of microcode 262, as 8-byte little-endian
/// words. [measured] 2026-11-27 from the disk of the ITS build; the RAM
/// file `ram.262` in KS10FPGA (microcode/ITS, commit 5090e7a1) has the same
/// words.
const RAM_FILE_SHA256: &str = "fa2396254dd99ecc62f67c0b499bd232453dfa6e102bd0fae24dacbe3f160844";

fn sha256(words: &[W36]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for w in words {
        h.update(w.get().to_le_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The RAM file of the generated control RAM. An address without a word
/// is three zero words, as on the disk.
#[test]
fn the_generated_microcode_gives_the_ram_file_of_the_disk() {
    let mut words = vec![W36::ZERO; 3 * CRAM_WORDS];
    for &(addr, bits) in ucode::CRAM {
        let at = 3 * usize::from(addr);
        words[at..at + 3].copy_from_slice(&cram::to_ram_entry(Microword::from_bits(bits)));
    }
    assert_eq!(sha256(&words), RAM_FILE_SHA256);
}

#[test]
fn the_dispatch_rom_agrees_with_the_instruction_table() {
    for op in &pdp10::instructions::OPS {
        let w = ucode::DROM[usize::from(op.code)];
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
}

#[test]
fn the_ram_file_of_the_disk_decodes_to_the_generated_microcode() {
    let Some(disk) = std::env::var_os("KS10_ITS_DISK") else {
        println!("SKIPPED: RAM file: KS10_ITS_DISK is not set");
        return;
    };
    // The 8080 path reads only; a read-only attachment keeps the image as it is.
    let f = std::fs::File::open(disk).expect("open the disk image");
    let mut m = Machine::new();
    m.io.disk.attach(0, f, true);
    let words = boot::read_microcode(&mut m, 0).expect("read the microcode");
    assert_eq!(words.len(), 3 * CRAM_WORDS);
    assert_eq!(sha256(&words), RAM_FILE_SHA256, "a different microcode");

    let cram = cram::cram_from_ram_file(&words).expect("pads and parity of every entry");
    let mask = cram::ram_mask();
    let mut want = vec![Microword::default(); CRAM_WORDS];
    let mut outside = 0u128;
    for &(addr, bits) in ucode::CRAM {
        want[usize::from(addr)] = Microword::from_bits(bits & mask);
        outside |= bits & !mask;
    }
    for (addr, (got, want)) in cram.iter().zip(&want).enumerate() {
        assert_eq!(got, want, "CRAM {addr:04o}");
    }
    println!(
        "{} words of ucode; bits outside the RAM file: {outside:o}",
        ucode::CRAM.len()
    );
    assert_eq!(outside, 0);
}

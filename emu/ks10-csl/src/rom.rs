//! The ROM of the 8080: version 5.2 of the console program, as the four
//! 2 KiB images of KS10_CSL_V5.2.zip (bitsavers, `third-party.lock`).
//!
//! **[measured]** 2026-11-28: each byte of the listing `KNS10.PNT`
//! (the tape ks10_8080_microcode, `third-party.lock`) is the byte of one
//! image at this address: E81 at 0000, E73 at 4000 (octal), E58 at 10000,
//! E18 at 14000. `tests/rom.rs` checks this.

use std::path::{Path, PathBuf};

/// The size of one ROM chip in bytes.
pub const CHIP: usize = 0o4000;

/// The chips and their first 8080 addresses.
pub const CHIPS: [(&str, u16); 4] = [
    ("E81", 0o00000),
    ("E73", 0o04000),
    ("E58", 0o10000),
    ("E18", 0o14000),
];

/// The size of the ROM in bytes: 8 KiB, 0 to 17777 (octal).
pub const SIZE: usize = 4 * CHIP;

/// The directory that `make csl` fills, if `KS10_CSL_DIR` names one.
pub fn dir() -> Option<PathBuf> {
    std::env::var_os("KS10_CSL_DIR").map(PathBuf::from)
}

/// The directory of the ROM images below the directory of `make csl`.
pub fn image_dir(d: &Path) -> PathBuf {
    d.join("rom").join("KS10_CSL_V5.2")
}

/// The listing of version 5.2 below the directory of `make csl`.
pub fn listing(d: &Path) -> PathBuf {
    d.join("tape").join("kns10.pnt")
}

/// Load the four images of `images` into one ROM.
pub fn load(images: &Path) -> Result<Vec<u8>, String> {
    let mut rom = vec![0; SIZE];
    for (chip, at) in CHIPS {
        let p = images.join(format!("{chip}.BIN"));
        let b = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        if b.len() != CHIP {
            return Err(format!("{}: {} bytes, not {CHIP}", p.display(), b.len()));
        }
        let at = usize::from(at);
        rom[at..at + CHIP].copy_from_slice(&b);
    }
    Ok(rom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_fill_the_rom() {
        let mut at: Vec<usize> = CHIPS.iter().map(|&(_, a)| usize::from(a)).collect();
        at.sort();
        assert_eq!(at, [0, CHIP, 2 * CHIP, 3 * CHIP]);
    }

    #[test]
    fn load_puts_each_chip_at_its_address() {
        let d = std::env::temp_dir().join(format!("ks10-csl-rom-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        for (k, (chip, _)) in CHIPS.iter().enumerate() {
            std::fs::write(d.join(format!("{chip}.BIN")), vec![k as u8 + 1; CHIP]).unwrap();
        }
        let rom = load(&d).unwrap();
        assert_eq!(
            (rom[0], rom[0o4000], rom[0o10000], rom[0o17777]),
            (1, 2, 3, 4)
        );
        std::fs::write(d.join("E18.BIN"), [0; 10]).unwrap();
        assert!(load(&d).unwrap_err().contains("10 bytes"));
        std::fs::remove_dir_all(&d).unwrap();
    }
}

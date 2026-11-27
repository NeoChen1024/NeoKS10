//! The SAV format of TOPS-10 (a loader for stand-alone programs, as the
//! diagnostics). A SAV file is a list of blocks: a word `-n,,a-1` (an IOWD)
//! and then n words for the addresses a to a+n-1. A word `JRST s` ends the
//! file; s is the start address **[emulator]** (sims 9556eca0,
//! PDP10/kx10_sys.c, load_sav; the same in KLH10 6d733f2a, src/klh10.c).

use pdp10::{Half, W36};

/// How 36-bit words are packed into bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Packing {
    /// 5 bytes for each word: 7 bits of the word in each of the bytes 1 to
    /// 4 and in the low bits of byte 5, bit 35 in bit 7 of byte 5. This is
    /// the output of `t10backup` of the ITS tree (tools/tapeutils,
    /// pars_5chars) and the "-c" format of sims (get_word).
    Ansi5,
    /// 8 bytes for each word, little-endian (the disk image format).
    Le64,
}

/// The words of `bytes`. A short last word is padded with zero bytes.
pub fn words(bytes: &[u8], packing: Packing) -> Vec<W36> {
    match packing {
        Packing::Ansi5 => bytes
            .chunks(5)
            .map(|c| {
                let mut b = [0u8; 5];
                b[..c.len()].copy_from_slice(c);
                let w = (u64::from(b[0] & 0o177) << 29)
                    | (u64::from(b[1] & 0o177) << 22)
                    | (u64::from(b[2] & 0o177) << 15)
                    | (u64::from(b[3] & 0o177) << 8)
                    | (u64::from(b[4] & 0o177) << 1)
                    | u64::from(b[4] >> 7);
                W36::wrap(w)
            })
            .collect(),
        Packing::Le64 => bytes
            .chunks(8)
            .map(|c| {
                let mut b = [0u8; 8];
                b[..c.len()].copy_from_slice(c);
                W36::wrap(u64::from_le_bytes(b))
            })
            .collect(),
    }
}

/// An error in a SAV file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SavError {
    /// A block goes past the end of the file.
    Truncated { block: usize },
    /// A word is neither an IOWD nor JRST.
    BadWord { index: usize, word: W36 },
    /// No JRST at the end.
    NoStart,
}

/// A SAV file: the blocks (first address, words) and the start address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sav {
    pub blocks: Vec<(Half, Vec<W36>)>,
    pub start: Half,
}

/// The blocks and the start address of a SAV file.
pub fn parse(w: &[W36]) -> Result<Sav, SavError> {
    let mut i = 0;
    let mut blocks = Vec::new();
    while i < w.len() {
        let x = w[i];
        if x.is_negative() {
            let n = (1u64 << 18) - x.lh().get();
            let start = (i + 1, i + 1 + n as usize);
            if start.1 > w.len() {
                return Err(SavError::Truncated {
                    block: blocks.len(),
                });
            }
            blocks.push((Half::wrap(x.rh().get() + 1), w[start.0..start.1].to_vec()));
            i = start.1;
        } else if x.field(0, 8) == 0o254 && x.field(9, 17) == 0 {
            return Ok(Sav {
                blocks,
                start: x.rh(),
            });
        } else {
            return Err(SavError::BadWord { index: i, word: x });
        }
    }
    Err(SavError::NoStart)
}

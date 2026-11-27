//! The words of a MICRO listing (the ITS build writes `KSHACK; MCR 262`).
//!
//! A control RAM word is a line `U aaaa, gggg,...` with the address and
//! nine groups of four octal digits, 108 bits from the left. A dispatch ROM
//! word is a line `D aaaa, gggg,gggg,gggg`, 36 bits **[microcode]** (the
//! listing of the ITS build; micro, PNTB). Other lines are text.

use std::collections::BTreeMap;

use crate::cram::Microword;

/// The number of dispatch ROM words: one for each opcode.
pub const DROM_WORDS: usize = 512;

/// The words of a listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// Control RAM words by address.
    pub cram: BTreeMap<u16, Microword>,
    /// Dispatch ROM words by opcode (36 bits each).
    pub drom: BTreeMap<u16, u64>,
}

/// The value of a field of a 36-bit dispatch ROM word (bit 0 at the left).
pub fn drom_field(word: u64, f: crate::fields::Field) -> u32 {
    let width = u32::from(f.last) - u32::from(f.first) + 1;
    ((word >> (35 - u32::from(f.last))) & ((1u64 << width) - 1)) as u32
}

/// A line that looks like a word but is not one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListingError {
    pub line: usize,
    pub text: String,
}

/// The octal groups of a word line `X aaaa, g,g,...`, or None if the line
/// is not one.
fn groups(line: &str, tag: char) -> Option<(u16, Vec<u16>)> {
    let rest = line.strip_prefix(tag)?.strip_prefix(' ')?;
    let (addr, rest) = rest.split_once(", ")?;
    let addr = u16::from_str_radix(addr, 8).ok()?;
    let data = rest.split(|c: char| c.is_whitespace() || c == ';').next()?;
    let g: Option<Vec<u16>> = data
        .split(',')
        .map(|x| {
            (x.len() == 4)
                .then(|| u16::from_str_radix(x, 8).ok())
                .flatten()
        })
        .collect();
    Some((addr, g?))
}

/// Read the U and D words of a listing. Lines that start with `U ` or `D `
/// but have no comma after the address are cross references and are
/// skipped. A word line with a bad group, a second word at one address, or
/// a D address past 777 is an error.
pub fn parse(text: &str) -> Result<Listing, ListingError> {
    let mut out = Listing::default();
    for (n, line) in text.lines().enumerate() {
        let err = || ListingError {
            line: n + 1,
            text: line.to_string(),
        };
        for (tag, count) in [('U', 9), ('D', 3)] {
            let Some(rest) = line.strip_prefix(tag).and_then(|r| r.strip_prefix(' ')) else {
                continue;
            };
            if !rest
                .split_whitespace()
                .next()
                .is_some_and(|a| a.ends_with(','))
            {
                continue;
            }
            let (addr, g) = groups(line, tag).ok_or_else(err)?;
            if g.len() != count {
                return Err(err());
            }
            let v = g.iter().fold(0u128, |acc, &x| (acc << 12) | u128::from(x));
            let fresh = if tag == 'U' {
                out.cram.insert(addr, Microword::from_bits(v)).is_none()
            } else {
                usize::from(addr) < DROM_WORDS && out.drom.insert(addr, v as u64).is_none()
            };
            if !fresh {
                return Err(err());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::u;

    #[test]
    fn reads_u_and_d_words_and_skips_cross_references() {
        let text = "\
U 0000, 0002,3771,0012,4374,4007,0700,0000,0037,7777\t; 2294\t0:\t[MASK]_#
D 0000\t\t3910\t5854
D 0001, 0001,1740,2100\t\t\t\t\t; 5
some text U 0001, 0000
";
        let l = parse(text).unwrap();
        assert_eq!(l.cram.len(), 1);
        let mw = l.cram[&0];
        assert_eq!(mw.get(u::J), 0o0002);
        assert_eq!(mw.get(u::NUM), 0o377777);
        assert_eq!(l.drom.get(&1), Some(&0o0001_1740_2100));
        assert_eq!(drom_field(0o0001_1740_2100, crate::fields::d::B), 0o1);
        assert_eq!(drom_field(0o0001_1740_2100, crate::fields::d::J), 0o1740);
        assert_eq!(l.drom.len(), 1);
    }

    #[test]
    fn rejects_a_bad_group_and_a_second_word() {
        assert_eq!(parse("U 0000, 0002,3771\n").unwrap_err().line, 1);
        assert_eq!(parse("D 0001, 0001,1740,2108\n").unwrap_err().line, 1);
        let two = "D 0001, 0001,1740,2100\nD 0001, 0001,1740,2100\n";
        assert_eq!(parse(two).unwrap_err().line, 2);
        assert_eq!(parse("D 1000, 0001,1740,2100\n").unwrap_err().line, 1);
    }
}

//! The data pattern of the block memprobe (spec/regmap.toml, note "Data
//! pattern"): FILL writes it, CHECK expects it.

/// The value of the 32-bit word at byte offset `off` of the HP memory.
pub fn word(seed: u32, off: u32) -> u32 {
    let w = off >> 2;
    seed ^ w ^ (w << 11) ^ (w << 22)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The expected values are the recorded values of tests/memprobe-pattern.txt.
    #[test]
    fn recorded_values() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/memprobe-pattern.txt"
        );
        let text = std::fs::read_to_string(path).unwrap();
        let mut n = 0;
        for line in text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let v: Vec<u32> = line
                .split_whitespace()
                .map(|x| u32::from_str_radix(x, 16).unwrap())
                .collect();
            assert_eq!(
                word(v[0], v[1]),
                v[2],
                "seed {:08x} offset {:08x}",
                v[0],
                v[1]
            );
            n += 1;
        }
        assert!(n > 0);
    }
}

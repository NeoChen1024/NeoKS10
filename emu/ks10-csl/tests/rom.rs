//! The ROM images against the listing of the same version (KNS10.PNT): the
//! listing is the output of the DEC cross assembler, recorded data that
//! does not come from this crate. A line of the listing has its line
//! number, the 8080 address and up to three bytes, in octal.

use ks10_csl::rom;

#[test]
fn the_images_agree_with_the_listing() {
    let Some(d) = rom::dir() else {
        eprintln!("SKIPPED: KS10_CSL_DIR is not set (make csl)");
        return;
    };
    let rom = rom::load(&rom::image_dir(&d)).expect("the ROM images");
    let p = rom::listing(&d);
    let bytes = std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let text = String::from_utf8_lossy(&bytes);
    let octal =
        |s: &str| (s.len() == 3 || s.len() == 6) && s.bytes().all(|c| (b'0'..=b'7').contains(&c));
    let (mut n, mut first, mut last) = (0, usize::MAX, 0);
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3
            || !f[0].bytes().all(|c| c.is_ascii_digit())
            || f[1].len() != 6
            || !octal(f[1])
        {
            continue;
        }
        let at = usize::from_str_radix(f[1], 8).unwrap();
        for (k, b) in f[2..]
            .iter()
            .take(3)
            .take_while(|b| b.len() == 3 && octal(b))
            .enumerate()
        {
            let a = at + k;
            let want = u8::from_str_radix(b, 8).unwrap();
            assert!(a < rom::SIZE, "listing address {a:o} outside the ROM");
            assert_eq!(
                rom[a], want,
                "address {a:o}: image {:03o}, listing {want:03o}",
                rom[a]
            );
            n += 1;
            first = first.min(a);
            last = last.max(a);
        }
    }
    eprintln!("{n} bytes of the listing agree, from {first:o} to {last:o}");
    assert!(n > 7000, "too few bytes in the listing: {n}");
}

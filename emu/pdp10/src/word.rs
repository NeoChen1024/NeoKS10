//! Words of N bits, with N from 1 to 63.
//!
//! [`Word<N>`] keeps an N-bit value in a `u64`. The bits above N are always
//! 0, so a sum of two words fits in the `u64` with its carry. The guest word
//! is [`W36`]; a half word is [`Half`]. `Word<64>` does not compile.
//!
//! Signed values: the methods without a prefix use two's complement, the
//! representation of the PDP-10. The methods `ones_*` use one's complement
//! (negation inverts all bits, and the sum has an end-around carry), for
//! machines and data formats that use it.
//!
//! Bit numbers follow DEC (AGENTS.md, "Conventions"): bit 0 is the most
//! significant bit, bit N-1 the least significant bit.

use std::fmt;

/// A word of `N` bits, 0 < N < 64.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Word<const N: u32>(u64);

/// The 36-bit word of the PDP-10.
pub type W36 = Word<36>;
/// A half word (18 bits).
pub type Half = Word<18>;

/// The result of an addition or a subtraction, with the carries of the
/// PDP-10: `cry0` out of bit 0, `cry1` out of bit 1 into bit 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sum<const N: u32> {
    pub word: Word<N>,
    pub cry0: bool,
    pub cry1: bool,
}

impl<const N: u32> Sum<N> {
    /// Two's complement overflow: the carries out of bit 0 and bit 1 differ.
    pub const fn overflow(self) -> bool {
        self.cry0 != self.cry1
    }
}

/// The result of a one's complement addition or subtraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OnesSum<const N: u32> {
    pub word: Word<N>,
    /// The carry out of bit 0, which was added back at bit N-1.
    pub end_around: bool,
    /// The operands had the same sign and the result has the other sign.
    pub overflow: bool,
}

impl<const N: u32> Word<N> {
    /// Number of bits.
    pub const BITS: u32 = {
        assert!(N > 0 && N < 64, "a Word has 1 to 63 bits");
        N
    };
    /// All N bits set.
    pub const MASK: u64 = (1u64 << Self::BITS) - 1;
    /// Bit 0 (the sign bit).
    pub const SIGN: u64 = 1u64 << (Self::BITS - 1);
    pub const ZERO: Self = Word(0);
    pub const ONES: Self = Word(Self::MASK);

    /// The word with value `v`, or `None` if `v` has more than N bits.
    pub const fn new(v: u64) -> Option<Self> {
        if v & !Self::MASK == 0 {
            Some(Word(v))
        } else {
            None
        }
    }

    /// The low N bits of `v`.
    pub const fn wrap(v: u64) -> Self {
        Word(v & Self::MASK)
    }

    /// The value, 0 to 2^N - 1.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The value as a two's complement number.
    pub const fn signed(self) -> i64 {
        if self.0 & Self::SIGN != 0 {
            self.0 as i64 - (1i64 << Self::BITS)
        } else {
            self.0 as i64
        }
    }

    /// The low N bits of a two's complement number.
    pub const fn from_signed(v: i64) -> Self {
        Self::wrap(v as u64)
    }

    /// Bit 0 is set.
    pub const fn is_negative(self) -> bool {
        self.0 & Self::SIGN != 0
    }

    /// Bit `n` (DEC numbering).
    pub const fn bit(self, n: u32) -> bool {
        assert!(n < Self::BITS);
        self.0 >> (Self::BITS - 1 - n) & 1 != 0
    }

    /// Bits `first` to `last` (DEC numbering), right-aligned.
    pub const fn field(self, first: u32, last: u32) -> u64 {
        assert!(first <= last && last < Self::BITS);
        let width = last - first + 1;
        (self.0 >> (Self::BITS - 1 - last)) & ((1u64 << width) - 1)
    }

    /// The word with bits `first` to `last` (DEC numbering) replaced by the
    /// low bits of `v`.
    pub const fn with_field(self, first: u32, last: u32, v: u64) -> Self {
        assert!(first <= last && last < Self::BITS);
        let width = last - first + 1;
        let shift = Self::BITS - 1 - last;
        let mask = ((1u64 << width) - 1) << shift;
        Word((self.0 & !mask) | ((v << shift) & mask))
    }

    /// `self + other`, with the carries.
    pub const fn add(self, other: Self) -> Sum<N> {
        Self::sum(self.0, other.0, 0)
    }

    /// `self - other`, as `self + !other + 1`, with the carries of that sum.
    pub const fn sub(self, other: Self) -> Sum<N> {
        Self::sum(self.0, !other.0 & Self::MASK, 1)
    }

    /// `0 - self`, with the carries.
    pub const fn neg(self) -> Sum<N> {
        Self::ZERO.sub(self)
    }

    const fn sum(a: u64, b: u64, cin: u64) -> Sum<N> {
        // a and b have N bits: the sums fit in a u64 with their carry.
        let full = a + b + cin;
        let low = Self::MASK >> 1;
        let into_bit0 = (a & low) + (b & low) + cin;
        Sum {
            word: Word(full & Self::MASK),
            cry0: full >> Self::BITS != 0,
            cry1: into_bit0 >> (Self::BITS - 1) != 0,
        }
    }

    /// The value as a one's complement number. Both zeros give 0.
    pub const fn ones_signed(self) -> i64 {
        if self.0 & Self::SIGN != 0 {
            -((!self.0 & Self::MASK) as i64)
        } else {
            self.0 as i64
        }
    }

    /// The one's complement word of `v`, or `None` if `v` does not fit
    /// (|v| < 2^(N-1)). 0 gives +0.
    pub const fn from_ones_signed(v: i64) -> Option<Self> {
        let limit = (Self::SIGN - 1) as i64;
        if v > limit || v < -limit {
            None
        } else if v < 0 {
            Some(Word(!((-v) as u64) & Self::MASK))
        } else {
            Some(Word(v as u64))
        }
    }

    /// All bits set: the negative zero of one's complement.
    pub const fn is_minus_zero(self) -> bool {
        self.0 == Self::MASK
    }

    /// One's complement negation: all bits inverted.
    pub const fn ones_neg(self) -> Self {
        self.not()
    }

    /// One's complement `self + other`: the carry out of bit 0 is added
    /// at bit N-1 (end-around carry).
    pub const fn ones_add(self, other: Self) -> OnesSum<N> {
        let full = self.0 + other.0;
        let carry = full >> Self::BITS;
        let word = Word((full + carry) & Self::MASK);
        let same = (self.0 ^ other.0) & Self::SIGN == 0;
        OnesSum {
            word,
            end_around: carry != 0,
            overflow: same && (word.0 ^ self.0) & Self::SIGN != 0,
        }
    }

    /// One's complement `self - other`, as `self + (-other)`.
    pub const fn ones_sub(self, other: Self) -> OnesSum<N> {
        self.ones_add(other.ones_neg())
    }

    /// Bitwise operations.
    pub const fn and(self, o: Self) -> Self {
        Word(self.0 & o.0)
    }
    pub const fn or(self, o: Self) -> Self {
        Word(self.0 | o.0)
    }
    pub const fn xor(self, o: Self) -> Self {
        Word(self.0 ^ o.0)
    }
    pub const fn not(self) -> Self {
        Word(!self.0 & Self::MASK)
    }

    /// Number of octal digits of a word.
    pub const DIGITS: usize = Self::BITS.div_ceil(3) as usize;
    /// Number of hexadecimal digits of a word.
    pub const HEX_DIGITS: usize = Self::BITS.div_ceil(4) as usize;

    /// The word of the text `text` in `radix` (2 to 16), for example
    /// `parse_radix("1f", 16)`. Only digits: no sign, prefix or separator.
    /// `None` if a character is not a digit of the radix or the value has
    /// more than N bits.
    pub fn parse_radix(text: &str, radix: u32) -> Option<Self> {
        if !(2..=16).contains(&radix) || text.is_empty() || !text.chars().all(|c| c.is_digit(radix))
        {
            return None;
        }
        Self::new(u64::from_str_radix(text, radix).ok()?)
    }
}

impl W36 {
    /// The left half (bits 0 to 17).
    pub const fn lh(self) -> Half {
        Word(self.0 >> 18)
    }

    /// The right half (bits 18 to 35).
    pub const fn rh(self) -> Half {
        Word(self.0 & Half::MASK)
    }

    /// The word `lh,,rh`.
    pub const fn from_halves(lh: Half, rh: Half) -> Self {
        Word((lh.0 << 18) | rh.0)
    }

    /// The halves exchanged.
    pub const fn swap(self) -> Self {
        Self::from_halves(self.rh(), self.lh())
    }
}

/// Octal with all digits, for example `000012` for a [`Half`]: the form of
/// PDP-10 values in logs and documents (AGENTS.md, "Conventions"). For other
/// radixes, use `{:x}`, `{:X}`, `{:o}` or `{:b}`.
impl<const N: u32> fmt::Display for Word<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Octal::fmt(self, f)
    }
}

/// The radix formats write all digits of the word (for example 9
/// hexadecimal digits for a [`W36`]). `#` adds the prefix `0x`, `0o` or
/// `0b`; a width pads the result as for the integer types.
macro_rules! radix_fmt {
    ($trait:ident, $fmt:literal, $digits:expr, $prefix:literal) => {
        impl<const N: u32> fmt::$trait for Word<N> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let s = format!($fmt, self.0, width = $digits);
                f.pad_integral(true, $prefix, &s)
            }
        }
    };
}
radix_fmt!(Octal, "{:0width$o}", Self::DIGITS, "0o");
radix_fmt!(LowerHex, "{:0width$x}", Self::HEX_DIGITS, "0x");
radix_fmt!(UpperHex, "{:0width$X}", Self::HEX_DIGITS, "0x");
radix_fmt!(Binary, "{:0width$b}", Self::BITS as usize, "0b");

impl<const N: u32> fmt::Debug for Word<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Word<{N}>({self})")
    }
}

impl W36 {
    /// The octal text `lh,,rh` of DDT and of the documents.
    pub fn halves_text(self) -> String {
        format!("{},,{}", self.lh(), self.rh())
    }

    /// The word of the text `lh,,rh` or of one octal number, as
    /// [`W36::halves_text`] and [`fmt::Display`] write it. `None` if the text
    /// is not octal or a value does not fit.
    pub fn parse(text: &str) -> Option<Self> {
        let oct = |s: &str| {
            if s.is_empty() || !s.bytes().all(|c| (b'0'..=b'7').contains(&c)) {
                None
            } else {
                u64::from_str_radix(s, 8).ok()
            }
        };
        match text.split_once(",,") {
            Some((l, r)) => Some(Self::from_halves(Half::new(oct(l)?)?, Half::new(oct(r)?)?)),
            None => Self::new(oct(text)?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The expected results come from integer arithmetic, not from the code
    // under test: for small widths, every pair of operands.
    fn check_all<const N: u32>() {
        let m = 1i64 << N;
        let half = m / 2;
        for a in 0..m {
            for b in 0..m {
                let (x, y) = (Word::<N>::wrap(a as u64), Word::<N>::wrap(b as u64));
                let s = x.add(y);
                assert_eq!(s.word.get() as i64, (a + b) % m);
                assert_eq!(s.cry0, a + b >= m, "cry0 {a}+{b}");
                assert_eq!(s.cry1, (a % half) + (b % half) >= half, "cry1 {a}+{b}");
                let signed = x.signed() + y.signed();
                assert_eq!(
                    s.overflow(),
                    !(-half..half).contains(&signed),
                    "ovf {a}+{b}"
                );

                let d = x.sub(y);
                assert_eq!(d.word.get() as i64, (a - b).rem_euclid(m));
                // a - b = a + (m - 1 - b) + 1: the carries of that sum.
                let nb = m - 1 - b;
                assert_eq!(d.cry0, a + nb + 1 >= m, "cry0 {a}-{b}");
                assert_eq!(d.cry1, (a % half) + (nb % half) + 1 >= half, "cry1 {a}-{b}");
                let signed = x.signed() - y.signed();
                assert_eq!(
                    d.overflow(),
                    !(-half..half).contains(&signed),
                    "ovf {a}-{b}"
                );
            }
        }
    }

    #[test]
    fn arithmetic_small_widths() {
        check_all::<1>();
        check_all::<4>();
        check_all::<5>();
        check_all::<7>();
    }

    // One's complement: every pair of words, both zeros included; the
    // expected values come from integer arithmetic.
    fn check_ones<const N: u32>() {
        let m = 1u64 << N;
        let limit = (m / 2 - 1) as i64;
        for a in 0..m {
            for b in 0..m {
                let (x, y) = (Word::<N>::wrap(a), Word::<N>::wrap(b));
                for (op, r, want) in [
                    ("+", x.ones_add(y), x.ones_signed() + y.ones_signed()),
                    ("-", x.ones_sub(y), x.ones_signed() - y.ones_signed()),
                ] {
                    let fits = (-limit..=limit).contains(&want);
                    assert_eq!(r.overflow, !fits, "overflow {a}{op}{b}");
                    if fits {
                        assert_eq!(r.word.ones_signed(), want, "value {a}{op}{b}");
                    }
                }
                let s = x.ones_add(y);
                assert_eq!(s.end_around, a + b >= m, "end-around {a}+{b}");
            }
            let x = Word::<N>::wrap(a);
            assert_eq!(x.ones_neg().ones_signed(), -x.ones_signed());
            if !x.is_minus_zero() {
                assert_eq!(Word::<N>::from_ones_signed(x.ones_signed()), Some(x));
            }
        }
        assert_eq!(Word::<N>::from_ones_signed(limit + 1), None);
        assert_eq!(Word::<N>::from_ones_signed(-limit - 1), None);
    }

    #[test]
    fn ones_complement_small_widths() {
        check_ones::<2>();
        check_ones::<4>();
        check_ones::<5>();
        check_ones::<7>();
    }

    #[test]
    fn ones_complement_36_bits() {
        let five = W36::from_ones_signed(5).unwrap();
        assert_eq!(five.ones_neg().get(), 0o777777777772);
        // x + (-x) gives -0 without an end-around carry.
        let z = five.ones_add(five.ones_neg());
        assert!(z.word.is_minus_zero() && !z.end_around && !z.overflow);
        // -1 + -1 = -2 with an end-around carry.
        let m1 = W36::from_ones_signed(-1).unwrap();
        let s = m1.ones_add(m1);
        assert_eq!((s.word.ones_signed(), s.end_around), (-2, true));
        let max = W36::from_ones_signed((1 << 35) - 1).unwrap();
        assert!(max.ones_add(W36::wrap(1)).overflow);
    }

    #[test]
    fn arithmetic_36_bits() {
        let max = W36::wrap(0o377777777777);
        let s = max.add(W36::wrap(1));
        assert_eq!(
            (s.word.get(), s.cry0, s.cry1),
            (0o400000000000, false, true)
        );
        assert!(s.overflow());
        let m1 = W36::ONES;
        let s = m1.add(m1);
        assert_eq!((s.word.get(), s.cry0, s.cry1), (0o777777777776, true, true));
        assert!(!s.overflow());
        let min = W36::wrap(0o400000000000);
        assert!(min.neg().overflow());
        assert_eq!(min.neg().word, min);
        assert_eq!(W36::wrap(5).neg().word.signed(), -5);
        assert_eq!(min.signed(), -(1 << 35));
    }

    #[test]
    fn bits_and_fields() {
        let w = W36::wrap(0o200040_000017);
        assert!(!w.bit(0) && w.bit(1) && w.bit(35));
        // Opcode bits 0 to 8, AC bits 9 to 12, Y bits 18 to 35.
        assert_eq!(w.field(0, 8), 0o200);
        assert_eq!(w.field(9, 12), 0o1);
        assert_eq!(w.field(18, 35), 0o17);
        assert_eq!(w.with_field(9, 12, 0o17).field(9, 12), 0o17);
        assert_eq!(w.with_field(9, 12, 0o17).field(0, 8), 0o200);
        assert_eq!(w.lh().get(), 0o200040);
        assert_eq!(w.swap(), W36::wrap(0o000017_200040));
        assert_eq!(W36::new(1 << 36), None);
        assert_eq!(Half::new(0o777777).map(Half::get), Some(0o777777));
    }

    #[test]
    fn octal_text() {
        let w = W36::wrap(0o200040_000017);
        assert_eq!(w.to_string(), "200040000017");
        assert_eq!(w.halves_text(), "200040,,000017");
        assert_eq!(Half::wrap(0o12).to_string(), "000012");
        assert_eq!(Word::<4>::wrap(0o17).to_string(), "17");
        assert_eq!(W36::parse("200040,,17"), Some(w));
        assert_eq!(W36::parse("200040000017"), Some(w));
        assert_eq!(W36::parse("1000000,,0"), None);
        assert_eq!(W36::parse("8"), None);
        assert_eq!(W36::parse(",,1"), None);
        assert_eq!(W36::parse("-1"), None);
    }

    #[test]
    fn radix_text() {
        let w = W36::wrap(0o200040_000017);
        // 0o200040000017 = 0x40080000F (Python: format(0o200040000017, "x")):
        // 9 hexadecimal digits for 36 bits.
        assert_eq!(format!("{w:x}"), "40080000f");
        assert_eq!(format!("{w:X}"), "40080000F");
        assert_eq!(format!("{w:#x}"), "0x40080000f");
        assert_eq!(format!("{w:o}"), "200040000017");
        assert_eq!(format!("{:x}", Half::wrap(1)), "00001");
        assert_eq!(format!("{:b}", Word::<5>::wrap(0b101)), "00101");
        assert_eq!(format!("{:#b}", Word::<5>::wrap(0b101)), "0b00101");
        assert_eq!(format!("{:>12x}", Word::<16>::wrap(0xbeef)), "        beef");
        assert_eq!(W36::parse_radix("40080000f", 16), Some(w));
        assert_eq!(W36::parse_radix("40080000F", 16), Some(w));
        assert_eq!(Word::<16>::parse_radix("10000", 16), None);
        assert_eq!(Word::<16>::parse_radix("0x10", 16), None);
        assert_eq!(Word::<8>::parse_radix("377", 8).map(Word::get), Some(0xff));
        assert_eq!(Word::<8>::parse_radix("12", 1), None);
    }
}

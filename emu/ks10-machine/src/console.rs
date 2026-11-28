//! The 8080 console as the KS10 sees it: the communication words in low
//! memory and the two APR flags between the KS10 and the 8080.
//!
//! This is a model of the protocol, not of the 8080. The behaviour comes
//! from the ITS version of the 8080 program **[its]** (`hcore.m80`,
//! "KS10 TO 8080 CHARACTER SERVICE", CHRRDY, and "8080 TO KS10 CHARACTER
//! SERVICE", TENCHR):
//!
//! - The KS10 sets the APR flag "KS interrupting the 8080". At its
//!   interrupt, the 8080 examines word 35 (KLINIK output) first. If its
//!   control byte (bits 20 to 27) is not 0, the 8080 handles only that
//!   word. Without a KLINIK line the console is in mode 0 (`subrtn.m80`,
//!   SETM0), and the character is discarded (NULKL).
//! - Else, if the control byte of word 33 (`8CTYOT`) is 1, the 8080 prints
//!   bits 28 to 35 on the CTY. Else it does nothing (NOACTN).
//! - After a character, the 8080 writes 0 into the word and sets the APR
//!   flag "8080 interrupting the KS" (TTOCOM, POKE10).
//! - For a CTY input character, the 8080 writes the character with
//!   control byte 1 (400 + character) into word 32 (`8CTYIN`) and
//!   interrupts the KS (TENCHR).

use std::collections::VecDeque;

use pdp10::{Half, W36};

pub const CTYIN: u32 = 0o32;
pub const CTYOT: u32 = 0o33;
pub const KLKOT: u32 = 0o35;

/// The time from the doorbell of the KS10 to the end of the 8080 service,
/// in instructions. **[estimate]**: 1 character at 9600 baud (10 bits,
/// about 1 ms) at the default 1 µs per instruction (docs/emulator.md,
/// "Time"). The CTY line speed is not known (docs/unknowns.md).
pub const OUTPUT_DELAY: u64 = 1000;

/// The minimum time between two input characters, in instructions; the
/// same estimate.
pub const INPUT_GAP: u64 = 1000;

#[derive(Clone, Debug, Default)]
pub struct Console {
    /// Characters from the host that the 8080 has not yet given to the
    /// KS10.
    pub input: VecDeque<u8>,
    /// Characters that the 8080 printed on the CTY.
    pub output: Vec<u8>,
    /// The instruction count at which the 8080 serves the last doorbell.
    service_at: Option<u64>,
    /// The earliest instruction count for the next input character.
    next_input_at: u64,
    /// The instruction count of the last character of `output`.
    last_output_at: u64,
}

/// What the 8080 does to the KS10 in one service.
pub struct Effect {
    /// Write 0 into this word.
    pub clear: Option<u32>,
    /// Write this word into `CTYIN`.
    pub input: Option<W36>,
    /// Set the APR flag "8080 interrupting the KS".
    pub interrupt: bool,
}

/// The control byte of a communication word: bits 20 to 27.
fn control(w: W36) -> u64 {
    w.field(20, 27)
}

impl Console {
    /// The KS10 set the APR flag "KS interrupting the 8080" at instruction
    /// `now`.
    pub fn doorbell(&mut self, now: u64) {
        if self.service_at.is_none() {
            self.service_at = Some(now + OUTPUT_DELAY);
        }
    }

    /// The instruction count at which the CTY output is quiet: no service
    /// is due, and no character came for [`QUIET`] instructions. `None`
    /// while a service is due.
    pub fn quiet_at(&self) -> Option<u64> {
        match self.service_at {
            Some(_) => None,
            None => Some(self.last_output_at + QUIET),
        }
    }

    /// The earliest instruction count at which the CTY output can be
    /// quiet: a bound for a caller that waits for [`Console::quiet_at`].
    pub fn quiet_bound(&self) -> u64 {
        let q = self.last_output_at + QUIET;
        self.service_at.map_or(q, |t| t.max(q))
    }

    /// The first instruction count at which [`Console::poll`] can do
    /// something, if the KS10 does not change `ctyin` (the word
    /// `CTYIN`) or ring the doorbell.
    pub fn deadline(&self, ctyin: W36) -> u64 {
        let mut t = self.service_at.unwrap_or(u64::MAX);
        if !self.input.is_empty() && ctyin == W36::ZERO {
            t = t.min(self.next_input_at);
        }
        t
    }

    /// The work of the 8080 at the instruction count `now`, on the memory
    /// of the processor: [`Console::poll`] and its writes. True if the
    /// 8080 interrupts the KS (APR flag "8080 interrupting the KS").
    pub fn run(&mut self, now: u64, mem: &mut [W36]) -> bool {
        let Some(e) = self.poll(now, |a| mem[a as usize]) else {
            return false;
        };
        if let Some(a) = e.clear {
            mem[a as usize] = W36::ZERO;
        }
        if let Some(w) = e.input {
            mem[CTYIN as usize] = w;
        }
        e.interrupt
    }

    /// The work of the 8080 at instruction `now`. `word` reads a
    /// communication word.
    fn poll(&mut self, now: u64, word: impl Fn(u32) -> W36) -> Option<Effect> {
        if self.service_at.is_some_and(|t| now >= t) {
            self.service_at = None;
            let n = self.output.len();
            let e = self.serve(&word);
            if self.output.len() != n {
                self.last_output_at = now;
            }
            return Some(e);
        }
        // A host policy, not a rule of the 8080 (TENCHR does not test
        // word 32): give the next character when the KS10 has taken the
        // last one, so that no character is lost.
        if !self.input.is_empty() && now >= self.next_input_at && word(CTYIN) == W36::ZERO {
            let c = self.input.pop_front()?;
            self.next_input_at = now + INPUT_GAP;
            return Some(Effect {
                clear: None,
                input: Some(W36::from_halves(
                    Half::ZERO,
                    Half::wrap(0o400 | u64::from(c)),
                )),
                interrupt: true,
            });
        }
        None
    }

    fn serve(&mut self, word: &impl Fn(u32) -> W36) -> Effect {
        let none = Effect {
            clear: None,
            input: None,
            interrupt: false,
        };
        if control(word(KLKOT)) != 0 {
            // Mode 0: NULKL, a null bit bucket.
            return Effect {
                clear: Some(KLKOT),
                interrupt: true,
                ..none
            };
        }
        let w = word(CTYOT);
        if control(w) != 1 {
            return none;
        }
        self.output.push(w.field(28, 35) as u8);
        Effect {
            clear: Some(CTYOT),
            interrupt: true,
            ..none
        }
    }
}

/// The CTY output is quiet after this many instructions without a
/// character and with no service due (docs/emulator.md, "Input without
/// loss"). An answer of a script comes only then: the DEC diagnostic
/// monitor takes and drops a character that comes while it prints (KLAD
/// sources, SUBSMM.MAC, `$TOUTB`, `$CYTYI`). **[measured]** 2026-11-28,
/// `make diag-soft`: with 53 the monitor loses the answer of DSKAI, with
/// 56 all programs pass. The value is a margin over that **[estimate]**.
pub const QUIET: u64 = 10 * OUTPUT_DELAY;

/// Host input for tests: answers that are typed when their prompts appear
/// in the CTY output, in order.
#[derive(Clone, Debug, Default)]
pub struct Script {
    answers: VecDeque<(Vec<u8>, Vec<u8>)>,
    /// Where the search for the next prompt starts.
    from: usize,
    /// The output length at the last search.
    seen: usize,
    /// The prompt of the next answer has come.
    prompted: bool,
}

impl Script {
    /// Type `text` after `prompt` appears.
    pub fn answer(&mut self, prompt: &str, text: &str) {
        self.answers
            .push_back((prompt.as_bytes().to_vec(), text.as_bytes().to_vec()));
    }

    /// If the prompt of the next answer has come: the earliest
    /// instruction count of the answer. The answer comes when the CTY
    /// output is quiet ([`Console::quiet_at`]).
    pub fn due(&self, con: &Console) -> Option<u64> {
        self.prompted.then(|| con.quiet_bound())
    }

    /// Call between instructions, at instruction count `now`.
    pub fn tick(&mut self, now: u64, con: &mut Console) {
        if !self.prompted && con.output.len() != self.seen {
            self.seen = con.output.len();
            if let Some((p, _)) = self.answers.front() {
                let out = &con.output[self.from..];
                if out.windows(p.len()).any(|w| w == p.as_slice()) {
                    self.prompted = true;
                    self.from = self.seen;
                }
            }
        }
        if self.prompted && con.quiet_at().is_some_and(|t| now >= t) {
            self.prompted = false;
            if let Some((_, t)) = self.answers.pop_front() {
                con.input.extend(t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(w33: u64, w35: u64) -> impl Fn(u32) -> W36 {
        move |a| match a {
            CTYOT => W36::wrap(w33),
            KLKOT => W36::wrap(w35),
            _ => W36::ZERO,
        }
    }

    // hcore.m80, CTYONLY: code 1 in bits 20-27 prints bits 28-35, then
    // TTOCOM clears word 33 and POKE10 interrupts the KS.
    #[test]
    fn output_after_delay() {
        let mut c = Console::default();
        c.doorbell(10);
        assert!(
            c.poll(10 + OUTPUT_DELAY - 1, words(0o400 | 0o101, 0))
                .is_none()
        );
        let e = c.poll(10 + OUTPUT_DELAY, words(0o400 | 0o101, 0)).unwrap();
        assert_eq!(c.output, b"A");
        assert_eq!(e.clear, Some(CTYOT));
        assert!(e.interrupt);
    }

    // A control byte other than 1 does nothing (NOACTN); a KLINIK word
    // goes first and is discarded in mode 0.
    #[test]
    fn noaction_and_klinik() {
        let mut c = Console::default();
        c.doorbell(0);
        let e = c.poll(OUTPUT_DELAY, words(0o1000 | 0o101, 0)).unwrap();
        assert!(!e.interrupt && e.clear.is_none());
        c.doorbell(0);
        let e = c
            .poll(OUTPUT_DELAY, words(0o400 | 0o101, 0o400 | 0o102))
            .unwrap();
        assert_eq!(e.clear, Some(KLKOT));
        assert!(c.output.is_empty());
    }

    // docs/emulator.md, "Input without loss": an answer comes only when
    // no CTY service is due and no character came for QUIET instructions.
    #[test]
    fn an_answer_waits_for_quiet_output() {
        let mut c = Console::default();
        let mut s = Script::default();
        s.answer("A", "x");
        // The KS10 prints "AB": the prompt matches at "A", and "B" is
        // still to come.
        c.doorbell(0);
        c.poll(OUTPUT_DELAY, words(0o400 | 0o101, 0));
        s.tick(OUTPUT_DELAY, &mut c);
        assert_eq!(s.due(&c), Some(OUTPUT_DELAY + QUIET));
        c.doorbell(OUTPUT_DELAY + 10);
        assert!(s.due(&c).is_some_and(|t| t >= 2 * OUTPUT_DELAY + 10));
        s.tick(2 * OUTPUT_DELAY, &mut c);
        c.poll(2 * OUTPUT_DELAY + 10, words(0o400 | 0o102, 0));
        assert_eq!(c.output, b"AB");
        // The quiet time starts again at "B".
        let b = 2 * OUTPUT_DELAY + 10;
        s.tick(b + QUIET - 1, &mut c);
        assert!(c.input.is_empty());
        s.tick(b + QUIET, &mut c);
        assert_eq!(c.input, b"x");
        assert_eq!(s.due(&c), None);
    }

    #[test]
    fn input_waits_for_empty_word() {
        let mut c = Console::default();
        c.input.extend(b"0\r");
        let e = c.poll(0, words(0, 0)).unwrap();
        assert_eq!(e.input.unwrap().get(), 0o400 | 0o60);
        assert!(c.poll(INPUT_GAP, |_| W36::wrap(0o460)).is_none());
        assert_eq!(
            c.poll(INPUT_GAP, words(0, 0)).unwrap().input.unwrap().get(),
            0o415
        );
    }
}

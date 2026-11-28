//! A person at the CTY, for tests: the typist answers prompts of the CTY
//! output with text.
//!
//! The rules are those of the high-level console (`ks10_machine::console`,
//! `Script` and `Console::poll`), so that both paths type the same way:
//!
//! - An answer starts when its prompt has appeared and the CTY output has
//!   been quiet for a time of the KS10 (`Ks10::time`), the unit of
//!   `ks10_machine::console::QUIET`. Some programs drop a character that
//!   comes while they print: the KS10 diagnostic monitor does (`QUIET`).
//!   Some flush the input after work without output: the disk program
//!   clears word 32 at the end of a load (dskdmp, `LOADG1`). The time of
//!   the KS10 measures that work; the time of the 8080 does not.
//! - In an answer, a character goes when the line is free and the KS10 has
//!   cleared word 32. The 8080 program deposits each character in word 32
//!   and does not wait for the KS10 to take the one before (hcore.m80,
//!   `TENCHR`), so input without this wait loses characters, on the board
//!   as in the model (docs/console-interface.md). There is no other wait:
//!   the line gives the speed.
//!
//! The waits are a choice of the tests, not a part of the board: the board
//! has no flow control.

use std::collections::VecDeque;

use crate::board::Csl;
use crate::ks10::Ks10;

/// The CTY input word of the 8080 program (`CTYIN`).
pub const CTYIN: u32 = 0o32;

/// Prompts, and the text that answers each.
#[derive(Debug)]
pub struct Typist {
    answers: VecDeque<(Vec<u8>, Vec<u8>)>,
    typing: VecDeque<u8>,
    /// The prompt of the next answer has appeared.
    prompted: bool,
    /// The CTY output before this position is not searched for the next
    /// prompt.
    from: usize,
    /// The length of the CTY output at the last tick, and the time at
    /// which it last changed.
    seen: usize,
    changed: u64,
    quiet: u64,
}

impl Typist {
    /// A typist that starts an answer after a time `quiet` without CTY
    /// output: in the units of `Ks10::time`, or in 8080 states if the KS10
    /// has no time. Call [`Typist::tick`] at intervals that are short
    /// compared with `quiet`.
    pub fn new(quiet: u64) -> Typist {
        Typist {
            answers: VecDeque::new(),
            typing: VecDeque::new(),
            prompted: false,
            from: 0,
            seen: 0,
            changed: 0,
            quiet,
        }
    }

    /// Type `text` after `prompt` appears on the CTY. The search for the
    /// prompt starts after the last character of the answer before.
    pub fn answer(&mut self, prompt: &str, text: &str) {
        self.answers
            .push_back((prompt.as_bytes().to_vec(), text.as_bytes().to_vec()));
    }

    /// All answers are typed.
    pub fn done(&self) -> bool {
        self.answers.is_empty() && self.typing.is_empty()
    }

    /// Call between runs of the board. If the KS10 gives no word 32
    /// (`Ks10::peek`), the typist waits only for the line.
    pub fn tick<K: Ks10>(&mut self, c: &mut Csl<K>) {
        let now = c.ks10().time().unwrap_or(c.now());
        let out = c.cty_output();
        if out.len() != self.seen {
            self.seen = out.len();
            self.changed = now;
        }
        if self.typing.is_empty()
            && !self.prompted
            && let Some((prompt, _)) = self.answers.front()
        {
            self.prompted = out[self.from..]
                .windows(prompt.len())
                .any(|w| w == prompt.as_slice());
        }
        if self.prompted && c.cty_queued().is_empty() && now >= self.changed + self.quiet {
            self.prompted = false;
            let (_, text) = self.answers.pop_front().unwrap();
            self.typing.extend(text);
        }
        if self.typing.is_empty()
            || c.cty_input_pending() != 0
            || c.ks10().peek(CTYIN).is_some_and(|w| w.get() != 0)
        {
            return;
        }
        let b = self.typing.pop_front().unwrap();
        c.type_cty(&[b]);
        if self.typing.is_empty() {
            self.from = c.cty_output().len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Config;
    use crate::ks10::{Inputs, Reply};
    use crate::rom;
    use pdp10::W36;

    /// A KS10 that has only word 32, and a time that the test sets.
    #[derive(Default)]
    struct Ctyin(W36, Inputs, Option<u64>);

    impl Ks10 for Ctyin {
        fn memory_read(&mut self, _a: u32) -> Result<Reply, &'static str> {
            Err("no memory")
        }
        fn memory_write(&mut self, _a: u32, _v: W36) -> Result<bool, &'static str> {
            Err("no memory")
        }
        fn io_read(&mut self, _a: u32, _b: bool) -> Result<Reply, &'static str> {
            Err("no device")
        }
        fn io_write(&mut self, _a: u32, _v: W36, _b: bool) -> Result<bool, &'static str> {
            Err("no device")
        }
        fn idle_to(&mut self, _now: u64) {}
        fn peek(&self, address: u32) -> Option<W36> {
            (address == CTYIN).then_some(self.0)
        }
        fn time(&self) -> Option<u64> {
            self.2
        }
        fn inputs(&self) -> Inputs {
            self.1
        }
        fn set_inputs(&mut self, i: Inputs) {
            self.1 = i;
        }
        fn halt_loop(&self) -> bool {
            false
        }
    }

    /// The stack at the top of the RAM; the CTY USART with TxEN, RxE and
    /// ER; the prompt `>`; interrupts on, and a loop that reads the status
    /// of the USART, as the program does. RST 7 echoes the received
    /// character.
    fn echo() -> Csl<Ctyin> {
        echo_at(10)
    }

    fn echo_at(char_states: u64) -> Csl<Ctyin> {
        let mut rom = vec![0; rom::SIZE];
        rom[..22].copy_from_slice(&[
            0o061, 0, 0o44, // LXI SP,22000
            0o323, 0o101, // OUT 101
            0o076, 0o116, 0o323, 0o201, // MVI A,116; OUT 201
            0o076, 0o025, 0o323, 0o201, // MVI A,25; OUT 201
            0o076, b'>', 0o323, 0o200, // MVI A,'>'; OUT 200
            0o373, // EI
            0o333, 0o201, // IN 201
            0o303, 18, // JMP 18 (the high byte 0 follows)
        ]);
        // IN 200; OUT 200; EI; RET
        rom[0o70..0o76].copy_from_slice(&[0o333, 0o200, 0o323, 0o200, 0o373, 0o311]);
        let config = Config {
            switches: 0,
            cty_carrier: false,
            klinik_carrier: false,
            char_states,
        };
        Csl::new(rom, config, Ctyin::default()).unwrap()
    }

    fn run(c: &mut Csl<Ctyin>, t: &mut Typist, states: u64) {
        let end = c.now() + states;
        while c.now() < end {
            c.run(1).unwrap();
            t.tick(c);
        }
    }

    #[test]
    fn an_answer_starts_after_the_prompt_and_quiet_output() {
        // No `?`: the first answer waits, and so do the others.
        let mut c = echo();
        let mut t = Typist::new(1000);
        t.answer("?", "x");
        t.answer(">", "ab");
        run(&mut c, &mut t, 3000);
        assert_eq!(c.cty_output(), b">");
        assert!(!t.done());

        // The prompt is the last output: the answer waits 1000 states
        // after it, then its characters follow each other at the speed of
        // the line (10 states each, and the echo).
        let mut c = echo();
        let mut t = Typist::new(1000);
        t.answer(">", "ab");
        let mut prompt = None;
        while c.now() < 900 {
            c.run(1).unwrap();
            t.tick(&mut c);
            if prompt.is_none() && c.cty_output() == b">" {
                prompt = Some(c.now());
            }
        }
        let prompt = prompt.unwrap();
        assert_eq!(c.cty_output(), b">");
        let mut first = None;
        while !t.done() || c.cty_input_pending() != 0 {
            c.run(1).unwrap();
            t.tick(&mut c);
            if first.is_none() && t.typing.len() == 1 {
                first = Some(c.now());
            }
        }
        let first = first.unwrap();
        assert!(first >= prompt + 1000, "{first} after {prompt}");
        run(&mut c, &mut t, 100);
        assert_eq!(c.cty_output(), b">ab");
        assert!(c.now() - first < 200, "{} after {first}", c.now());
    }

    #[test]
    fn the_quiet_time_starts_when_the_usart_has_sent_all() {
        // The program sends `>` and `.`; 2000 states for each character,
        // 500 states of quiet. The answer waits for the end of `.`, not
        // only for the prompt.
        let mut rom = vec![0; rom::SIZE];
        rom[..32].copy_from_slice(&[
            0o061, 0, 0o44, // LXI SP,22000
            0o323, 0o101, // OUT 101
            0o076, 0o116, 0o323, 0o201, // MVI A,116; OUT 201
            0o076, 0o025, 0o323, 0o201, // MVI A,25; OUT 201
            0o076, b'>', 0o323, 0o200, // MVI A,'>'; OUT 200
            0o333, 0o201, 0o346, 1, 0o312, 17, 0, // IN 201; ANI 1; JZ 17
            0o076, b'.', 0o323, 0o200, // MVI A,'.'; OUT 200
            0o333, 0o201, // IN 201
            0o303, 28, // JMP 28 (the high byte 0 follows)
        ]);
        let config = Config {
            switches: 0,
            cty_carrier: false,
            klinik_carrier: false,
            char_states: 2000,
        };
        let mut c = Csl::new(rom, config, Ctyin::default()).unwrap();
        let mut t = Typist::new(500);
        t.answer(">", "a");
        while c.cty_input_pending() == 0 {
            assert!(c.now() < 10_000);
            c.run(1).unwrap();
            t.tick(&mut c);
        }
        assert_eq!(c.cty_output(), b">.");
    }

    #[test]
    fn the_quiet_time_is_a_time_of_the_ks10() {
        // The KS10 does no work: its time stays at 0, and the answer waits
        // however long the 8080 runs.
        let mut c = echo();
        c.ks10_mut().2 = Some(0);
        let mut t = Typist::new(1000);
        t.answer(">", "a");
        run(&mut c, &mut t, 20_000);
        assert_eq!(c.cty_output(), b">");
        c.ks10_mut().2 = Some(999);
        run(&mut c, &mut t, 100);
        assert_eq!(c.cty_output(), b">");
        c.ks10_mut().2 = Some(1000);
        run(&mut c, &mut t, 100);
        assert_eq!(c.cty_output(), b">a");
    }

    #[test]
    fn the_typist_waits_for_the_ks10_to_take_the_character() {
        // The fake KS10 holds a character in word 32 (bit 27 is the valid
        // bit of TENCHR) until the test clears it.
        let held = W36::wrap(0o400 | u64::from(b'z'));
        let mut c = echo();
        c.ks10_mut().0 = held;
        let mut t = Typist::new(100);
        t.answer(">", "ab");
        run(&mut c, &mut t, 2000);
        assert_eq!(c.cty_output(), b">");
        c.ks10_mut().0 = W36::ZERO;
        while c.cty_input_pending() == 0 {
            c.run(1).unwrap();
            t.tick(&mut c);
        }
        c.ks10_mut().0 = held;
        run(&mut c, &mut t, 2000);
        assert_eq!(c.cty_output(), b">a");
        c.ks10_mut().0 = W36::ZERO;
        run(&mut c, &mut t, 500);
        assert_eq!(c.cty_output(), b">ab");
    }

    #[test]
    fn the_typist_waits_for_the_line() {
        // A character time of 500 states: the next character waits until
        // the one before has arrived.
        let mut c = echo_at(500);
        let mut t = Typist::new(100);
        t.answer(">", "abc");
        let end = c.now() + 5000;
        while c.now() < end {
            c.run(1).unwrap();
            t.tick(&mut c);
            assert!(c.cty_input_pending() <= 1);
        }
        assert!(t.done());
    }

    #[test]
    fn the_next_prompt_is_searched_after_the_answer() {
        // The second `>` answer needs a `>` after the `a` is typed: the
        // prompt before it does not count.
        let mut c = echo();
        let mut t = Typist::new(100);
        t.answer(">", "a");
        t.answer(">", "b");
        run(&mut c, &mut t, 3000);
        assert_eq!(c.cty_output(), b">a");
        assert!(!t.done());
    }
}

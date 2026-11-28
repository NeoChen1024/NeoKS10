//! A person at the CTY, for tests: the typist answers prompts of the CTY
//! output with text, one character at a time.
//!
//! The 8080 program deposits each character in word 32 (`CTYIN`) and
//! does not wait for the KS10 to take the one before (hcore.m80,
//! `TENCHR`). The disk program does not read the CTY while it loads ITS.
//! So input without waits loses characters, on the board as in the model
//! (docs/console-interface.md). The typist waits as a person waits for
//! the echo: it types a character only after the KS10 has cleared word 32,
//! and a fixed time after the character before. This is a choice of the
//! test, not a part of the board.

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
    /// The CTY output before this position is not searched for the next
    /// prompt.
    from: usize,
    /// The state count of the last typed character.
    typed: Option<u64>,
    gap: u64,
}

impl Typist {
    /// A typist with `gap` 8080 states from one character to the next.
    pub fn new(gap: u64) -> Typist {
        Typist {
            answers: VecDeque::new(),
            typing: VecDeque::new(),
            from: 0,
            typed: None,
            gap,
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
    /// (`Ks10::peek`), the typist keeps only the time between characters.
    pub fn tick<K: Ks10>(&mut self, c: &mut Csl<K>) {
        if self.typing.is_empty()
            && let Some((prompt, _)) = self.answers.front()
        {
            let out = &c.cty_output()[self.from..];
            if out.windows(prompt.len()).any(|w| w == prompt.as_slice()) {
                let (_, text) = self.answers.pop_front().unwrap();
                self.typing.extend(text);
            }
        }
        let now = c.now();
        if self.typing.is_empty()
            || c.cty_input_pending() != 0
            || c.ks10().peek(CTYIN).is_some_and(|w| w.get() != 0)
            || self.typed.is_some_and(|t| now < t + self.gap)
        {
            return;
        }
        let b = self.typing.pop_front().unwrap();
        c.type_cty(&[b]);
        self.typed = Some(now);
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

    /// A KS10 that has only word 32.
    #[derive(Default)]
    struct Ctyin(W36, Inputs);

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
    fn the_typist_answers_the_prompt_one_character_at_a_time() {
        let mut c = echo();
        let mut t = Typist::new(1000);
        t.answer("?", "x");
        t.answer(">", "ab");
        // No `?`: the first answer waits, and so do the others.
        run(&mut c, &mut t, 3000);
        assert_eq!(c.cty_output(), b">");
        assert!(!t.done());

        let mut c = echo();
        let mut t = Typist::new(1000);
        t.answer(">", "ab");
        run(&mut c, &mut t, 900);
        assert_eq!(c.cty_output(), b">a");
        // The second character comes 1000 states after the first.
        run(&mut c, &mut t, 2000);
        assert_eq!(c.cty_output(), b">ab");
        assert!(t.done());
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
        run(&mut c, &mut t, 50);
        c.ks10_mut().0 = held;
        run(&mut c, &mut t, 2000);
        assert_eq!(c.cty_output(), b">a");
        c.ks10_mut().0 = W36::ZERO;
        run(&mut c, &mut t, 500);
        assert_eq!(c.cty_output(), b">ab");
    }

    #[test]
    fn the_typist_waits_for_the_line() {
        // A character time of 500 states and 100 states between
        // characters: the next character waits until the one before has
        // arrived.
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

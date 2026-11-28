//! The 8251 USART of the CSL board, in the asynchronous mode that the
//! console program sets (`PWRUP`, `CTYCTL` and `REMCTL`).
//!
//! All statements are **[manual]** Intel 98-153B (September 1975), 8251
//! data sheet:
//! - p. 5-136, RESET: a reset makes the device wait for its control words.
//! - p. 5-137, CTS: a low CTS enables the transmission if TxEN is set.
//! - p. 5-137: RxRDY says that the 8251 has a character for the CPU; a
//!   read of the character resets it.
//! - p. 5-139: TxRDY is reset when the CPU writes a character; the 8251
//!   cannot begin transmission until TxEN is set and CTS is low.
//! - p. 5-139: at the end of the stop bit, the character goes into the
//!   receiver buffer and RxRDY rises. If the CPU did not read the previous
//!   character, the new one replaces it and the overrun flag rises.
//! - p. 5-140: the mode instruction of the asynchronous mode; bits D1 D0
//!   are the baud rate factor (00 is the synchronous mode), D3 D2 the
//!   character length.
//! - p. 5-142: after the mode, each control write is a command; command
//!   bit D6 (IR) returns the device to the mode format, D4 (ER) resets
//!   the error flags, and D2 (RxE) enables the receiver. The status bits are D0 TxRDY, D1 RxRDY, D2 TxE,
//!   D3 PE, D4 OE, D5 FE, D6 SYNDET and D7 DSR.
//!
//! These are choices of the model, not statements of the manual:
//! - The board gives the transmitter time in 8080 states. The buffer
//!   moves to the shift register in the same state that the register
//!   becomes empty.
//! - TxRDY is 1 while the buffer is empty, also before the mode word.
//! - The line is perfect: no parity or framing error. A character that
//!   arrives while RxE is 0 or before the mode word is lost. The data
//!   read gives the last received character, 0 at first.

/// A command bit: transmit enable.
const TXEN: u8 = 1 << 0;
/// A command bit: receive enable.
const RXE: u8 = 1 << 2;
/// A command bit: error reset.
const ER: u8 = 1 << 4;
/// A command bit: internal reset.
const IR: u8 = 1 << 6;

/// A status bit: transmitter ready.
pub const TXRDY: u8 = 1 << 0;
/// A status bit: receiver ready.
pub const RXRDY: u8 = 1 << 1;
/// A status bit: the transmitter is empty.
pub const TXE: u8 = 1 << 2;
/// A status bit: overrun error.
pub const OE: u8 = 1 << 4;

/// A control word that the model does not implement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unsupported {
    /// The control word.
    pub mode: u8,
}

/// One 8251.
#[derive(Clone, Debug)]
pub struct Usart {
    /// The next control write is a mode instruction.
    expect_mode: bool,
    mode: u8,
    command: u8,
    /// The CTS input is low (asserted).
    cts: bool,
    /// The time to shift one character out, in 8080 states.
    char_states: u64,
    /// The transmitter buffer.
    holding: Option<u8>,
    /// The character in the shift register and the time its stop bits end.
    shifting: Option<(u8, u64)>,
    /// The characters that left on TxD.
    sent: Vec<u8>,
    /// The receiver buffer.
    received: u8,
    /// RxRDY and the overrun flag.
    rx_ready: bool,
    overrun: bool,
}

impl Usart {
    /// An 8251 after a RESET (p. 5-136), with the CTS input `cts` and a
    /// character time of `char_states`.
    pub fn new(cts: bool, char_states: u64) -> Usart {
        Usart {
            expect_mode: true,
            mode: 0,
            command: 0,
            cts,
            char_states,
            holding: None,
            shifting: None,
            sent: Vec::new(),
            received: 0,
            rx_ready: false,
            overrun: false,
        }
    }

    /// A write with C/D = 1: the mode or a command.
    pub fn write_control(&mut self, v: u8, now: u64) -> Result<(), Unsupported> {
        self.advance(now);
        if self.expect_mode {
            if v & 0b11 == 0 {
                return Err(Unsupported { mode: v });
            }
            self.mode = v;
            self.expect_mode = false;
        } else if v & IR != 0 {
            self.expect_mode = true;
            self.command = 0;
        } else {
            self.command = v;
            if v & ER != 0 {
                self.overrun = false;
            }
        }
        self.advance(now);
        Ok(())
    }

    /// A write with C/D = 0: a character for the transmitter.
    pub fn write_data(&mut self, v: u8, now: u64) {
        self.advance(now);
        let bits = 5 + ((self.mode >> 2) & 0b11);
        self.holding = Some(v & ((1u16 << bits) - 1) as u8);
        self.advance(now);
    }

    /// A read with C/D = 1: the status.
    pub fn status(&mut self, now: u64) -> u8 {
        self.advance(now);
        let mut s = 0;
        if self.rx_ready {
            s |= RXRDY;
        }
        if self.overrun {
            s |= OE;
        }
        if self.holding.is_none() {
            s |= TXRDY;
            if self.shifting.is_none() {
                s |= TXE;
            }
        }
        s
    }

    /// A read with C/D = 0: the receiver buffer.
    pub fn read_data(&mut self, now: u64) -> u8 {
        self.advance(now);
        self.rx_ready = false;
        self.received
    }

    /// A character whose stop bit ends now on RxD.
    pub fn receive(&mut self, c: u8) {
        if self.expect_mode || self.command & RXE == 0 {
            return;
        }
        if self.rx_ready {
            self.overrun = true;
        }
        let bits = 5 + ((self.mode >> 2) & 0b11);
        self.received = c & ((1u16 << bits) - 1) as u8;
        self.rx_ready = true;
    }

    /// The RxRDY output.
    pub fn rx_ready(&self) -> bool {
        self.rx_ready
    }

    /// The characters sent so far.
    pub fn sent(&self) -> &[u8] {
        &self.sent
    }

    /// The characters in the shift register and in the buffer, in the
    /// order that they will be sent.
    pub fn queued(&self) -> Vec<u8> {
        self.shifting
            .map(|(c, _)| c)
            .into_iter()
            .chain(self.holding)
            .collect()
    }

    /// The last command; ER is not kept (p. 5-142).
    pub fn command(&self) -> u8 {
        self.command & !ER
    }

    /// Move the transmitter to the time `now`.
    fn advance(&mut self, now: u64) {
        loop {
            if let Some((c, end)) = self.shifting
                && end <= now
            {
                self.sent.push(c);
                self.shifting = None;
            }
            let enabled = !self.expect_mode && self.command & TXEN != 0 && self.cts;
            match (self.shifting, self.holding) {
                (None, Some(c)) if enabled => {
                    self.holding = None;
                    self.shifting = Some((c, now + self.char_states));
                }
                _ => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mode of `PWRUP` with 8 data bits (p. 5-140: 16X, 8 bits, two
    /// stop bits).
    const MODE_8: u8 = 0b1100_1110;
    /// The command of `PWRUP`: TxEN, RxE, ER (p. 5-142).
    const RUN: u8 = 0o25;

    #[test]
    fn the_first_control_write_is_the_mode() {
        let mut u = Usart::new(true, 0);
        u.write_control(MODE_8, 0).unwrap();
        assert_eq!(u.command(), 0);
        u.write_control(RUN, 0).unwrap();
        assert_eq!(u.command(), RUN & !ER);
    }

    #[test]
    fn internal_reset_returns_to_the_mode_format() {
        let mut u = Usart::new(true, 0);
        u.write_control(MODE_8, 0).unwrap();
        u.write_control(RUN, 0).unwrap();
        // IR (D6): the next control write is a mode again, so this one is
        // not a command.
        u.write_control(IR, 0).unwrap();
        u.write_control(MODE_8 | IR, 0).unwrap();
        assert_eq!(u.command(), 0);
    }

    #[test]
    fn the_synchronous_mode_is_not_modelled() {
        let mut u = Usart::new(true, 0);
        assert_eq!(
            u.write_control(0b0000_1100, 0),
            Err(Unsupported { mode: 0b0000_1100 })
        );
    }

    #[test]
    fn a_character_waits_for_txen_and_cts() {
        let mut u = Usart::new(false, 0);
        u.write_control(MODE_8, 0).unwrap();
        u.write_control(RUN, 0).unwrap();
        u.write_data(b'A', 0);
        // CTS is high: no transmission (p. 5-139), TxRDY stays reset.
        assert_eq!(u.status(100), 0);
        assert!(u.sent().is_empty());

        let mut u = Usart::new(true, 0);
        u.write_control(MODE_8, 0).unwrap();
        u.write_data(b'A', 0);
        assert_eq!(u.status(100), 0);
        u.write_control(RUN, 100).unwrap();
        assert_eq!(u.status(100), TXRDY | TXE);
        assert_eq!(u.sent(), b"A");
    }

    #[test]
    fn txrdy_returns_when_the_shift_register_takes_the_character() {
        let mut u = Usart::new(true, 1000);
        u.write_control(MODE_8, 0).unwrap();
        u.write_control(RUN, 0).unwrap();
        u.write_data(b'A', 10);
        // The buffer moves to the empty shift register at once.
        assert_eq!(u.status(10), TXRDY);
        u.write_data(b'B', 20);
        assert_eq!(u.status(1009), 0);
        assert_eq!(u.sent(), b"");
        assert_eq!(u.queued(), b"AB");
        assert_eq!(u.status(1010), TXRDY);
        assert_eq!(u.sent(), b"A");
        assert_eq!(u.status(2010), TXRDY | TXE);
        assert_eq!(u.sent(), b"AB");
    }

    #[test]
    fn a_received_character_waits_for_the_read() {
        let mut u = Usart::new(true, 0);
        u.write_control(MODE_8, 0).unwrap();
        // Before RxE (p. 5-142), a character is lost.
        u.receive(b'x');
        assert_eq!(u.status(0) & RXRDY, 0);
        u.write_control(RUN, 0).unwrap();
        u.receive(b'a');
        assert_eq!(u.status(0) & (RXRDY | OE), RXRDY);
        assert!(u.rx_ready());
        assert_eq!(u.read_data(0), b'a');
        assert_eq!(u.status(0) & RXRDY, 0);
        // A second character before the read: overrun, the first is lost
        // (p. 5-139). ER resets the flag (p. 5-142).
        u.receive(b'b');
        u.receive(b'c');
        assert_eq!(u.status(0) & (RXRDY | OE), RXRDY | OE);
        assert_eq!(u.read_data(0), b'c');
        u.write_control(RUN, 0).unwrap();
        assert_eq!(u.status(0) & OE, 0);
        // 7 data bits (mode 312 of the CTY) mask bit 7.
        let mut u = Usart::new(true, 0);
        u.write_control(0o312, 0).unwrap();
        u.write_control(RUN, 0).unwrap();
        u.receive(0o341);
        assert_eq!(u.read_data(0), 0o141);
    }

    #[test]
    fn the_character_length_masks_the_data() {
        let mut u = Usart::new(true, 0);
        // 7 bits (L2 L1 = 10, p. 5-140).
        u.write_control(0b0100_1010, 0).unwrap();
        u.write_control(RUN, 0).unwrap();
        u.write_data(0o315, 0);
        assert_eq!(u.sent(), [0o115]);
    }
}

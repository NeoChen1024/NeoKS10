//! The microcode-level model as the KS10 of the board.

use ks10_core_uop::machine::Machine;
use ks10_machine::INSTRUCTIONS_PER_MS;
use ks10_machine::unibus::Reply as IoReply;
use pdp10::W36;

use crate::ks10::{Inputs, Ks10, Reply};

impl Ks10 for Machine {
    /// The 8080 program of the board serves the CTY, not the CTY service
    /// of the model.
    fn attach(&mut self) {
        self.csl_model = true;
    }

    /// A memory that does not have the address does not answer: the rule
    /// of the model for its own cycles (`Machine::step`, NXM ERR;
    /// EK-OKS10-TM-002 5.6.5).
    fn memory_read(&mut self, address: u32) -> Result<Reply, &'static str> {
        Ok(self
            .memory
            .get(address as usize)
            .map_or(Reply::None, |&w: &W36| Reply::Data(w)))
    }

    /// A write to the memory of the model, as the devices write it: the
    /// cache does not see it.
    fn memory_write(&mut self, address: u32, value: W36) -> Result<bool, &'static str> {
        Ok(self
            .memory
            .get_mut(address as usize)
            .map(|w| *w = value)
            .is_some())
    }

    fn peek(&self, address: u32) -> Option<W36> {
        self.memory.get(address as usize).copied()
    }

    fn io_read(&mut self, address: u32, byte: bool) -> Result<Reply, &'static str> {
        let now = self.now();
        Ok(match self.io.read_cycle(address, byte, now) {
            IoReply::Value(v) => Reply::Data(W36::wrap(v)),
            IoReply::NoDevice => Reply::None,
        })
    }

    fn io_write(&mut self, address: u32, value: W36, byte: bool) -> Result<bool, &'static str> {
        let now = self.now();
        Ok(self
            .io
            .write_cycle(&mut self.memory, now, address, value.get(), byte))
    }

    /// The device time of the board: the 8080 time at the rate of the
    /// 1 ms clock of the model. One millisecond is 1638.4 states, the
    /// 8080 clock of 14.7456 MHz / 9 **[manual]** (EK-OKS10-TM-002 5.8.1,
    /// "14.746 MHz"), and `INSTRUCTIONS_PER_MS` units of device time.
    fn idle_to(&mut self, stopped: u64) {
        let target = u128::from(stopped) * u128::from(INSTRUCTIONS_PER_MS) * 5 / 8192;
        let target = u64::try_from(target).unwrap_or(u64::MAX);
        if let Some(d) = target.checked_sub(self.idle) {
            self.idle(d);
        }
    }

    fn bus_reset(&mut self) {
        Machine::bus_reset(self);
    }

    fn dp_reset(&mut self) {
        Machine::dp_reset(self);
    }

    fn cram_reset(&mut self) {
        Machine::cram_reset(self);
    }

    fn cram_address_load(&mut self, data: u16) -> Result<(), &'static str> {
        Machine::cram_address_load(self, data);
        Ok(())
    }

    fn cram_write(&mut self, function: u8, data: u16) -> Result<(), &'static str> {
        Machine::cram_write(self, function, data)
    }

    /// A stop of the processor stays in `pending_stop`; the next `step`
    /// returns it again.
    fn run_ticks(&mut self, ticks: u64) -> Result<u64, &'static str> {
        Machine::run_ticks(self, ticks).map_err(|s| {
            self.pending_stop = Some(s);
            "the processor stopped (Machine::pending_stop)"
        })
    }

    fn take_doorbell(&mut self) -> bool {
        Machine::take_doorbell(self)
    }

    fn int_10(&mut self) {
        Machine::int_10(self);
    }

    fn set_console(&mut self, data: W36, io_data_cycle: bool) {
        self.csl.data = data;
        self.csl.io_data_cycle = io_data_cycle;
    }

    fn inputs(&self) -> Inputs {
        let c = &self.csl;
        Inputs {
            run: c.run,
            execute: c.execute,
            cont: c.cont,
            trap_en: c.trap_en,
            cache_en: c.cache_en,
            one_ms_en: c.one_ms_en,
        }
    }

    fn set_inputs(&mut self, i: Inputs) {
        let c = &mut self.csl;
        (c.run, c.execute, c.cont) = (i.run, i.execute, i.cont);
        (c.trap_en, c.cache_en, c.one_ms_en) = (i.trap_en, i.cache_en, i.one_ms_en);
    }

    fn halt_loop(&self) -> bool {
        self.csl.halt_loop
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ks10_core_uop::alu::W40;

    #[test]
    fn peek_gives_the_memory_word_without_a_cycle() {
        let mut m = Machine::its(W40::ZERO);
        m.memory[0o32] = W36::wrap(0o541);
        let cycles = m.cycles;
        assert_eq!(m.peek(0o32), Some(W36::wrap(0o541)));
        assert_eq!(m.peek(m.memory.len() as u32), None);
        assert_eq!(m.cycles, cycles);
    }

    #[test]
    fn the_device_time_follows_the_8080_clock() {
        // 1638.4 states in 1 ms: 16384 states are 10 ms.
        let mut m = Machine::its(W40::ZERO);
        m.idle_to(16384);
        assert_eq!(
            (m.idle, m.now()),
            (10 * INSTRUCTIONS_PER_MS, 10 * INSTRUCTIONS_PER_MS)
        );
        // The time does not go back, and a part of a unit waits.
        m.idle_to(100);
        m.idle_to(16385);
        assert_eq!(m.idle, 10 * INSTRUCTIONS_PER_MS);
        assert_eq!(m.instructions, 0);
    }
}

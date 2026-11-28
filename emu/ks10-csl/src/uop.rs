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
    fn idle_to(&mut self, now: u64) {
        let target = u128::from(now) * u128::from(INSTRUCTIONS_PER_MS) * 5 / 8192;
        let target = u64::try_from(target).unwrap_or(u64::MAX);
        if let Some(d) = target.checked_sub(self.idle) {
            self.idle(d);
        }
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

//! The microcode-level model as the KS10 of the board.

use ks10_core_uop::machine::Machine;
use pdp10::W36;

use crate::ks10::{Inputs, Ks10, Reply};

impl Ks10 for Machine {
    /// A memory that does not have the address does not answer: the rule
    /// of the model for its own cycles (`Machine::step`, NXM ERR;
    /// EK-OKS10-TM-002 5.6.5).
    fn memory_read(&mut self, address: u32) -> Result<Reply, &'static str> {
        Ok(self
            .memory
            .get(address as usize)
            .map_or(Reply::None, |&w: &W36| Reply::Data(w)))
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

//! Helpers for the tests that run the ITS microcode on the model.

#![allow(dead_code)]

use ks10_core_uop::alu::W40;
use ks10_core_uop::machine::{Machine, PcFlags, Stop};
use ks10_core_uop::ucode;

/// The value of a symbol of a microword field: a register (field `A`) or
/// a workspace address (field `WORK`) of the ITS microcode.
pub fn symbol(field: &str, name: &str) -> usize {
    ucode::symbol(field, name).expect(name) as usize
}

/// The address of a tag of the ITS microcode.
pub fn label(name: &str) -> u16 {
    ucode::tag(name).expect(name)
}

/// The model with the control RAM and the dispatch ROM of the ITS
/// microcode (`ucode`, assembled by spec/micro.py).
pub fn machine(fill: W40) -> Machine {
    Machine::its(fill)
}

/// The two fills of each test: 0 and a pattern of alternate bits.
pub const FILLS: [W40; 2] = [W40::ZERO, W40::wrap(0xA5_A5A5_A5A5)];

/// Step until `done` or a stop; at most `limit` cycles.
pub fn run_until(
    m: &mut Machine,
    limit: u64,
    done: impl Fn(&Machine) -> bool,
) -> Result<(), String> {
    let start = m.cycles;
    while !done(m) {
        if m.cycles - start >= limit {
            return Err(format!("no end after {limit} cycles, upc {:04o}", m.upc));
        }
        m.step()
            .map_err(|Stop::Unmodelled { addr, what }| format!("stop at {addr:04o}: {what}"))?;
    }
    Ok(())
}

/// The program flags of the model in the form of `ks10-core-soft`.
pub fn ks10_flags(f: PcFlags) -> u32 {
    use ks10_core_soft::flags::Flags as F;
    [
        (f.ov, F::AROV),
        (f.cry0, F::CRY0),
        (f.cry1, F::CRY1),
        (f.fov, F::FOV),
        (f.fpd, F::FPD),
        (f.user, F::USER),
        (f.pcu, F::USERIO),
        (f.trap2, F::TRAP2),
        (f.trap1, F::TRAP1),
        (f.fxu, F::FXU),
        (f.nodiv, F::NODIV),
    ]
    .iter()
    .filter(|(v, _)| *v)
    .fold(0, |a, (_, b)| a | b)
}

/// The program flags of `ks10-core-soft` in the form of the model: the
/// inverse of [`ks10_flags`].
pub fn pc_flags(v: u32) -> PcFlags {
    use ks10_core_soft::flags::Flags as F;
    let has = |f: u32| v & f != 0;
    PcFlags {
        ov: has(F::AROV),
        cry0: has(F::CRY0),
        cry1: has(F::CRY1),
        fov: has(F::FOV),
        fpd: has(F::FPD),
        user: has(F::USER),
        pcu: has(F::USERIO),
        trap2: has(F::TRAP2),
        trap1: has(F::TRAP1),
        fxu: has(F::FXU),
        nodiv: has(F::NODIV),
    }
}

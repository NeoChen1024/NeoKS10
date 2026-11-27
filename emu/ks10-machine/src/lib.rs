//! The KS10 machine around the processor (docs/emulator.md). A processor
//! crate (`ks10-core-soft`, `ks10-core-uop`) owns the memory and an
//! [`unibus::Io`], and uses this crate for them.
//!
//! - [`unibus`]: the I/O bus, the adapters; [`rh11`]: the disk; [`tm03`]:
//!   the tape; [`dz11`]: the terminal multiplexer; [`ch11`]: the Chaosnet
//!   interface; [`imp`]: the IMP interface.
//! - [`devregs`]: the device registers, generated from spec/devices.toml.
//! - [`console`]: the 8080 console words.
//! - [`boot`]: the 8080 boot commands.
//! - [`sav`]: the SAV loader for stand-alone programs; [`diag`]: the run
//!   file of the DEC diagnostics.

pub mod boot;
pub mod ch11;
pub mod console;
#[rustfmt::skip]
pub mod devregs;
pub mod diag;
pub mod dz11;
pub mod imp;
pub mod rh11;
pub mod sav;
pub mod tm03;
pub mod unibus;

/// Words of memory: 512 Ki (docs/emulator.md, configuration DB, `TSYSM`).
pub const MEMORY_WORDS: usize = 512 * 1024;

/// Virtual time: completed instructions in one millisecond of the 1 ms
/// clock, for both processors and for the pace of `neoks10`
/// **[estimate]**: 1 µs for each instruction (docs/emulator.md, "Time"). The KS10 instruction times
/// are not known yet (docs/unknowns.md).
pub const INSTRUCTIONS_PER_MS: u64 = 1000;

/// The MSEC counter (DPMC E181, E189, E180) at instruction `now`, for a
/// tick at `last_tick`: it counts 2^12 units in each millisecond of the
/// time base, and stays below 2^12 until the next tick.
pub fn msec_count(now: u64, last_tick: u64) -> u64 {
    let k = now.saturating_sub(last_tick).min(INSTRUCTIONS_PER_MS - 1);
    (k << 12) / INSTRUCTIONS_PER_MS
}

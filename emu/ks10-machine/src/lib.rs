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
//! - [`sav`]: the SAV loader for stand-alone programs.

pub mod boot;
pub mod ch11;
pub mod console;
#[rustfmt::skip]
pub mod devregs;
pub mod dz11;
pub mod imp;
pub mod rh11;
pub mod sav;
pub mod tm03;
pub mod unibus;

/// Words of memory: 512 Ki (docs/emulator.md, configuration DB, `TSYSM`).
pub const MEMORY_WORDS: usize = 512 * 1024;

/// Virtual time: completed instructions in one millisecond of the 1 ms
/// clock, for both processors **[estimate]**: 1 µs for each instruction,
/// the default of docs/emulator.md ("Time"). The KS10 instruction times
/// are not known yet (docs/unknowns.md).
pub const INSTRUCTIONS_PER_MS: u64 = 1000;

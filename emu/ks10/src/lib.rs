//! The KS10 with the behaviour of the ITS microcode (docs/emulator.md).
//!
//! - [`machine`]: the state and the instruction cycle of the software
//!   processor; [`ops`]: the instructions.
//! - [`flags`]: the program flags; [`apr`]: the APR and PI registers.
//! - [`byte`]: the byte instructions.
//! - [`dbl`]: double word instructions and ADJSP.
//! - [`extend`]: the string instructions.
//! - [`float`]: floating point.
//! - [`devregs`]: the device registers, generated from spec/devices.toml.
//! - [`console`]: the 8080 console words.
//! - [`muldiv`]: multiply and divide.
//! - [`unibus`]: the I/O bus, the adapters; [`rh11`]: the disk; [`dz11`]:
//!   the terminal multiplexer; [`ch11`]: the Chaosnet interface; [`imp`]:
//!   the IMP interface.
//! - [`pager`]: the ITS pager; [`io`]: the I/O instructions.
//! - [`pag`]: the EBR and UBR registers (opcode 701).
//! - [`shift`]: shifts, rotations and BLT.
//! - [`timer`]: the time base, the interval timer, the PC samples.
//! - [`boot`]: the 8080 boot commands.
//! - [`sav`]: the SAV loader for stand-alone programs.

pub mod apr;
pub mod boot;
pub mod byte;
pub mod ch11;
pub mod console;
pub mod dbl;
#[rustfmt::skip]
pub mod devregs;
pub mod dz11;
pub mod extend;
pub mod flags;
pub mod float;
pub mod imp;
pub mod io;
pub mod machine;
pub mod muldiv;
pub mod ops;
pub mod pag;
pub mod pager;
pub mod rh11;
pub mod sav;
pub mod shift;
pub mod timer;
pub mod unibus;

pub use machine::{Machine, Stop};

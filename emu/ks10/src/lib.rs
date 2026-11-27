//! The KS10 with the behaviour of the ITS microcode (docs/emulator.md).
//!
//! - [`machine`]: the state and the instruction cycle of the software
//!   processor; [`ops`]: the instructions.
//! - [`flags`]: the program flags; [`apr`]: the APR and PI registers.
//! - [`shift`]: shifts, rotations and BLT.
//! - [`sav`]: the SAV loader for stand-alone programs.

pub mod apr;
pub mod flags;
pub mod machine;
pub mod ops;
pub mod sav;
pub mod shift;

pub use machine::{Machine, Stop};

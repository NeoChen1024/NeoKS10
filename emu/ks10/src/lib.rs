//! The KS10 with the behaviour of the ITS microcode (docs/emulator.md).
//!
//! - [`machine`]: the state and the instruction cycle of the software
//!   processor; [`ops`]: the instructions.
//! - [`flags`]: the program flags; [`apr`]: the APR and PI registers.
//! - [`byte`]: the byte instructions.
//! - [`console`]: the 8080 console words.
//! - [`muldiv`]: multiply and divide.
//! - [`pag`]: the EBR and UBR registers (opcode 701).
//! - [`shift`]: shifts, rotations and BLT.
//! - [`sav`]: the SAV loader for stand-alone programs.

pub mod apr;
pub mod byte;
pub mod console;
pub mod flags;
pub mod machine;
pub mod muldiv;
pub mod ops;
pub mod pag;
pub mod sav;
pub mod shift;

pub use machine::{Machine, Stop};

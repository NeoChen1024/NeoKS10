//! The KS10 with the behaviour of the ITS microcode (docs/emulator.md).
//!
//! - [`machine`]: the state and the instruction cycle of the software
//!   processor; [`ops`]: the instructions.
//! - [`flags`]: the program flags; [`apr`]: the APR and PI registers.
//! - [`byte`]: the byte instructions.
//! - [`dbl`]: double word instructions and ADJSP.
//! - [`extend`]: the string instructions.
//! - [`float`]: floating point.
//! - [`console`]: the 8080 console words.
//! - [`muldiv`]: multiply and divide.
//! - [`pager`]: the ITS pager; [`io`]: the I/O instructions.
//! - [`pag`]: the EBR and UBR registers (opcode 701).
//! - [`shift`]: shifts, rotations and BLT.
//! - [`timer`]: the time base, the interval timer, the PC samples.
//! - [`sav`]: the SAV loader for stand-alone programs.

pub mod apr;
pub mod byte;
pub mod console;
pub mod dbl;
pub mod extend;
pub mod flags;
pub mod float;
pub mod io;
pub mod machine;
pub mod muldiv;
pub mod ops;
pub mod pag;
pub mod pager;
pub mod sav;
pub mod shift;
pub mod timer;

pub use machine::{Machine, Stop};

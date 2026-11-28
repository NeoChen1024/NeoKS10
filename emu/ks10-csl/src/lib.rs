//! The CSL board of the KS10, for tests only: the DEC 8080 console program
//! as the four ROM images of the board, on a model of the 8080 and of the
//! board (docs/console-interface.md, "Comparison"). The system in daily use does not run it; its console is
//! the high-level console of `ks10-machine`.
//!
//! The ROM images and the listing are third-party files and are not in
//! git. `make csl` extracts them from `references/` into the build output,
//! and `KS10_CSL_DIR` names that directory.

pub mod board;
pub mod ks10;
pub mod rom;
pub mod typist;
pub mod uop;
pub mod usart;

pub use board::{Config, Csl, Stop, Write};

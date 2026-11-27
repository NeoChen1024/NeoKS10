//! The KS10 at the level of the microcode (docs/architecture.md, the
//! microcode-level model). A model of the KS10 data path runs the
//! microcode that the ITS build assembles. It is the executable form of
//! the first source of AGENTS.md ("Sources of truth") for the data path
//! and the microcode. `ks10-core-soft` is compared with it, but does not
//! copy its internal state.
//!
//! - [`fields`]: the fields of the microword and of the dispatch ROM word,
//!   and the layout of the RAM file, generated from the ITS microcode
//!   sources by spec/gen_microcode.py.
//! - [`ucode`]: the control RAM words, the dispatch ROM words and the
//!   symbols of the ITS microcode, assembled by spec/micro.py and generated
//!   by spec/gen_microcode.py.
//! - [`cram`]: the 108-bit microword and the RAM file entry.
//! - [`drom`]: the dispatch ROM word.
//! - [`alu`]: the 40-bit arithmetic unit and its shift mixers.
//! - [`machine`]: the microsequencer and the rest of the data path.

pub mod alu;
pub mod cram;
pub mod drom;
#[rustfmt::skip]
pub mod fields;
pub mod machine;
#[rustfmt::skip]
pub mod ucode;

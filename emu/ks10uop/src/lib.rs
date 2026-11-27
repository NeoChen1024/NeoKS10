//! The KS10 at the level of the microcode (docs/architecture.md, the
//! microcode-level model). A model of the KS10 data path runs the
//! microcode that the ITS build assembles. Where the sources disagree about
//! an instruction, this model decides (AGENTS.md, "Sources of truth").
//!
//! - [`fields`]: the fields of the microword and of the dispatch ROM word,
//!   and the layout of the RAM file, generated from the ITS microcode
//!   sources by spec/gen_microcode.py.
//! - [`cram`]: the 108-bit microword and the RAM file entry.
//! - [`listing`]: the U and D words of a MICRO listing.

pub mod cram;
#[rustfmt::skip]
pub mod fields;
pub mod listing;

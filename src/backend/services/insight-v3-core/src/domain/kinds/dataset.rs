//! What a dataset says about the records it holds, and whether it holds up.
//!
//! A dataset is the only relation a custom metric reads. This module owns the
//! declaration and its rules; where the records live and who may change them
//! belongs elsewhere.

pub(crate) mod declaration;
// Its own tests exercise it; the compiler that reads a dataset arrives in the
// step after this one. `expect` rather than `allow`, so the marker fails once
// it does.
#[cfg_attr(not(test), expect(dead_code, reason = "wired up by the compiler"))]
pub(crate) mod read;
pub(crate) mod state;
pub(crate) mod validate;

//! What a dataset says about the records it holds, and whether it holds up.
//!
//! A dataset is the only relation a custom metric reads. This module owns the
//! declaration and its rules; where the records live and who may change them
//! belongs elsewhere.

pub(crate) mod declaration;
pub(crate) mod lifecycle;
pub(crate) mod validate;

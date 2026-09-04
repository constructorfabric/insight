//! The query engine: one query contract over declared datasets.
//!
//! INVARIANT: identity is a dataset's declared policy, and no dataset in this
//! build declares one.

// The engine is assembled bottom-up; its layers are reached from tests until
// the handler lands on top of them.
#![allow(dead_code)]

pub mod datasets;

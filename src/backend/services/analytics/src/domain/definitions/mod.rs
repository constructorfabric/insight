//! Semantic-layer definition grammars: the closed, typed shapes a measure is
//! authored in (filter tree, scalar expressions). This module owns structural
//! validity — a value that parses here is well-formed in isolation. Binding to
//! a dataset's field catalog (does the field exist, does its type admit the
//! operator) is the seed pipeline's second validation stage.

// dead_code: the grammars land ahead of the seed pipeline that consumes them,
// so they can be reviewed and tested in isolation; the allow leaves with it.
#[allow(dead_code)]
pub mod expr;
#[allow(dead_code)]
pub mod filter;

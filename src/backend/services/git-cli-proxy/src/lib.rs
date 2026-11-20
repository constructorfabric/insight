#![expect(
    clippy::missing_errors_doc,
    reason = "lib.rs only serves this crate's binary and tests; service and tool crates carry no doc comments"
)]

pub mod api;
pub mod config;
pub mod engine;
pub mod gear;

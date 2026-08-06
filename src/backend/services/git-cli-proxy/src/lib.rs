//! Insight Git CLI Proxy — clone-based commit data extraction for the nocode
//! git connectors (design: `docs/components/connectors/git/git-cli-proxy/`).
//!
//! Lib/bin split (fakeidp precedent): the binary is a thin bootstrap; all
//! logic lives here so it is a normal library surface for tests and never
//! trips dead-code lints while the API phases land.

pub mod api;
pub mod config;
pub mod engine;
pub mod gear;

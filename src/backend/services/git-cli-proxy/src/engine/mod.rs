//! The git engine: repository cache (blobless bare clones, hashed cache-key
//! layout, fetch-if-stale freshness) and hermetic git subprocess invocation.

pub mod key;
pub mod meta;
pub mod runner;
pub mod store;

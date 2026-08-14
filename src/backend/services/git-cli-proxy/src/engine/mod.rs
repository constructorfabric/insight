//! The git engine: repository cache (blobless bare clones, hashed cache-key
//! layout, fetch-if-stale freshness) and hermetic git subprocess invocation.

pub mod disk;
pub mod index;
pub mod key;
pub mod meta;
pub mod metrics;
pub mod page;
pub mod read;
pub mod runner;
pub mod store;
pub mod url;

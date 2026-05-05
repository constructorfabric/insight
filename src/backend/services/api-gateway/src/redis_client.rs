//! Shared Redis client for the API Gateway pod.
//!
//! Initializes a single `redis::aio::ConnectionManager` from config and
//! publishes it on the ClientHub as `Arc<RedisShared>` so other ModKit
//! modules in the same binary (BFF today, Router tomorrow) can consume it
//! without re-opening a connection pool.
//!
//! # Configuration
//!
//! ```yaml
//! modules:
//!   redis-client:
//!     config:
//!       url: "redis://redis:6379/0"
//! ```

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use modkit::context::ModuleCtx;
use modkit::contracts::Module;
use redis::aio::ConnectionManager;
use serde::Deserialize;
use tracing::info;

/// Module configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RedisClientConfig {
    /// Redis connection URL (e.g. `redis://localhost:6379/0`). Empty means
    /// "no Redis configured" — modules that depend on this client will fail
    /// at their own init when they call `client_hub.get::<RedisShared>()`.
    pub url: String,
}

/// Shared Redis handle. Cheap to clone — wraps `ConnectionManager` which
/// itself is a `Clone` reference into a multiplexed connection pool.
#[derive(Clone)]
pub struct RedisShared {
    manager: ConnectionManager,
}

impl std::fmt::Debug for RedisShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisShared").finish_non_exhaustive()
    }
}

impl RedisShared {
    /// Borrow the underlying multiplexed connection manager.
    ///
    /// Callers clone this to obtain an owned `ConnectionManager` that can be
    /// passed by value to `redis::pipe()` / `cmd().query_async(&mut conn)`.
    #[must_use]
    pub fn manager(&self) -> ConnectionManager {
        self.manager.clone()
    }

    /// Test-only constructor — wraps an externally-built `ConnectionManager`
    /// without going through the module init path. Lets integration tests
    /// drive `SessionStore` against a real Redis from `cargo test`.
    #[cfg(test)]
    #[must_use]
    #[doc(hidden)]
    pub fn __test_from_manager(manager: ConnectionManager) -> Self {
        Self { manager }
    }
}

/// Redis client module. Has no capabilities — its only job is to initialize
/// a connection manager and register it with the ClientHub.
#[modkit::module(name = "redis-client")]
pub struct RedisClientModule {
    shared: OnceLock<Arc<RedisShared>>,
}

impl Default for RedisClientModule {
    fn default() -> Self {
        Self {
            shared: OnceLock::new(),
        }
    }
}

#[async_trait]
impl Module for RedisClientModule {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        let cfg: RedisClientConfig = ctx.config()?;

        if cfg.url.is_empty() {
            anyhow::bail!(
                "redis-client: url is required. \
                 Set modules.redis-client.config.url in your config."
            );
        }

        info!(url = redacted_url(&cfg.url).as_str(), "connecting to Redis");

        let client = redis::Client::open(cfg.url.clone())
            .map_err(|e| anyhow::anyhow!("invalid redis url: {e}"))?;
        let manager = ConnectionManager::new(client)
            .await
            .map_err(|e| anyhow::anyhow!("failed to open redis connection manager: {e}"))?;

        let shared = Arc::new(RedisShared { manager });
        self.shared
            .set(shared.clone())
            .map_err(|_| anyhow::anyhow!("redis-client already initialized"))?;

        ctx.client_hub().register::<RedisShared>(shared);
        info!("redis-client: registered RedisShared on ClientHub");

        Ok(())
    }
}

/// Strip credentials and query string from a Redis URL for safe logging.
fn redacted_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) => {
            let _ = u.set_password(None);
            let _ = u.set_username("");
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        }
        Err(_) => "<unparseable redis url>".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_url_strips_credentials() {
        assert_eq!(
            redacted_url("redis://user:secret@host:6379/0"),
            "redis://host:6379/0"
        );
        assert_eq!(
            redacted_url("redis://host:6379/0?foo=bar"),
            "redis://host:6379/0"
        );
        assert_eq!(redacted_url("redis://host:6379"), "redis://host:6379");
        assert_eq!(redacted_url("not a url"), "<unparseable redis url>");
    }

    #[test]
    fn config_default_has_empty_url() {
        let c = RedisClientConfig::default();
        assert!(c.url.is_empty());
    }
}

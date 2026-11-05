//! Direct access to the rig's Redis, for assertions about what the
//! authenticator stored that no HTTP response exposes (record TTLs, index
//! membership). `run-e2e.sh` provides `E2E_REDIS_URL`; the default matches it.

use redis::aio::MultiplexedConnection;

pub async fn connect() -> MultiplexedConnection {
    let url =
        std::env::var("E2E_REDIS_URL").unwrap_or_else(|_| "redis://localhost:6399".to_owned());
    redis::Client::open(url)
        .expect("E2E_REDIS_URL must be a Redis URL")
        .get_multiplexed_async_connection()
        .await
        .expect("the rig's Redis must accept connections")
}

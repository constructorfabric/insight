//! The settings a request carries travel in its URL, which
//! [`clickhouse::test::Mock`] does not let a handler see.

use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::any;

// INVARIANT: the hostname `clickhouse::test::Mock` hands out. A client built
// with `test-util` rewrites it to loopback, speaks header-less `RowBinary` and
// skips the `DESCRIBE` an insert otherwise opens with; any other host would
// need both answered.
const MOCKED_HOST: &str = "http://mocked.clickhouse.test";

#[derive(Debug, Clone)]
pub struct Received {
    pairs: Vec<(String, String)>,
}

impl Received {
    #[must_use]
    pub fn pair(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone, Default)]
struct Seen(Arc<Mutex<Vec<Received>>>);

#[derive(Debug)]
pub struct Capture {
    url: String,
    seen: Seen,
    server: tokio::task::JoinHandle<io::Result<()>>,
}

impl Capture {
    /// # Errors
    ///
    /// Returns the error if no port could be bound.
    pub async fn start() -> io::Result<Self> {
        let seen = Seen::default();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let app = Router::new().fallback(any(record)).with_state(seen.clone());
        let server = tokio::spawn(axum::serve(listener, app).into_future());

        Ok(Self {
            url: format!("{MOCKED_HOST}:{port}"),
            seen,
            server,
        })
    }

    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    #[must_use]
    pub fn received(&self) -> Vec<Received> {
        self.seen
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn record(
    State(seen): State<Seen>,
    Query(pairs): Query<Vec<(String, String)>>,
) -> StatusCode {
    seen.0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Received { pairs });
    StatusCode::OK
}

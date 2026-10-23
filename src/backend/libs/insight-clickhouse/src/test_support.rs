//! A warehouse that answers nothing and remembers what it was asked.
//!
//! The settings a request carries travel in its URL, which
//! [`clickhouse::test::Mock`] does not let a handler see. This server records
//! every request's URL pairs so a test can pin which settings a call sends —
//! the read-after-write pair, the coordinated-DDL wait — and which it does not.
//!
//! Every request is answered `200` with an empty body: zero rows to a read, an
//! acknowledgement to a write or a statement.

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

/// One request as the warehouse received it: every URL pair, the settings
/// and the `query` a read or insert travels as.
#[derive(Debug, Clone)]
pub struct Received {
    pairs: Vec<(String, String)>,
}

impl Received {
    /// The value of the URL pair `name`, if the request carried one.
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

/// The recording warehouse. Dropping it stops the server.
#[derive(Debug)]
pub struct Capture {
    url: String,
    seen: Seen,
    server: tokio::task::JoinHandle<io::Result<()>>,
}

impl Capture {
    /// Starts listening on a free loopback port.
    ///
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

    /// The URL to build a [`Config`](crate::Config) with.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Every request so far, in arrival order.
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

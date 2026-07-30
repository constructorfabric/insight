//! Shared e2e test support: an HTTP client that mirrors the `reqwest` builder
//! surface and records every response from the authenticator under test into
//! the endpoint-coverage ledger.
//!
//! The ledger lands at `$E2E_COVERAGE_LEDGER` (run-e2e.sh sets it; unset means
//! recording is off and the client is a plain passthrough). Its schema matches
//! the bronze-to-api rig's `observed_endpoints.json` — a JSON list of
//! `{method, path, statuses}` rows — so the same gate script consumes it:
//! `src/ingestion/tests/e2e/lib/api_coverage.py --suite authenticator`.
//!
//! Only requests whose origin matches `$AUTH_BASE` / `$AUTH_BASE_DISABLED`
//! (the two authenticator instances) are recorded: the same client also talks
//! to fakeidp and the service-token listener, and those must not pollute the
//! ledger the authenticator spec is matched against. Each `cargo test --test
//! e2e_*` invocation is its own process, and run-e2e.sh runs them serially, so
//! the read-merge-write below needs no cross-process locking; the in-process
//! mutex covers concurrent tests within one binary.

// Each integration-test crate compiles its own copy of this module and none
// uses the full surface.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

static LEDGER_LOCK: Mutex<()> = Mutex::new(());

/// A `reqwest::Client` that records `(method, path) -> {status}` on `send()`.
#[derive(Clone)]
pub struct Client {
    inner: reqwest::Client,
}

/// The e2e default client: redirects OFF, so 302s from `/auth/login`,
/// `/authorize`, and `/auth/callback` are observed rather than followed.
pub fn client() -> Client {
    Client {
        inner: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client must build"),
    }
}

impl Client {
    pub fn get(&self, url: impl reqwest::IntoUrl) -> RequestBuilder {
        self.request(reqwest::Method::GET, url)
    }

    pub fn post(&self, url: impl reqwest::IntoUrl) -> RequestBuilder {
        self.request(reqwest::Method::POST, url)
    }

    pub fn delete(&self, url: impl reqwest::IntoUrl) -> RequestBuilder {
        self.request(reqwest::Method::DELETE, url)
    }

    pub fn request(&self, method: reqwest::Method, url: impl reqwest::IntoUrl) -> RequestBuilder {
        RequestBuilder {
            client: self.inner.clone(),
            inner: self.inner.request(method, url),
        }
    }
}

/// Thin wrapper over `reqwest::RequestBuilder`; `send()` returns the plain
/// `reqwest::Response`, so call sites downstream of `send()` are untouched.
pub struct RequestBuilder {
    client: reqwest::Client,
    inner: reqwest::RequestBuilder,
}

impl RequestBuilder {
    pub fn header(self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        Self {
            inner: self.inner.header(key.as_ref(), value.as_ref()),
            ..self
        }
    }

    pub fn json<T: serde::Serialize + ?Sized>(self, json: &T) -> Self {
        Self {
            inner: self.inner.json(json),
            ..self
        }
    }

    pub fn form<T: serde::Serialize + ?Sized>(self, form: &T) -> Self {
        Self {
            inner: self.inner.form(form),
            ..self
        }
    }

    pub async fn send(self) -> reqwest::Result<reqwest::Response> {
        // Build first so the request's final method + URL are readable; a
        // build error surfaces exactly like reqwest's own send() would.
        let request = self.inner.build()?;
        let method = request.method().clone();
        let url = request.url().clone();
        let response = self.client.execute(request).await?;
        record(&method, &url, response.status().as_u16());
        Ok(response)
    }
}

/// Merge one observation into the ledger file, if recording is on and the
/// request targeted an authenticator instance.
fn record(method: &reqwest::Method, url: &reqwest::Url, status: u16) {
    let Ok(ledger) = std::env::var("E2E_COVERAGE_LEDGER") else {
        return;
    };
    if !is_authenticator_origin(url) {
        return;
    }
    let _guard = LEDGER_LOCK.lock().expect("ledger lock");

    // (method, path) -> statuses; same row shape api_coverage._dump writes.
    let mut merged: BTreeMap<(String, String), BTreeSet<u16>> = BTreeMap::new();
    if let Ok(existing) = std::fs::read_to_string(&ledger)
        && let Ok(rows) = serde_json::from_str::<Vec<serde_json::Value>>(&existing)
    {
        for row in rows {
            let (Some(m), Some(p), Some(statuses)) = (
                row["method"].as_str(),
                row["path"].as_str(),
                row["statuses"].as_array(),
            ) else {
                continue;
            };
            merged
                .entry((m.to_owned(), p.to_owned()))
                .or_default()
                .extend(
                    statuses
                        .iter()
                        .filter_map(|s| s.as_u64().and_then(|v| u16::try_from(v).ok())),
                );
        }
    }
    merged
        .entry((method.as_str().to_owned(), url.path().to_owned()))
        .or_default()
        .insert(status);

    let rows: Vec<serde_json::Value> = merged
        .into_iter()
        .map(|((m, p), statuses)| {
            serde_json::json!({
                "method": m,
                "path": p,
                "statuses": statuses.into_iter().collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut out = serde_json::to_string_pretty(&rows).expect("ledger serializes");
    out.push('\n');
    if let Some(parent) = std::path::Path::new(&ledger).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&ledger, out).expect("ledger write must succeed");
}

/// True when `url` targets one of the authenticator instances under test
/// (`AUTH_BASE`, and `AUTH_BASE_DISABLED` — `e2e_override`'s second instance).
/// `AUTH_BASE` falls back to the same default the tests themselves use.
fn is_authenticator_origin(url: &reqwest::Url) -> bool {
    let auth_base =
        std::env::var("AUTH_BASE").unwrap_or_else(|_| "http://localhost:8083".to_owned());
    [Some(auth_base), std::env::var("AUTH_BASE_DISABLED").ok()]
        .into_iter()
        .flatten()
        .filter_map(|base| reqwest::Url::parse(&base).ok())
        .any(|base| base.origin() == url.origin())
}

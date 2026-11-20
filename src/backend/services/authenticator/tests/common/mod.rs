// Each integration-test crate compiles its own copy; none uses the full surface.
#![allow(dead_code)]

pub mod kc;
pub mod store;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

static LEDGER_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone)]
pub struct Client {
    inner: reqwest::Client,
}

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

    pub fn query<T: serde::Serialize + ?Sized>(self, query: &T) -> Self {
        Self {
            inner: self.inner.query(query),
            ..self
        }
    }

    pub async fn send(self) -> reqwest::Result<reqwest::Response> {
        let request = self.inner.build()?;
        let method = request.method().clone();
        let url = request.url().clone();
        let response = self.client.execute(request).await?;
        record(&method, &url, response.status().as_u16());
        Ok(response)
    }
}

fn record(method: &reqwest::Method, url: &reqwest::Url, status: u16) {
    let Ok(ledger) = std::env::var("E2E_COVERAGE_LEDGER") else {
        return;
    };
    if !is_authenticator_origin(url) {
        return;
    }
    let _guard = LEDGER_LOCK.lock().expect("ledger lock");

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

fn is_authenticator_origin(url: &reqwest::Url) -> bool {
    let auth_base =
        std::env::var("AUTH_BASE").unwrap_or_else(|_| "http://localhost:8083".to_owned());
    [Some(auth_base), std::env::var("AUTH_BASE_DISABLED").ok()]
        .into_iter()
        .flatten()
        .filter_map(|base| reqwest::Url::parse(&base).ok())
        .any(|base| base.origin() == url.origin())
}

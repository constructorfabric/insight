//! Identity client.
//!
//! Calls the Identity service to look up person info by email.
//! Used by the query engine to enrich results with display names and org data.

use serde::{Deserialize, Serialize};

/// Person info returned by the Identity service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub email: String,
    pub display_name: String,
    pub first_name: String,
    pub last_name: String,
    pub department: String,
    pub division: String,
    pub job_title: String,
    pub status: String,
    pub supervisor_email: Option<String>,
    pub supervisor_name: Option<String>,
    pub subordinates: Vec<Subordinate>,
}

/// Subordinate summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subordinate {
    pub email: String,
    pub display_name: String,
    pub job_title: String,
}

/// Identity API client.
#[derive(Clone)]
pub struct IdentityClient {
    base_url: String,
    http: reqwest::Client,
}

impl IdentityClient {
    /// Create a new client. `base_url` is the identity service root,
    /// e.g. `http://insight-identity:8082`.
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
        }
    }

    /// Look up a person by email address.
    ///
    /// Calls `GET {base_url}/v1/persons/{email}`.
    /// Returns `None` if the person is not found (404).
    ///
    /// # Errors
    ///
    /// Returns error if the service is unreachable or returns an unexpected error.
    pub async fn get_person(&self, email: &str) -> anyhow::Result<Option<Person>> {
        let url = format!("{}/v1/persons/{email}", self.base_url);

        let resp = self.http.get(&url).send().await?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            tracing::warn!(email = %email, status = %status, body = %body, "identity lookup failed");
            anyhow::bail!("identity service returned {status}");
        }

        let person: Person = resp.json().await?;
        Ok(Some(person))
    }

    /// Check if the identity service is configured (URL is non-empty).
    #[must_use]
    pub fn is_configured(&self) -> bool {
        !self.base_url.is_empty()
    }
}

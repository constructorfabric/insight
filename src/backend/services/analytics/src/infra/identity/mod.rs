//! Identity client.
//!
//! Calls the Identity service to look up person info by email.
//! Used by the query engine to enrich results with display names and org data.

use serde::{Deserialize, Serialize};

/// Person info returned by the Identity service.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Subordinate {
    pub email: String,
    pub display_name: String,
    pub job_title: String,
}

// `Person` is the analytics-facing person model (the `GET /v1/persons/{email}`
// response body of *this* service). `Subordinate` is nested inside it and needs
// only `ToSchema` (above).
impl toolkit::api::api_dto::ResponseApiDto for Person {}

/// Request body of the identity service's `POST /v1/profiles`
/// (`ResolveProfileCommandModel`). For an email lookup, `value_type="email"`,
/// `value` is the email, and the two `insight_source_*` fields are omitted.
#[derive(Debug, Serialize)]
struct ResolveProfileRequest<'a> {
    value_type: &'a str,
    value: &'a str,
}

/// Response body of the identity service's `POST /v1/profiles`
/// (`ProfileResponse`). Only the fields analytics maps into its own [`Person`]
/// model are declared; identity omits null-valued optionals from the JSON, so
/// every non-required field is `Option` and defaults on absence.
#[derive(Debug, Deserialize)]
struct ProfileResponse {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    first_name: Option<String>,
    #[serde(default)]
    last_name: Option<String>,
    #[serde(default)]
    department: Option<String>,
    #[serde(default)]
    division: Option<String>,
    #[serde(default)]
    job_title: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    supervisor_email: Option<String>,
    #[serde(default)]
    supervisor_name: Option<String>,
    #[serde(default)]
    subordinates: Vec<ProfileSubordinate>,
}

/// A subordinate entry inside `ProfileResponse.subordinates` (identity's
/// `PersonResponse`). Only the three fields analytics' [`Subordinate`] carries
/// are declared; the rest of `PersonResponse` is ignored.
#[derive(Debug, Deserialize)]
struct ProfileSubordinate {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    job_title: Option<String>,
}

impl From<ProfileResponse> for Person {
    fn from(p: ProfileResponse) -> Self {
        Person {
            // `GET /v1/persons` returned these as required strings; `ProfileResponse`
            // makes them optional (omitted when null). Preserve the analytics `Person`
            // contract by defaulting a missing value to the empty string.
            email: p.email.unwrap_or_default(),
            display_name: p.display_name.unwrap_or_default(),
            first_name: p.first_name.unwrap_or_default(),
            last_name: p.last_name.unwrap_or_default(),
            department: p.department.unwrap_or_default(),
            division: p.division.unwrap_or_default(),
            job_title: p.job_title.unwrap_or_default(),
            status: p.status.unwrap_or_default(),
            // These were already optional on `Person`; forward verbatim.
            supervisor_email: p.supervisor_email,
            supervisor_name: p.supervisor_name,
            subordinates: p
                .subordinates
                .into_iter()
                .map(|s| Subordinate {
                    email: s.email.unwrap_or_default(),
                    display_name: s.display_name.unwrap_or_default(),
                    job_title: s.job_title.unwrap_or_default(),
                })
                .collect(),
        }
    }
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
    /// Calls `POST {base_url}/v1/profiles` with `{ value_type: "email", value:
    /// <email> }` — the contemporary, non-deprecated resolution endpoint. (The
    /// old `GET /v1/persons/{email}` emits RFC 8594 `Deprecation` headers; the
    /// email in the path leaks into observability surfaces, so it must not be
    /// used.) The `ProfileResponse` is mapped into this service's [`Person`]
    /// model, keeping the analytics `/v1/persons/{email}` response unchanged.
    ///
    /// Returns `None` if the person is not found (404) — identity resolves to
    /// exactly one person, so a 422 `ambiguous_profile` (multiple matches) is
    /// surfaced as an error rather than silently collapsed.
    ///
    /// `authorization` is the caller's incoming `Authorization` header (the
    /// gateway JWT). Identity now verifies that JWT itself (`NGINX_BFF` R1), so a
    /// user-context call **must** forward it (G1) — otherwise identity replies
    /// 401. It is threaded through verbatim rather than re-minted: the reissue-
    /// ahead margin guarantees the token still verifies across this extra hop.
    ///
    /// # Errors
    ///
    /// Returns error if the service is unreachable or returns an unexpected error.
    pub async fn get_person(
        &self,
        email: &str,
        authorization: Option<&str>,
    ) -> anyhow::Result<Option<Person>> {
        let url = format!("{}/v1/profiles", self.base_url);

        let body = ResolveProfileRequest {
            value_type: "email",
            value: email,
        };

        let mut req = self.http.post(&url).json(&body);
        if let Some(auth) = authorization {
            req = req.header(reqwest::header::AUTHORIZATION, auth);
        }
        let resp = req.send().await?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            let status = resp.status();
            // No email/body in the log: both are PII (#1846); status is enough.
            tracing::warn!(status = %status, "identity lookup failed");
            anyhow::bail!("identity service returned {status}");
        }

        let profile: ProfileResponse = resp.json().await?;
        Ok(Some(profile.into()))
    }

    /// Check if the identity service is configured (URL is non-empty).
    #[must_use]
    pub fn is_configured(&self) -> bool {
        !self.base_url.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};

    use axum::Router;
    use axum::http::{HeaderMap, StatusCode, header::AUTHORIZATION};
    use axum::routing::post;

    /// What the loopback identity observed: forwarded Authorization + request body.
    type Seen = Arc<Mutex<Option<(Option<String>, serde_json::Value)>>>;

    /// Loopback identity serving `POST /v1/profiles` with a canned reply.
    async fn spawn_identity(status: StatusCode, body: serde_json::Value) -> (String, Seen) {
        let seen: Seen = Arc::default();
        let record = Arc::clone(&seen);
        let app = Router::new().route(
            "/v1/profiles",
            post(
                move |headers: HeaderMap, axum::Json(req): axum::Json<serde_json::Value>| {
                    let record = Arc::clone(&record);
                    let body = body.clone();
                    async move {
                        let auth = headers
                            .get(AUTHORIZATION)
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned);
                        *record.lock().unwrap() = Some((auth, req));
                        (status, axum::Json(body))
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), seen)
    }

    #[tokio::test]
    async fn found_maps_profile_and_forwards_auth() {
        let (url, seen) = spawn_identity(
            StatusCode::OK,
            serde_json::json!({
                "email": "a@example.com",
                "display_name": "Ada",
                "department": "Eng",
                "supervisor_name": "Boss",
                "subordinates": [
                    {"email": "b@example.com", "display_name": "Bob", "job_title": "SRE"},
                    {}
                ],
                "unknown_field": "ignored"
            }),
        )
        .await;

        let person = IdentityClient::new(&url)
            .get_person("a@example.com", Some("Bearer tok"))
            .await
            .unwrap()
            .expect("seeded person resolves");

        assert_eq!(person.email, "a@example.com");
        assert_eq!(person.display_name, "Ada");
        assert_eq!(person.department, "Eng");
        // Omitted profile fields: required strings default to "", options to None.
        assert_eq!(person.first_name, "");
        assert_eq!(person.status, "");
        assert_eq!(person.supervisor_email, None);
        assert_eq!(person.supervisor_name.as_deref(), Some("Boss"));
        assert_eq!(person.subordinates.len(), 2);
        assert_eq!(person.subordinates[0].display_name, "Bob");
        assert_eq!(person.subordinates[1].email, "");

        let (auth, req) = seen.lock().unwrap().take().unwrap();
        assert_eq!(auth.as_deref(), Some("Bearer tok"));
        assert_eq!(
            req,
            serde_json::json!({"value_type": "email", "value": "a@example.com"})
        );
    }

    #[tokio::test]
    async fn not_found_is_none_and_auth_stays_absent() {
        let (url, seen) = spawn_identity(StatusCode::NOT_FOUND, serde_json::json!({})).await;
        let got = IdentityClient::new(&url)
            .get_person("x@example.com", None)
            .await
            .unwrap();
        assert!(got.is_none());
        let (auth, _) = seen.lock().unwrap().take().unwrap();
        assert_eq!(auth, None);
    }

    #[tokio::test]
    async fn unexpected_status_is_error() {
        // 422 ambiguous_profile must surface as an error, not collapse to None.
        let (url, _seen) = spawn_identity(
            StatusCode::UNPROCESSABLE_ENTITY,
            serde_json::json!({"title": "ambiguous_profile"}),
        )
        .await;
        let err = IdentityClient::new(&url)
            .get_person("x@example.com", None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("422"), "{err}");
    }

    #[test]
    fn is_configured_and_base_url_normalization() {
        assert!(!IdentityClient::new("").is_configured());
        let c = IdentityClient::new("http://identity:8082/");
        assert!(c.is_configured());
        assert_eq!(c.base_url, "http://identity:8082");
    }
}

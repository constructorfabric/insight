//! Identity client.
//!
//! Calls the Identity service to resolve which persons the caller may see.
//! Used by the person-visibility gate on the metric-result surfaces.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Request body of the identity service's `POST /v1/visible-persons`
/// (canonical person UUIDs since the identity cutover).
#[derive(Debug, Serialize)]
struct VisiblePersonsRequest<'a> {
    person_ids: &'a [Uuid],
}

/// Response body of `POST /v1/visible-persons`.
#[derive(Debug, Deserialize)]
struct VisiblePersonsResponse {
    // INVARIANT: no serde default — a 200 omitting this field is a contract
    // mismatch, and an empty set here would deny every caller.
    visible: Vec<Uuid>,
}

#[derive(Debug, Serialize)]
struct ProfilesBatchRequest<'a> {
    person_ids: &'a [Uuid],
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfilesBatchResponse {
    profiles: Vec<IdentityProfile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdentityProfile {
    pub(crate) person_id: Uuid,
    pub(crate) attributes: BTreeMap<String, String>,
    pub(crate) supervisor: Option<IdentityProfileRelationship>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdentityProfileRelationship {
    pub(crate) person_id: Uuid,
    pub(crate) attributes: BTreeMap<String, String>,
}

/// The seeded `admin` role id — a stable migration constant of the identity
/// service, mirrored here so a role check needs no extra round trip.
const ADMIN_ROLE_ID: Uuid = Uuid::from_u128(0xa4d1_1000_0000_4000_8000_0000_0000_0001);

#[derive(Debug, serde::Deserialize)]
struct MeResponse {
    roles: Vec<MeRole>,
}

#[derive(Debug, serde::Deserialize)]
struct MeRole {
    role_id: Uuid,
}

/// Identity API client.
#[derive(Clone)]
pub struct IdentityClient {
    base_url: String,
    http: reqwest::Client,
}

impl IdentityClient {
    /// Create a new client. `base_url` is the identity service root,
    /// e.g. `http://insight-identity-resolution:8082`.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be built.
    pub fn new(base_url: &str) -> anyhow::Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()?,
        })
    }

    pub(crate) async fn visible_person_ids(
        &self,
        person_ids: &[Uuid],
        authorization: Option<&str>,
    ) -> anyhow::Result<HashSet<Uuid>> {
        let url = format!("{}/v1/visible-persons", self.base_url);

        let req = self
            .http
            .post(&url)
            .json(&VisiblePersonsRequest { person_ids });
        let resp = Self::send(req, authorization, "visibility check").await?;

        let checked: VisiblePersonsResponse = resp.json().await?;
        Ok(checked.visible.into_iter().collect())
    }

    pub(crate) async fn profiles_batch(
        &self,
        person_ids: &[Uuid],
        authorization: Option<&str>,
    ) -> anyhow::Result<Vec<IdentityProfile>> {
        let url = format!("{}/v1/profiles/batch", self.base_url);

        let req = self
            .http
            .post(&url)
            .json(&ProfilesBatchRequest { person_ids });
        let resp = Self::send(req, authorization, "profile batch").await?;

        let batch: ProfilesBatchResponse = resp.json().await?;
        Ok(batch.profiles)
    }

    /// Whether the caller holds the active `admin` identity role.
    ///
    /// The role lives in the identity service's `person_roles`, not in the
    /// gateway JWT — the `roles` claim carries realm roles, which no identity
    /// endpoint reads.
    pub(crate) async fn is_admin(&self, authorization: Option<&str>) -> anyhow::Result<bool> {
        let url = format!("{}/v1/me", self.base_url);

        let resp = Self::send(self.http.get(&url), authorization, "role lookup").await?;

        let me: MeResponse = resp.json().await?;
        Ok(me.roles.iter().any(|role| role.role_id == ADMIN_ROLE_ID))
    }

    /// Forward the caller's authorization, if the gateway supplied one, and
    /// fail loudly on anything but success.
    async fn send(
        req: reqwest::RequestBuilder,
        authorization: Option<&str>,
        what: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let req = match authorization {
            Some(auth) => req.header(reqwest::header::AUTHORIZATION, auth),
            None => req,
        };
        let resp = req.send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            tracing::warn!(status = %status, what, "identity request failed");
            anyhow::bail!("identity service returned {status}");
        }
        Ok(resp)
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

    async fn spawn_visible_persons(status: StatusCode, body: serde_json::Value) -> (String, Seen) {
        let seen: Seen = Arc::default();
        let record = Arc::clone(&seen);
        let app = Router::new().route(
            "/v1/visible-persons",
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

    async fn spawn_profiles_batch(status: StatusCode, body: serde_json::Value) -> (String, Seen) {
        let seen: Seen = Arc::default();
        let record = Arc::clone(&seen);
        let app = Router::new().route(
            "/v1/profiles/batch",
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
    async fn visible_person_ids_forwards_the_callers_bearer_and_sends_every_id() {
        let (url, seen) = spawn_visible_persons(
            StatusCode::OK,
            serde_json::json!({"visible": ["00000000-0000-0000-0000-00000000000a"]}),
        )
        .await;

        let visible = IdentityClient::new(&url)
            .unwrap()
            .visible_person_ids(
                &[Uuid::from_u128(0xa), Uuid::from_u128(0xb)],
                Some("Bearer caller-tok"),
            )
            .await
            .unwrap();

        assert_eq!(visible, HashSet::from([Uuid::from_u128(0xa)]));

        let (auth, req) = seen.lock().unwrap().take().unwrap();
        assert_eq!(
            auth.as_deref(),
            Some("Bearer caller-tok"),
            "identity resolves the caller from this header, so it must be forwarded verbatim"
        );
        assert_eq!(
            req,
            serde_json::json!({"person_ids": ["00000000-0000-0000-0000-00000000000a", "00000000-0000-0000-0000-00000000000b"]})
        );
    }

    #[tokio::test]
    async fn a_success_without_the_visible_field_is_an_error_not_an_empty_set() {
        let (url, _seen) =
            spawn_visible_persons(StatusCode::OK, serde_json::json!({"unexpected": []})).await;

        let err = IdentityClient::new(&url)
            .unwrap()
            .visible_person_ids(&[Uuid::from_u128(0xa)], Some("Bearer tok"))
            .await
            .unwrap_err();

        assert!(
            !err.to_string().is_empty(),
            "a contract mismatch must surface as a dependency error, not as `nothing visible`"
        );
    }

    #[tokio::test]
    async fn profiles_batch_forwards_auth_and_decodes_generic_attributes() {
        let person_id = Uuid::from_u128(0xa);
        let supervisor_id = Uuid::from_u128(0xb);
        let (url, seen) = spawn_profiles_batch(
            StatusCode::OK,
            serde_json::json!({
                "profiles": [{
                    "person_id": person_id,
                    "attributes": {
                        "display_name": "Example User",
                        "department": "Engineering"
                    },
                    "supervisor": {
                        "person_id": supervisor_id,
                        "attributes": {"display_name": "Example Manager"}
                    }
                }]
            }),
        )
        .await;

        let profiles = IdentityClient::new(&url)
            .unwrap()
            .profiles_batch(&[person_id], Some("Bearer caller-tok"))
            .await
            .unwrap();

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].person_id, person_id);
        assert_eq!(
            profiles[0]
                .attributes
                .get("display_name")
                .map(String::as_str),
            Some("Example User")
        );
        assert_eq!(
            profiles[0].supervisor.as_ref().map(|value| value.person_id),
            Some(supervisor_id)
        );

        let (auth, req) = seen.lock().unwrap().take().unwrap();
        assert_eq!(auth.as_deref(), Some("Bearer caller-tok"));
        assert_eq!(req, serde_json::json!({"person_ids": [person_id]}));
    }

    #[tokio::test]
    async fn malformed_profiles_batch_success_is_an_error() {
        let person_id = Uuid::from_u128(0xa);
        let (url, _) =
            spawn_profiles_batch(StatusCode::OK, serde_json::json!({"profiles": [{}]})).await;

        let error = IdentityClient::new(&url)
            .unwrap()
            .profiles_batch(&[person_id], Some("Bearer tok"))
            .await
            .unwrap_err();

        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn is_configured_and_base_url_normalization() {
        assert!(!IdentityClient::new("").unwrap().is_configured());
        let c = IdentityClient::new("http://identity:8082/").unwrap();
        assert!(c.is_configured());
        assert_eq!(c.base_url, "http://identity:8082");
    }
}

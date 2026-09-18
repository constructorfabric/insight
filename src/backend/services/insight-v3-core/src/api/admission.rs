use std::fmt;
use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderMap, HeaderValue};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use toolkit::api::{ParamLocation, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, resource_error};

use crate::config::{MAX_INGEST_TOKEN_BYTES, MIN_INGEST_TOKEN_BYTES};

pub(crate) const MAX_CONCURRENT_WRITES: usize = 64;
pub(crate) const MAX_REQUEST_BODY_BYTES: usize = 1_048_576;
pub(crate) const INGEST_TOKEN_HEADER: &str = "x-insight-token";

#[resource_error("gts.cf.insight.insight_v3_core.admission.v1~")]
struct AdmissionError;

#[derive(Debug, Clone)]
pub(crate) struct IngestAdmission {
    verifier: TokenVerifier,
    pub(crate) write_slots: Arc<tokio::sync::Semaphore>,
}

impl IngestAdmission {
    pub(crate) fn new(token: &SecretString) -> Self {
        Self {
            verifier: TokenVerifier::new(token),
            write_slots: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_WRITES)),
        }
    }
}

#[derive(Clone)]
pub(crate) struct TokenVerifier([u8; 32]);

impl TokenVerifier {
    pub(crate) fn new(token: &SecretString) -> Self {
        Self(Sha256::digest(token.expose_secret().as_bytes()).into())
    }

    pub(crate) fn authorizes(&self, headers: &HeaderMap) -> bool {
        if headers.get_all(INGEST_TOKEN_HEADER).iter().count() != 1 {
            return false;
        }
        let Some(token) = headers
            .get(INGEST_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
        else {
            return false;
        };
        if token.len() < MIN_INGEST_TOKEN_BYTES
            || token.len() > MAX_INGEST_TOKEN_BYTES
            || !token.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return false;
        }

        let actual: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        bool::from(actual.ct_eq(&self.0))
    }
}

impl fmt::Debug for TokenVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenVerifier(<redacted>)")
    }
}

pub(crate) fn instance_token_parameter() -> ParamSpec {
    ParamSpec {
        name: "X-Insight-Token".to_owned(),
        location: ParamLocation::Header,
        required: true,
        description: Some(
            "Static per-instance token configured on this service; it is not obtained from an authentication endpoint"
                .to_owned(),
        ),
        param_type: "string".to_owned(),
        array: false,
    }
}

pub(crate) fn protect(router: Router, admission: IngestAdmission) -> Router {
    router
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(middleware::from_fn_with_state(admission, authenticate))
}

async fn authenticate(
    State(admission): State<IngestAdmission>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    if !admission.verifier.authorizes(&headers) {
        return no_store(unauthenticated_response());
    }
    let Ok(permit) = admission.write_slots.try_acquire_owned() else {
        return no_store(capacity_error().into_response());
    };

    // INVARIANT: admission spans body polling, preparation, and ClickHouse I/O.
    let response = next.run(request).await;
    drop(permit);

    no_store(response)
}

fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));

    response
}

fn unauthenticated_response() -> Response {
    CanonicalError::unauthenticated()
        .with_reason("INVALID_INGEST_TOKEN")
        .create()
        .into_response()
}

fn capacity_error() -> CanonicalError {
    AdmissionError::resource_exhausted("Insight v3 Core write API is busy")
        .with_quota_violation("core_writes", "too many concurrent writes")
        .create()
}

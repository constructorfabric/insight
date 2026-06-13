//! `CanonicalJson<T>` — drop-in replacement for Axum's `Json<T>` that
//! converts every `JsonRejection` variant into the canonical RFC 9457
//! `application/problem+json` envelope mandated by DNA `REST/API.md §7`.
//!
//! ## Why this exists
//!
//! `axum::Json<T>` enforces `Content-Type: application/json` and handles
//! body deserialization — but its rejection responses use Axum's default
//! plain-text body, which fails the canonical-envelope contract every
//! analytics-api endpoint is supposed to honor. Hand-rolling
//! content-type checks and `serde_json::from_slice` at every handler
//! call site (a) duplicates code, (b) drifts between handlers, and
//! (c) reimplements something Axum already does correctly.
//!
//! Drop-in pattern:
//!
//! ```ignore
//! use crate::api::canonical_json::CanonicalJson;
//!
//! async fn my_handler(
//!     CanonicalJson(body): CanonicalJson<MyRequest>,
//! ) -> Response { ... }
//! ```
//!
//! ## What we map
//!
//! | `JsonRejection` variant | HTTP | GTS category |
//! |---|---|---|
//! | `MissingJsonContentType` | 415 | `unsupported_media_type` |
//! | `JsonSyntaxError`        | 400 | `invalid_argument` (field: `body`) |
//! | `JsonDataError`          | 400 | `invalid_argument` (field: `body`) — covers `deny_unknown_fields`, missing required fields, type mismatches |
//! | `BytesRejection`         | 400 | `invalid_argument` (field: `body`) |
//! | _other / future_         | 400 | `invalid_argument` (defensive default) |
//!
//! All bodies are RFC 9457 Problem Details with
//! `Content-Type: application/problem+json`.
//!
//! ## No `resource_type` on body-parse errors
//!
//! The §3.3 envelope's `context.resource_type` names the resource the
//! request was *targeting*. Body-parse failures fire BEFORE we know what
//! the caller wanted to do — we haven't parsed the body, so we don't
//! know which `(metric_id, threshold_id, …)` they were addressing. The
//! envelope omits the field rather than guessing; handlers that need to
//! attach a resource type for downstream errors continue to use the
//! `#[resource_error]` macro on their own builders.
//!
//! ## Cross-cutting future
//!
//! TODO: when admin-crud (#525) and any future POST/PUT handler ships,
//! the duplication this module prevents will be visible. If a third
//! cyberfabric service grows the same need, upstream `CanonicalJson` to
//! `cf-gears-toolkit-canonical-errors` (where `Problem` already lives) and
//! drop this module.

use axum::Json;
use axum::extract::{FromRequest, Request, rejection::JsonRejection};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde_json::json;
use toolkit_canonical_errors::Problem;

/// GTS type URI for the canonical `invalid_argument` category. Mirrors what
/// `CanonicalError::gts_type` would emit for the same variant; we build the
/// `Problem` directly so we don't need a resource-bound builder for the
/// body-parse path.
const INVALID_ARGUMENT_TYPE: &str =
    "gts://gts.cf.core.errors.err.v1~cf.core.err.invalid_argument.v1~";

/// GTS type URI for the catalog-local `unsupported_media_type` category.
/// `toolkit-canonical-errors` v0.7.3 doesn't expose a builder for this
/// category (it maps every variant to a fixed HTTP status from a closed
/// set, and 415 isn't in the set). When the upstream crate grows the
/// variant, swap this constant for the standard `CanonicalError::gts_type`
/// path.
const UNSUPPORTED_MEDIA_TYPE_TYPE: &str =
    "gts://gts.cf.core.errors.err.v1~cf.core.err.unsupported_media_type.v1~";

/// Drop-in replacement for `axum::Json<T>` whose `FromRequest` rejection
/// emits the canonical RFC 9457 envelope.
///
/// `T` must be `serde::de::DeserializeOwned + Send`.
#[derive(Debug, Clone)]
pub struct CanonicalJson<T>(pub T);

impl<S, T> FromRequest<S> for CanonicalJson<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rej) => Err(json_rejection_to_response(&rej)),
        }
    }
}

/// Map an Axum `JsonRejection` onto the canonical envelope. Exposed
/// `pub(crate)` so handlers that need fine-grained control over the body
/// extraction (e.g., conditional body presence, multipart fallback) can
/// reuse the mapper without going through the extractor wrapper.
pub(crate) fn json_rejection_to_response(rej: &JsonRejection) -> Response {
    match rej {
        JsonRejection::MissingJsonContentType(_) => unsupported_media_type_response(),
        JsonRejection::JsonSyntaxError(e) => {
            tracing::debug!(error = %e, "canonical_json: JSON syntax error");
            invalid_body_response("request body must be valid JSON")
        }
        JsonRejection::JsonDataError(e) => {
            // `JsonDataError` catches `deny_unknown_fields` (e.g., a smuggled
            // `tenant_id`), missing required fields, and type mismatches.
            // The serde-path on the error message points at the offending
            // field — kept in the server-side log; the wire detail is the
            // generic "did not match schema" so we don't leak internal
            // schema shape to untrusted callers.
            tracing::debug!(error = %e, "canonical_json: JSON data error");
            invalid_body_response("request body did not match the expected schema")
        }
        JsonRejection::BytesRejection(e) => {
            tracing::debug!(error = %e, "canonical_json: request body could not be read");
            invalid_body_response("request body could not be read")
        }
        // `JsonRejection` is `#[non_exhaustive]` — future variants degrade
        // to a generic 400 so a new Axum version doesn't surface a
        // non-canonical default rejection shape.
        _ => invalid_body_response("request body rejected by extractor"),
    }
}

/// 400 `invalid_argument` for body deserialization failures.
fn invalid_body_response(description: &'static str) -> Response {
    let problem = Problem {
        problem_type: INVALID_ARGUMENT_TYPE.to_owned(),
        title: "Invalid Argument".to_owned(),
        status: StatusCode::BAD_REQUEST.as_u16(),
        detail: description.to_owned(),
        instance: None,
        trace_id: None,
        context: json!({
            "field_violations": [
                { "field": "body", "description": description, "reason": "INVALID" }
            ]
        }),
    };
    problem.into_response()
}

/// 415 `unsupported_media_type` envelope. `Problem::into_response()` picks
/// the HTTP status from `problem.status`, serializes with
/// `application/problem+json` content-type, and falls back to a canonical
/// 500 envelope on serialization failure — same path every other
/// canonical error in the system uses.
fn unsupported_media_type_response() -> Response {
    Problem {
        problem_type: UNSUPPORTED_MEDIA_TYPE_TYPE.to_owned(),
        title: "Unsupported Media Type".to_owned(),
        status: StatusCode::UNSUPPORTED_MEDIA_TYPE.as_u16(),
        detail: "Content-Type: application/json required".to_owned(),
        instance: None,
        trace_id: None,
        context: json!({
            "precondition_violations": [
                {
                    "type": "content_type",
                    "subject": "Content-Type",
                    "description": "request must use Content-Type: application/json"
                }
            ]
        }),
    }
    .into_response()
}

#[cfg(test)]
mod tests {
    //! End-to-end coverage. Drives `CanonicalJson` through a real Axum
    //! router so the mapper is exercised against actual `JsonRejection`
    //! values produced by the production extractor — not synthetic ones.

    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, header::CONTENT_TYPE};
    use axum::routing::post;
    use serde::Deserialize;
    use tower::ServiceExt;

    use super::*;

    #[derive(Debug, Deserialize, Default)]
    #[serde(deny_unknown_fields)]
    struct TestReq {
        #[serde(default)]
        name: Option<String>,
    }

    async fn echo(CanonicalJson(req): CanonicalJson<TestReq>) -> Response {
        // Echo back the parsed name to prove the body round-tripped.
        axum::Json(serde_json::json!({ "name": req.name })).into_response()
    }

    fn router() -> Router {
        Router::new().route("/echo", post(echo))
    }

    async fn body_json(resp: Response) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let bytes = to_bytes(resp.into_body(), 64 * 1024).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    #[tokio::test]
    async fn missing_content_type_returns_canonical_415() -> Result<(), Box<dyn std::error::Error>>
    {
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .body(Body::from(r"{}"))?;
        let resp = router().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(
            resp.headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("application/problem+json")
        );
        let body = body_json(resp).await?;
        assert_eq!(body["status"], 415);
        assert_eq!(body["type"], UNSUPPORTED_MEDIA_TYPE_TYPE);
        assert_eq!(body["title"], "Unsupported Media Type");
        Ok(())
    }

    #[tokio::test]
    async fn wrong_content_type_returns_canonical_415() -> Result<(), Box<dyn std::error::Error>> {
        // Form-urlencoded → 415 (CSRF closure path).
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("name=bob"))?;
        let resp = router().oneshot(req).await?;
        assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let body = body_json(resp).await?;
        assert_eq!(body["status"], 415);
        Ok(())
    }

    #[tokio::test]
    async fn malformed_json_returns_canonical_400() -> Result<(), Box<dyn std::error::Error>> {
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(r"{not json"))?;
        let resp = router().oneshot(req).await?;
        assert_eq!(resp.status(), 400);
        let body = body_json(resp).await?;
        assert_eq!(body["type"], INVALID_ARGUMENT_TYPE);
        assert_eq!(body["status"], 400);
        let violations = body["context"]["field_violations"]
            .as_array()
            .ok_or("field_violations must be an array")?;
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0]["field"], "body");
        assert_eq!(violations[0]["reason"], "INVALID");
        Ok(())
    }

    #[tokio::test]
    async fn unknown_field_returns_canonical_400() -> Result<(), Box<dyn std::error::Error>> {
        // `deny_unknown_fields` on the request struct → `JsonDataError`
        // → canonical 400.
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"tenant_id":"sneaky"}"#))?;
        let resp = router().oneshot(req).await?;
        assert_eq!(resp.status(), 400);
        let body = body_json(resp).await?;
        assert_eq!(body["type"], INVALID_ARGUMENT_TYPE);
        Ok(())
    }

    #[tokio::test]
    async fn well_formed_body_passes_through() -> Result<(), Box<dyn std::error::Error>> {
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"name":"alice"}"#))?;
        let resp = router().oneshot(req).await?;
        assert_eq!(resp.status(), 200);
        let body = body_json(resp).await?;
        assert_eq!(body["name"], "alice");
        Ok(())
    }

    #[tokio::test]
    async fn charset_parameter_on_content_type_is_accepted()
    -> Result<(), Box<dyn std::error::Error>> {
        // `application/json; charset=utf-8` is what browsers and stdlib
        // HTTP clients commonly send — Axum's `Json<T>` accepts it.
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .header(CONTENT_TYPE, "application/json; charset=utf-8")
            .body(Body::from(r"{}"))?;
        let resp = router().oneshot(req).await?;
        assert_eq!(resp.status(), 200);
        Ok(())
    }

    #[tokio::test]
    async fn body_parse_envelope_has_no_resource_type() -> Result<(), Box<dyn std::error::Error>> {
        // Body-parse errors happen before we know which resource the
        // caller was addressing; the envelope intentionally omits
        // `context.resource_type` rather than guessing.
        let req = Request::builder()
            .method("POST")
            .uri("/echo")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"unknown":1}"#))?;
        let resp = router().oneshot(req).await?;
        let body = body_json(resp).await?;
        assert!(
            body["context"].get("resource_type").is_none(),
            "body-parse envelope MUST NOT carry resource_type; got: {body}"
        );
        Ok(())
    }
}

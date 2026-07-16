//! `CanonicalJson<T>` — drop-in replacement for Axum's `Json<T>` that converts
//! every `JsonRejection` into the canonical RFC 9457 `application/problem+json`
//! envelope. Ported from the analytics gear so body-parse failures on this
//! service emit the same error shape as every other endpoint (DNA REST/API §7).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde_json::json;
use toolkit_canonical_errors::Problem;

const INVALID_ARGUMENT_TYPE: &str =
    "gts://gts.cf.core.errors.err.v1~cf.core.err.invalid_argument.v1~";
const UNSUPPORTED_MEDIA_TYPE_TYPE: &str =
    "gts://gts.cf.core.errors.err.v1~cf.core.err.unsupported_media_type.v1~";

/// Drop-in replacement for `axum::Json<T>` whose rejection emits the canonical
/// RFC 9457 envelope. `T: DeserializeOwned + Send`.
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

fn json_rejection_to_response(rej: &JsonRejection) -> Response {
    match rej {
        JsonRejection::MissingJsonContentType(_) => unsupported_media_type_response(),
        JsonRejection::JsonSyntaxError(e) => {
            tracing::debug!(error = %e, "canonical_json: JSON syntax error");
            invalid_body_response("request body must be valid JSON")
        }
        JsonRejection::JsonDataError(e) => {
            tracing::debug!(error = %e, "canonical_json: JSON data error");
            invalid_body_response("request body did not match the expected schema")
        }
        JsonRejection::BytesRejection(e) => {
            tracing::debug!(error = %e, "canonical_json: request body could not be read");
            invalid_body_response("request body could not be read")
        }
        // `JsonRejection` is `#[non_exhaustive]`.
        _ => invalid_body_response("request body rejected by extractor"),
    }
}

/// 400 `invalid_argument` for body deserialization failures.
fn invalid_body_response(description: &'static str) -> Response {
    Problem {
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
    }
    .into_response()
}

/// 415 `unsupported_media_type` for a missing/wrong `Content-Type`.
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

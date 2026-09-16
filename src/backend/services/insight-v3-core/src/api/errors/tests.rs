use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use sea_orm::{ConnAcquireErr, DbErr};

use super::ApiErrors;
use crate::api::definitions::DefinitionApiError;
use crate::definitions::{DefinitionError, DefinitionStoreError};

const BODY_LIMIT_BYTES: usize = 64 * 1024;

async fn said(error: toolkit_canonical_errors::CanonicalError) -> (StatusCode, String) {
    let response = error.into_response();
    let status = response.status();
    let body = to_bytes(response.into_body(), BODY_LIMIT_BYTES)
        .await
        .unwrap_or_else(|error| panic!("the body must be readable: {error}"));

    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn a_store_too_busy_to_answer_is_a_wait_the_caller_may_retry() {
    let busy = DefinitionStoreError::Database(DbErr::ConnectionAcquire(ConnAcquireErr::Timeout));

    let (status, _) = said(DefinitionApiError::definition_store_error(busy)).await;

    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
}

#[tokio::test]
async fn a_store_that_failed_says_nothing_about_the_database() {
    let broken = DefinitionStoreError::Database(DbErr::Custom(
        "column `body` missing from `metrics`".to_owned(),
    ));

    let (status, body) = said(DefinitionApiError::definition_store_error(broken)).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.contains("metrics"), "{body}");
    assert!(!body.contains("column"), "{body}");
}

#[tokio::test]
async fn a_body_that_cannot_be_written_says_nothing_about_the_store() {
    let Err(unreadable) = serde_json::from_str::<serde_json::Value>("{") else {
        panic!("the fragment must not parse");
    };

    let (status, body) = said(DefinitionApiError::definition_store_error(
        DefinitionStoreError::Json(unreadable),
    ))
    .await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!body.contains("EOF"), "{body}");
}

#[tokio::test]
async fn a_name_someone_holds_is_a_conflict_that_names_it() {
    let taken = DefinitionStoreError::NameTaken("by_actor".to_owned());

    let (status, body) = said(DefinitionApiError::definition_store_error(taken)).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.contains("by_actor"), "{body}");
}

#[tokio::test]
async fn a_name_the_store_cannot_hold_is_the_callers_to_fix() {
    let (status, _) = said(DefinitionApiError::definition_error(DefinitionError::Name)).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

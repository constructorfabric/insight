pub mod config;
pub mod infra;
pub mod migration;
pub mod people;

pub use crate::people::PeopleStore;

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;

pub fn build_router(store: PeopleStore) -> axum::Router {
    let state = Arc::new(store);
    axum::Router::new()
        .route("/v1/persons/{email}", axum::routing::get(get_person))
        .route("/health", axum::routing::get(health))
        .route("/healthz", axum::routing::get(healthz))
        .with_state(state)
}

async fn get_person(
    State(store): State<Arc<PeopleStore>>,
    axum::extract::Path(email): axum::extract::Path<String>,
) -> impl IntoResponse {
    match store.get_by_email(&email) {
        Some(person) => Json(serde_json::json!(person)).into_response(),
        None => (
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "type": "urn:insight:error:person_not_found",
                "title": "Not Found",
                "status": 404,
                "detail": format!("person with email '{email}' not found"),
            })),
        )
            .into_response(),
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "healthy" }))
}

async fn healthz() -> &'static str {
    "ok"
}

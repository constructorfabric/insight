//! HTTP API layer — shared state, route table, auth.
//!
//! Route surface (DESIGN.md §4): everything under `/v1` sits behind the
//! static bearer middleware ([`auth`]). Health is served by the host itself
//! (`GET /healthz` comes with the `api-gateway` system gear — registering our
//! own panics with an overlapping-route error). The data endpoints land in
//! later phases; phase 1 mounts the skeleton so the host wiring and auth are
//! real and tested end to end.

pub mod auth;

use std::sync::Arc;

use axum::routing::get;
use axum::{Extension, Router};

use crate::config::GearConfig;
use crate::engine::store::RepoStore;

/// Shared application state, injected into handlers via `Extension`.
pub struct AppState {
    /// The repository cache (clone/fetch-if-stale engine).
    pub store: Arc<RepoStore>,
    /// Gear config (budgets, staleness window, data dir).
    pub config: GearConfig,
}

/// Mount the git-cli-proxy routes onto the host's router.
///
/// Builds our endpoints on a fresh sub-router (so the `AppState` extension and
/// the bearer middleware scope to our routes, not the host's own paths), then
/// merges it into the host router.
pub fn register_routes(host_router: Router, state: Arc<AppState>) -> Router {
    let bearer = auth::ProxyAuth::new(state.config.proxy_token.clone());

    let v1 = Router::new()
        // Data endpoints (commits / file-changes / branches) mount here in
        // later phases.
        .route("/v1/ping", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(
            bearer,
            auth::require_bearer,
        ))
        .layer(Extension(state));

    host_router.merge(v1)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    fn state() -> Arc<AppState> {
        let data_dir =
            std::env::temp_dir().join(format!("git-cli-proxy-api-tests-{}", std::process::id()));
        let store = match RepoStore::new(&data_dir, 2) {
            Ok(s) => s,
            Err(e) => panic!("store init: {e}"),
        };
        Arc::new(AppState {
            store: Arc::new(store),
            config: GearConfig {
                data_dir: data_dir.display().to_string(),
                disk_budget_bytes: 1_000_000,
                max_repo_bytes: 500_000,
                default_max_staleness_seconds: 300,
                heavy_ops_concurrency: 2,
                proxy_token: "t0ken".to_owned(),
            },
        })
    }

    fn app() -> Router {
        register_routes(Router::new(), state())
    }

    async fn get_status(uri: &str, bearer: Option<&str>) -> StatusCode {
        let mut builder = Request::builder().uri(uri);
        if let Some(token) = bearer {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let request = builder.body(Body::empty()).unwrap_or_default();
        match app().oneshot(request).await {
            Ok(response) => response.status(),
            Err(never) => match never {},
        }
    }

    #[tokio::test]
    async fn v1_requires_the_token() {
        assert_eq!(get_status("/v1/ping", None).await, StatusCode::UNAUTHORIZED);
        assert_eq!(
            get_status("/v1/ping", Some("wrong")).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(get_status("/v1/ping", Some("t0ken")).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn healthz_is_left_to_the_host() {
        // INVARIANT: the api-gateway host owns GET /healthz; registering our
        // own panics at boot with an overlapping-route error. In isolation our
        // router has no such route — the bearer middleware wraps the fallback,
        // so an unmatched path answers 401, never our handler. In the real
        // process the host's /healthz route wins the merge and stays public
        // (proven by tests/boot.rs).
        assert_eq!(get_status("/healthz", None).await, StatusCode::UNAUTHORIZED);
    }
}

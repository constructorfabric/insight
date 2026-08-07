pub mod auth;
pub mod data;
pub mod error;
pub mod request;

use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use crate::config::GearConfig;
use crate::engine::store::RepoStore;

pub struct AppState {
    pub store: Arc<RepoStore>,
    pub config: GearConfig,
}

/// Mount the git-cli-proxy routes onto the host's router.
///
/// Our routes live on a fresh sub-router so the bearer middleware and state
/// scope to `/v1` only; `/healthz` belongs to the `api-gateway` host gear.
pub fn register_routes(host_router: Router, state: Arc<AppState>) -> Router {
    let bearer = auth::ProxyAuth::new(state.config.proxy_token.clone());

    let v1 = Router::new()
        .route("/v1/commits", get(data::list_commits))
        .route("/v1/file-changes", get(data::list_file_changes))
        .route("/v1/branches", get(data::list_branches))
        .layer(axum::middleware::from_fn_with_state(
            bearer,
            auth::require_bearer,
        ))
        .with_state(state);

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
                ca_cert_path: String::new(),
            },
        })
    }

    async fn status_of(uri: &str, bearer: Option<&str>, identity: bool) -> StatusCode {
        let mut builder = Request::builder().uri(uri);
        if let Some(token) = bearer {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        if identity {
            builder = builder
                .header("x-tenant-id", "t")
                .header("x-source-id", "s")
                .header("x-git-username", "u")
                .header("x-git-token", "p");
        }
        let request = builder.body(Body::empty()).unwrap_or_default();
        match register_routes(Router::new(), state())
            .oneshot(request)
            .await
        {
            Ok(response) => response.status(),
            Err(never) => match never {},
        }
    }

    #[tokio::test]
    async fn every_data_route_requires_the_proxy_token() {
        for uri in [
            "/v1/commits?repo=x",
            "/v1/file-changes?repo=x",
            "/v1/branches?repo=x",
        ] {
            assert_eq!(
                status_of(uri, None, true).await,
                StatusCode::UNAUTHORIZED,
                "unauthenticated {uri} must be rejected"
            );
            assert_eq!(
                status_of(uri, Some("wrong"), true).await,
                StatusCode::UNAUTHORIZED,
                "wrong token on {uri} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn identity_headers_are_mandatory_on_data_routes() {
        for uri in [
            "/v1/commits?repo=x",
            "/v1/file-changes?repo=x",
            "/v1/branches?repo=x",
        ] {
            assert_eq!(
                status_of(uri, Some("t0ken"), false).await,
                StatusCode::BAD_REQUEST,
                "{uri} without identity headers must be a bad request"
            );
        }
    }

    #[tokio::test]
    async fn missing_repo_parameter_is_a_bad_request() {
        assert_eq!(
            status_of("/v1/commits", Some("t0ken"), true).await,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn malformed_page_token_is_a_bad_request() {
        assert_eq!(
            status_of("/v1/commits?repo=x&page_token=!!!", Some("t0ken"), true).await,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn healthz_is_left_to_the_host() {
        // INVARIANT: the api-gateway host owns GET /healthz; registering our
        // own panics at boot with an overlapping-route error. In isolation the
        // path falls through our bearer layer, so it answers 401 here.
        assert_eq!(
            status_of("/healthz", None, false).await,
            StatusCode::UNAUTHORIZED
        );
    }
}

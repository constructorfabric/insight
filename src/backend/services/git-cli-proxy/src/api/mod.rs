pub mod auth;
pub mod data;
pub mod error;
pub mod request;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use axum::{Extension, Router};
use toolkit::api::{OpenApiInfo, OpenApiRegistry, OpenApiRegistryImpl, OperationBuilder};

use crate::config::GearConfig;
use crate::engine::metrics;
use crate::engine::store::RepoStore;
use crate::engine::url::CloneUrlPolicy;

pub struct AppState {
    pub store: Arc<RepoStore>,
    pub config: GearConfig,
}

impl AppState {
    #[must_use]
    pub fn clone_url_policy(&self) -> CloneUrlPolicy<'_> {
        CloneUrlPolicy {
            allow_file: self.config.allow_file_repos,
            allowed_hosts: &self.config.allowed_repo_hosts,
        }
    }
}

/// Mount the git-cli-proxy routes onto the host's router.
///
/// Our routes live on a fresh sub-router so the bearer middleware and state
/// scope to `/v1` only; `/healthz` is mounted by the host's base router
/// (`gear.rs`) and stays public for probes.
pub fn register_routes(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let bearer = auth::ProxyAuth::new(state.config.proxy_token.clone());

    let v1 = build_operations(Router::new(), openapi)
        // Innermost, so only handler work is on the clock — auth and logging
        // stay outside the budget.
        .layer(axum::middleware::from_fn(bound_handler))
        .layer(axum::middleware::from_fn_with_state(
            bearer,
            auth::require_bearer,
        ))
        .layer(Extension(state))
        // Outside the bearer layer, so a rejected request is timed too — an
        // operator watching a token rotation needs exactly those.
        .layer(axum::middleware::from_fn(observe))
        .layer(insight_http_metrics::ServerMetricsLayer::new(
            "git-cli-proxy",
        ));

    host_router.merge(v1)
}

/// Every wait a handler can make is individually bounded (git budgets, the
/// inline preparation wait, the read-lock wait), but a hold that is never
/// released — a leaked guard, a wedged permit holder — turns the NEXT
/// request's wait into forever: the connector has no client timeout, so one
/// such request froze a multi-day sync invisibly. This ceiling converts that
/// class into a bounded, retryable answer. It must stay above every legal
/// inline duration; the longest is a page-serve blob prefetch (10 minutes).
const HANDLER_BUDGET: Duration = Duration::from_mins(15);

/// Answer 503 when a handler outlives [`HANDLER_BUDGET`].
///
/// 503, not 408: the connectors' declarative handlers already RETRY 503 (and
/// would FAIL on an unlisted status). The dropped future releases whatever
/// the request itself held via RAII; whatever it was WAITING on stays wedged
/// and is the defect to find in the log this leaves behind.
async fn bound_handler(request: Request, next: Next) -> Response {
    answer_within(HANDLER_BUDGET, request, next).await
}

async fn answer_within(budget: Duration, request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    if let Ok(response) = tokio::time::timeout(budget, next.run(request)).await {
        return response;
    }
    metrics::record_handler_timeout();
    tracing::error!(%method, %path, budget_s = budget.as_secs(),
        "handler exceeded its budget; answering 503");
    error::handler_timed_out()
}

/// Record §4.3's per-endpoint histograms for every request that reached a
/// route, and log the request line.
///
/// The metric label is the matched ROUTE, not the request path: a path would
/// make the label set unbounded. A request that matched nothing has no
/// endpoint to attribute and is not recorded.
async fn observe(request: Request, next: Next) -> Response {
    let endpoint = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_owned());
    let method = request.method().clone();
    let query = request.uri().query().unwrap_or_default().to_owned();

    let started = Instant::now();
    let response = next.run(request).await;
    let elapsed = started.elapsed();

    tracing::info!(
        %method,
        endpoint = endpoint.as_deref().unwrap_or("<unmatched>"),
        %query,
        status = response.status().as_u16(),
        duration_ms = elapsed.as_millis(),
        "request"
    );

    if let Some(endpoint) = endpoint {
        let bytes = response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        metrics::record_request(
            &endpoint,
            response.status().as_u16(),
            elapsed.as_secs_f64(),
            bytes,
        );
    }
    response
}

/// Title/version/description of the emitted document.
fn openapi_info() -> OpenApiInfo {
    OpenApiInfo {
        title: "Insight Git CLI Proxy API".to_owned(),
        version: "1.0.0".to_owned(),
        description: Some(
            "Commit-level git data served from a local clone cache, so the \
             nocode git connectors do not spend one vendor API call per \
             commit. Cluster-internal and consumed only by Airbyte connector \
             jobs; it is not mounted behind the API Gateway."
                .to_owned(),
        ),
        servers: Vec::new(),
    }
}

/// Build the document offline — no `AppState`, no listener. Backs the
/// `git-cli-proxy openapi` subcommand and its drift gate, reusing the exact
/// route table the live gear serves so the two cannot diverge.
///
/// # Errors
///
/// Returns an error if the registry cannot assemble the document.
pub fn openapi_document() -> anyhow::Result<utoipa::openapi::OpenApi> {
    let openapi = OpenApiRegistryImpl::new();
    let _ = build_operations(Router::new(), &openapi);

    openapi
        .build_openapi(&openapi_info())
        .map_err(|e| anyhow::anyhow!("failed to build git-cli-proxy OpenAPI document: {e}"))
}

/// Declare each operation via the toolkit `OperationBuilder` (route + OpenAPI
/// spec + auth/error metadata in one place).
///
/// The error set is declared explicitly rather than with `standard_errors`:
/// this document IS the connectors' contract, and `standard_errors` would
/// advertise a `403` the service never emits.
#[expect(
    clippy::too_many_lines,
    reason = "one flat block per route, as in the sibling services"
)]
fn build_operations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/v1/commits")
        .operation_id("git_cli_proxy.commits.list")
        .summary("Commits reachable from any branch, ascending by (committed_date, sha)")
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "repo",
            true,
            "Clone URL of the repository (http/https)",
            "string",
        )
        .query_param_typed("since", false, "Lower bound on committed_date", "string")
        .query_param_typed(
            "sha",
            false,
            "Comma-separated commit ids or hex prefixes, 7 to 64 characters. A prefix selects every commit it matches.",
            "string",
        )
        .query_param_typed("page_size", false, "1..=1000, default 1000; larger values are clamped", "integer")
        .query_param_typed(
            "page_token",
            false,
            "Cursor from a previous page; pins the snapshot and never fetches",
            "string",
        )
        .json_response_with_schema::<data::CommitsPage>(
            openapi,
            StatusCode::OK,
            "One page of commits",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_429(openapi)
        .error_500(openapi)
        .problem_response(
            openapi,
            StatusCode::PAYLOAD_TOO_LARGE,
            "Repository exceeds the configured per-repository size cap",
        )
        .handler(data::list_commits)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/file-changes")
        .operation_id("git_cli_proxy.file_changes.list")
        .summary("Per-file changes of non-merge commits, with optional patch text")
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "repo",
            true,
            "Clone URL of the repository (http/https)",
            "string",
        )
        .query_param_typed("since", false, "Lower bound on committed_date", "string")
        .query_param_typed(
            "sha",
            false,
            "Comma-separated commit ids or hex prefixes, 7 to 64 characters. A prefix selects every commit it matches.",
            "string",
        )
        .query_param_typed(
            "page_size",
            false,
            "Commits per page, 1..=1000, default 1000; larger values are clamped. Rows fan \
             out per changed file, so a page is additionally capped by row count and total \
             patch bytes.",
            "integer",
        )
        .query_param_typed(
            "page_token",
            false,
            "Cursor from a previous page; pins the snapshot and never fetches",
            "string",
        )
        .query_param_typed(
            "include_patch",
            false,
            "Attach unified diff text (default true)",
            "boolean",
        )
        .query_param_typed(
            "max_patch_bytes",
            false,
            "Per-file patch budget; longer diffs are cut and flagged. Default 1 MiB, max 8 MiB.",
            "integer",
        )
        .json_response_with_schema::<data::FileChangesPage>(
            openapi,
            StatusCode::OK,
            "One page of file changes",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_429(openapi)
        .error_500(openapi)
        .problem_response(
            openapi,
            StatusCode::PAYLOAD_TOO_LARGE,
            "Repository exceeds the configured per-repository size cap",
        )
        .handler(data::list_file_changes)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/authors")
        .operation_id("git_cli_proxy.authors.list")
        .summary("Distinct commit authors, ascending by e-mail")
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "repo",
            true,
            "Clone URL of the repository (http/https)",
            "string",
        )
        .query_param_typed(
            "since",
            false,
            "Lower bound on committed_date; an author whose commits all predate it is omitted",
            "string",
        )
        .query_param_typed(
            "page_size",
            false,
            "1..=1000, default 1000; larger values are clamped",
            "integer",
        )
        .query_param_typed(
            "page_token",
            false,
            "Cursor from a previous page; pins the snapshot and never fetches",
            "string",
        )
        .json_response_with_schema::<data::AuthorsPage>(
            openapi,
            StatusCode::OK,
            "One page of authors",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_429(openapi)
        .error_500(openapi)
        .problem_response(
            openapi,
            StatusCode::PAYLOAD_TOO_LARGE,
            "Repository exceeds the configured per-repository size cap",
        )
        .handler(data::list_authors)
        .register(router, openapi);

    OperationBuilder::get("/v1/branches")
        .operation_id("git_cli_proxy.branches.list")
        .summary("Branch heads, ascending by name")
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "repo",
            true,
            "Clone URL of the repository (http/https)",
            "string",
        )
        .query_param_typed(
            "page_size",
            false,
            "1..=1000, default 1000; larger values are clamped",
            "integer",
        )
        .query_param_typed(
            "page_token",
            false,
            "Cursor from a previous page; pins the snapshot and never fetches",
            "string",
        )
        .json_response_with_schema::<data::BranchesPage>(
            openapi,
            StatusCode::OK,
            "One page of branches",
        )
        .error_400(openapi)
        .error_401(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_429(openapi)
        .error_500(openapi)
        .problem_response(
            openapi,
            StatusCode::PAYLOAD_TOO_LARGE,
            "Repository exceeds the configured per-repository size cap",
        )
        .handler(data::list_branches)
        .register(router, openapi)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    /// A store per caller, never a shared directory: constructing a store
    /// clears its `tmp/`, so two tests sharing one data dir race each other.
    fn state() -> Arc<AppState> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // nosemgrep: rust.lang.security.temp-dir.temp-dir -- test fixture; names carry pid/thread/counter and hold no secrets
        let data_dir = std::env::temp_dir().join(format!(
            "git-cli-proxy-api-tests-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let store = match RepoStore::new(&data_dir, 2) {
            Ok(s) => s,
            Err(e) => panic!("store init: {e}"),
        };
        Arc::new(AppState {
            store: Arc::new(store),
            config: GearConfig {
                bind_addr: "127.0.0.1:0".to_owned(),
                allowed_repo_hosts: Vec::new(),
                data_dir: data_dir.display().to_string(),
                disk_budget_bytes: 1_000_000,
                max_repo_bytes: 500_000,
                default_max_staleness_seconds: 300,
                heavy_ops_concurrency: 2,
                proxy_token: "t0ken".to_owned(),
                ca_cert_path: String::new(),
                allow_file_repos: true,
            },
        })
    }

    async fn response_for(
        uri: &str,
        bearer: Option<&str>,
        identity: bool,
    ) -> axum::response::Response {
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
        match register_routes(Router::new(), &OpenApiRegistryImpl::new(), state())
            .oneshot(request)
            .await
        {
            Ok(response) => response,
            Err(never) => match never {},
        }
    }

    async fn status_of(uri: &str, bearer: Option<&str>, identity: bool) -> StatusCode {
        response_for(uri, bearer, identity).await.status()
    }

    #[test]
    fn build_operations_registers_the_full_table_without_state() {
        // Catches overlapping routes and bad builder state with no AppState,
        // which is exactly what the offline `openapi` subcommand does.
        let openapi = OpenApiRegistryImpl::new();
        let _ = build_operations(Router::new(), &openapi);
    }

    #[test]
    fn openapi_document_lists_every_operation() {
        let Ok(document) = openapi_document() else {
            panic!("the document must build offline")
        };
        let ids: Vec<String> = document
            .paths
            .paths
            .values()
            .filter_map(|item| item.get.as_ref())
            .filter_map(|op| op.operation_id.clone())
            .collect();

        for expected in [
            "git_cli_proxy.commits.list",
            "git_cli_proxy.file_changes.list",
            "git_cli_proxy.branches.list",
        ] {
            assert!(
                ids.iter().any(|id| id == expected),
                "missing {expected} in {ids:?}"
            );
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
    async fn every_rejection_carries_the_problem_envelope() {
        // §4.4 promises one envelope for every failure. Two paths used to
        // escape it: the bearer layer answered with an empty body, and axum
        // rejected a bad query string as text/plain before any handler ran.
        let cases: Vec<(&str, &str, Option<&str>, bool)> = vec![
            ("no bearer", "/v1/commits?repo=x", None, true),
            ("wrong bearer", "/v1/commits?repo=x", Some("wrong"), true),
            ("missing repo", "/v1/commits", Some("t0ken"), true),
            (
                "non-numeric page_size",
                "/v1/commits?repo=x&page_size=abc",
                Some("t0ken"),
                true,
            ),
            (
                "non-boolean include_patch",
                "/v1/file-changes?repo=x&include_patch=yes",
                Some("t0ken"),
                true,
            ),
            (
                "missing identity headers",
                "/v1/branches?repo=x",
                Some("t0ken"),
                false,
            ),
        ];

        for (name, uri, bearer, identity) in cases {
            let response = response_for(uri, bearer, identity).await;
            assert!(
                response.status().is_client_error(),
                "case {name}: expected a client error, got {}",
                response.status()
            );
            let content_type = response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            assert!(
                content_type.starts_with("application/problem+json"),
                "case {name}: expected problem+json, got {content_type:?}"
            );

            let Ok(body) = axum::body::to_bytes(response.into_body(), usize::MAX).await else {
                panic!("case {name}: body must be readable")
            };
            assert!(
                !body.is_empty(),
                "case {name}: the envelope must have a body"
            );
        }
    }

    #[tokio::test]
    async fn healthz_is_left_to_the_host() {
        // INVARIANT: the host's base router owns GET /healthz; registering our
        // own panics at boot with an overlapping-route error. In isolation the
        // path falls through our bearer layer, so it answers 401 here.
        assert_eq!(
            status_of("/healthz", None, false).await,
            StatusCode::UNAUTHORIZED
        );
    }

    /// The pending handler never resolves, so only the budget can produce an
    /// answer — and it must be the 503 the connectors RETRY. A tiny budget
    /// stands in for the real one; `bound_handler` differs only by constant.
    #[tokio::test]
    async fn a_handler_that_never_answers_is_cut_off_with_503() {
        let app = axum::Router::new()
            .route(
                "/wedged",
                axum::routing::get(std::future::pending::<axum::response::Response>),
            )
            .layer(axum::middleware::from_fn(
                |request: axum::extract::Request, next: Next| {
                    answer_within(Duration::from_millis(20), request, next)
                },
            ));

        let request = match Request::builder().uri("/wedged").body(Body::empty()) {
            Ok(r) => r,
            Err(e) => panic!("build request: {e}"),
        };
        let response = match app.oneshot(request).await {
            Ok(r) => r,
            Err(e) => panic!("drive router: {e}"),
        };
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}

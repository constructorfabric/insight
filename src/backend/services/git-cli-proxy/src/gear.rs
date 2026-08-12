//! The git-cli-proxy gear: validates config fail-fast, builds the repo store,
//! owns the HTTP host, and serves `/v1` behind the service's own bearer token
//! (Airbyte-only consumer, never behind the platform gateway).
//!
//! INVARIANT: this service is its own `rest_host`, not a guest of the
//! `api-gateway` gear, because that gear wraps every route in a hardcoded 30s
//! `tower` timeout with no config knob — and a page read legitimately runs for
//! minutes fetching blobs from origin ([`crate::engine::runner::Timeouts`]),
//! so under that host a working request returned `504` and lost its work.
//! Re-adding the gear silently reinstates the ceiling.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use axum::Router;
use axum::routing::get;
use tokio_util::sync::CancellationToken;
use toolkit::api::{OpenApiRegistry, OpenApiRegistryImpl};
use toolkit::contracts::ApiGatewayCapability;
use toolkit::lifecycle::ReadySignal;
use toolkit::{Gear, GearCtx, RestApiCapability};
use tower_http::catch_panic::CatchPanicLayer;

use crate::api::AppState;
use crate::config::GearConfig;
use crate::engine::disk::Budget;
use crate::engine::store::RepoStore;

/// Config key is the gear name `git-cli-proxy`; env overrides are
/// `APP__gears__git_cli_proxy__config__*`.
#[derive(Default)]
#[toolkit::gear(
    name = "git-cli-proxy",
    capabilities = [rest_host, rest, stateful],
    lifecycle(entry = "serve", stop_timeout = "30s", await_ready)
)]
pub struct GitCliProxyGear {
    state: OnceLock<Arc<AppState>>,
    openapi: OpenApiRegistryImpl,
    router: OnceLock<Router>,
}

impl GitCliProxyGear {
    fn state(&self) -> anyhow::Result<Arc<AppState>> {
        self.state
            .get()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("git-cli-proxy gear not initialized"))
    }

    /// Lifecycle entry: bind, report ready, serve until cancelled.
    async fn serve(
        self: Arc<Self>,
        cancel: CancellationToken,
        ready: ReadySignal,
    ) -> anyhow::Result<()> {
        let bind_addr = self.state()?.config.bind_addr.clone();
        let router = self
            .router
            .get()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("the router was never finalized"))?;

        let listener = tokio::net::TcpListener::bind(bind_addr.as_str())
            .await
            .map_err(|e| anyhow::anyhow!("cannot bind {bind_addr}: {e}"))?;
        tracing::info!(%bind_addr, "git-cli-proxy listening");
        ready.notify();

        axum::serve(listener, router)
            .with_graceful_shutdown(async move { cancel.cancelled().await })
            .await
            .map_err(|e| anyhow::anyhow!("the HTTP server stopped: {e}"))
    }
}

#[async_trait]
impl Gear for GitCliProxyGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let config: GearConfig = ctx.config()?;
        config.validate()?;
        tracing::info!(config = ?config, "starting git-cli-proxy gear");

        let store = RepoStore::open_cache(
            Path::new(&config.data_dir),
            config.heavy_ops_concurrency,
            Some(config.ca_cert_path.clone()),
            Budget {
                total_bytes: config.disk_budget_bytes,
            },
            config.max_repo_bytes,
        )?;

        let store = Arc::new(store);
        store.require_purge_support().await.map_err(|e| {
            anyhow::anyhow!(
                "the installed git cannot run the blob purge \
                 (`repack --filter --filter-to`, git 2.43+): {e}"
            )
        })?;

        // §4.3: the gauges observe a cached snapshot the store refreshes on
        // every admission check, so the collector's callback does no I/O.
        crate::engine::metrics::register_disk_gauges(store.gauges());

        let state = AppState { store, config };
        self.state
            .set(Arc::new(state))
            .map_err(|_| anyhow::anyhow!("{} gear already initialized", Self::MODULE_NAME))?;
        Ok(())
    }
}

impl RestApiCapability for GitCliProxyGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<Router> {
        Ok(crate::api::register_routes(router, openapi, self.state()?))
    }
}

impl ApiGatewayCapability for GitCliProxyGear {
    fn rest_prepare(&self, _ctx: &GearCtx, router: Router) -> anyhow::Result<Router> {
        Ok(router.route("/healthz", get(|| async { "ok" })))
    }

    fn rest_finalize(&self, _ctx: &GearCtx, router: Router) -> anyhow::Result<Router> {
        // Panics are caught below the canonical layer, so the 500 it produces
        // is rendered like every other failure.
        let router = router
            .layer(CatchPanicLayer::new())
            .layer(axum::middleware::from_fn(
                toolkit::api::canonical_error_middleware,
            ));

        self.router
            .set(router.clone())
            .map_err(|_| anyhow::anyhow!("the router was finalized twice"))?;
        Ok(router)
    }

    fn as_registry(&self) -> &dyn OpenApiRegistry {
        &self.openapi
    }
}

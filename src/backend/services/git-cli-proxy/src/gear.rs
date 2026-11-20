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

        crate::engine::metrics::register_disk_gauges(store.gauges());

        let serves = Arc::new(tokio::sync::Semaphore::new(config.serve_concurrency));
        let state = AppState {
            store,
            config,
            serves,
        };
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
    fn rest_prepare(
        &self,
        _ctx: &GearCtx,
        router: Router,
        _hc_registry: Arc<toolkit::RestHealthcheckRegistry>,
    ) -> anyhow::Result<Router> {
        Ok(prepared_router(router))
    }

    fn rest_finalize(
        &self,
        _ctx: &GearCtx,
        router: Router,
        _hc_registry: Arc<toolkit::RestHealthcheckRegistry>,
    ) -> anyhow::Result<Router> {
        finalize_router(router, &self.router)
    }

    fn as_registry(&self) -> &dyn OpenApiRegistry {
        &self.openapi
    }
}

fn prepared_router(router: Router) -> Router {
    router.route("/healthz", get(|| async { "ok" }))
}

fn finalize_router(router: Router, slot: &OnceLock<Router>) -> anyhow::Result<Router> {
    let router = router
        .layer(CatchPanicLayer::new())
        .layer(axum::middleware::from_fn(
            toolkit::api::canonical_error_middleware,
        ));

    slot.set(router.clone())
        .map_err(|_| anyhow::anyhow!("the router was finalized twice"))?;
    Ok(router)
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    type R = Result<(), Box<dyn std::error::Error>>;

    #[tokio::test]
    async fn the_prepared_router_serves_healthz() -> R {
        let router = prepared_router(Router::new());

        let response = router
            .oneshot(Request::get("/healthz").body(Body::empty())?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        Ok(())
    }

    #[tokio::test]
    async fn a_finalized_router_turns_a_panic_into_a_500() -> R {
        let slot = OnceLock::new();
        let panicking = Router::new().route(
            "/boom",
            get(|| async {
                panic!("handler panic");
                #[expect(unreachable_code)]
                ""
            }),
        );

        let router = finalize_router(panicking, &slot)?;
        let response = router
            .oneshot(Request::get("/boom").body(Body::empty())?)
            .await?;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        Ok(())
    }

    #[test]
    fn a_second_finalize_is_rejected() {
        let slot = OnceLock::new();

        let first = finalize_router(Router::new(), &slot);

        assert!(first.is_ok());
        assert!(finalize_router(Router::new(), &slot).is_err());
    }
}

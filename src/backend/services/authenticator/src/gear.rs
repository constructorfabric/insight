//! The `authenticator` gear (DESIGN §3.10).
//!
//! `init` loads the §4.1 config, opens the Redis session store, loads the ES256
//! signing keys, builds the OIDC + person-resolver clients, and registers the
//! SDK `LocalClient` in the `ClientHub`. `rest` mounts the endpoints via the
//! operation builder; `stateful` owns the background workers — a no-op stub for
//! step 04 (the IdP refresher + janitor land in steps 06/10).

use std::path::Path;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use authenticator_sdk::AuthenticatorClientV1;
use tokio_util::sync::CancellationToken;
use toolkit::api::OpenApiRegistry;
use toolkit::{Gear, GearCtx, RestApiCapability, RunnableCapability};

use crate::api::{self, AppState};
use crate::config::AuthenticatorConfig;
use crate::identity::{IdentityPersonResolver, PersonResolver};
use crate::jwt::KeyStore;
use crate::local_client::LocalClient;
use crate::oidc::OidcClient;
use crate::service_token::{self, ServiceRegistry};
use crate::session::SessionManager;

/// The authenticator gear. Capabilities: `rest` (the API surface) and
/// `stateful` (background workers — stubbed for step 04).
#[toolkit::gear(
    name = "authenticator",
    deps = ["types-registry"],
    capabilities = [rest, stateful]
)]
pub struct AuthenticatorGear {
    state: OnceLock<Arc<AppState>>,
}

impl Default for AuthenticatorGear {
    fn default() -> Self {
        Self {
            state: OnceLock::new(),
        }
    }
}

#[async_trait]
impl Gear for AuthenticatorGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        let cfg: AuthenticatorConfig = ctx.config_or_default()?;
        cfg.validate()?;
        tracing::info!(
            gateway_issuer = %cfg.gateway_issuer,
            idp_issuer = %cfg.idp.issuer_url,
            "starting authenticator gear"
        );

        // ES256 signing keys are always loaded from the mounted `signing_keys_path`
        // (fail closed if absent). Real deployments mount a Secret; dev mounts a
        // directory whose key dev-compose.sh generates (gitignored, not baked).
        let keystore = Arc::new(KeyStore::load(Path::new(&cfg.signing_keys_path))?);

        // Redis session store (fail closed — no in-process fallback). Probe at
        // boot so a missing Redis surfaces here rather than on first request.
        let sessions = SessionManager::connect(&cfg.redis_url).await?;
        sessions.ping().await?;

        let oidc = OidcClient::new(&cfg.idp)?;
        // The resolver authenticates its internal Identity lookup with a service
        // JWT it mints via the same keystore (fail-closed Identity, R1).
        let resolver: Arc<dyn PersonResolver> = Arc::new(IdentityPersonResolver::new(
            &cfg.identity_url,
            keystore.clone(),
            cfg.gateway_issuer.clone(),
            cfg.jwt_audience.clone(),
        ));

        // Parse the service-token registry (DD-AUTH-05). Fails closed at boot
        // on a malformed public key rather than on the first token request.
        let service_registry = ServiceRegistry::build(&cfg.service_tokens)?;
        tracing::info!(
            services = cfg.service_tokens.services.len(),
            token_bind = %cfg.service_tokens.token_bind_addr,
            "service-token registry loaded"
        );

        // Register the inter-gear client contract in the hub (DESIGN §3.10).
        // The same instance backs the admin revoke-by-user HTTP operation, so
        // the SDK contract is the single revoke path.
        let authn_client: Arc<dyn AuthenticatorClientV1> =
            Arc::new(LocalClient::new(sessions.clone()));
        ctx.client_hub()
            .register::<dyn AuthenticatorClientV1>(authn_client.clone());

        // Audit sink (PRD nfr-auth-audit). Fails the gear on a malformed
        // broker config; unconfigured = disabled (structured log only).
        let audit = crate::audit::AuditEmitter::new(
            &cfg.audit.brokers,
            &cfg.audit.topic,
            cfg.audit.retention_ms,
        )?;

        let state = Arc::new(AppState {
            cfg,
            sessions,
            keystore,
            oidc,
            resolver,
            service_registry,
            authn_client,
            audit,
        });
        self.state
            .set(state)
            .map_err(|_| anyhow::anyhow!("authenticator gear already initialized"))?;

        Ok(())
    }
}

impl RestApiCapability for AuthenticatorGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: axum::Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<axum::Router> {
        let state = self
            .state
            .get()
            .ok_or_else(|| anyhow::anyhow!("authenticator gear not initialized"))?
            .clone();
        Ok(api::register_routes(router, openapi, state))
    }
}

#[async_trait]
impl RunnableCapability for AuthenticatorGear {
    async fn start(&self, cancel: CancellationToken) -> anyhow::Result<()> {
        // `start` must return promptly — the host awaits it before starting the
        // next gear (including the api-gateway HTTP server). We bind the
        // service-token listener here (surfacing a bad bind at boot) and spawn
        // its server, holding `cancel` for graceful shutdown. The IdP refresher
        // (G5) and janitor land in step 10 and will spawn here the same way.
        let state = self
            .state
            .get()
            .ok_or_else(|| anyhow::anyhow!("authenticator gear not initialized"))?
            .clone();
        service_token::spawn(state.clone(), cancel.clone()).await?;
        // Leader-elected background workers (step 10): the IdP refresher (G5)
        // and the index janitor (DESIGN §4.3).
        crate::refresher::spawn(state.clone(), cancel.clone());
        crate::janitor::spawn(state, cancel);
        tracing::info!(
            "authenticator runnable: service-token listener + idp refresher + janitor started"
        );
        Ok(())
    }

    async fn stop(&self, _deadline_token: CancellationToken) -> anyhow::Result<()> {
        Ok(())
    }
}

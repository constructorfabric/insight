//! OIDC `AuthN` plugin module registration.
//!
//! Registers with the cyberfabric runtime via the `#[modkit::module]` macro.
//! Discovered by `authn-resolver` gateway via the GTS types-registry.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use authn_resolver_sdk::{AuthNResolverPluginClient, AuthNResolverPluginSpecV1};
use modkit::Module;
use modkit::client_hub::ClientScope;
use modkit::context::ModuleCtx;
use modkit::gts::BaseModkitPluginV1;
use tracing::info;
use types_registry_sdk::{RegisterResult, TypesRegistryClient};

use crate::config::OidcAuthnPluginConfig;
use crate::domain::client::OidcAuthnClient;
use crate::domain::service::OidcService;

/// OIDC authentication plugin module.
///
/// Validates JWT bearer tokens against an OIDC provider (Okta, Keycloak, Auth0, etc.)
/// using JWKS key discovery.
#[modkit::module(
    name = "oidc-authn-plugin",
    deps = ["types-registry"]
)]
pub struct OidcAuthnPlugin {
    service: OnceLock<Arc<OidcService>>,
}

impl Default for OidcAuthnPlugin {
    fn default() -> Self {
        Self {
            service: OnceLock::new(),
        }
    }
}

#[async_trait]
impl Module for OidcAuthnPlugin {
    async fn init(&self, ctx: &ModuleCtx) -> anyhow::Result<()> {
        let config: OidcAuthnPluginConfig = ctx.config()?;

        if config.issuer_url.is_empty() {
            anyhow::bail!(
                "oidc-authn-plugin: issuer_url is required. \
                 Set modules.oidc-authn-plugin.config.issuer_url in your config."
            );
        }

        if config.jwks_refresh_interval_seconds == 0 {
            anyhow::bail!("oidc-authn-plugin: jwks_refresh_interval_seconds must be > 0");
        }

        if config.leeway_seconds < 0 {
            anyhow::bail!(
                "oidc-authn-plugin: leeway_seconds must be >= 0, got {}",
                config.leeway_seconds
            );
        }

        info!(
            issuer = %config.issuer_url,
            audience = %config.audience,
            jwks_url = %config.effective_jwks_url(),
            "initializing OIDC authn plugin"
        );

        // Create JWKS key provider using modkit-auth
        let key_provider = Arc::new(
            modkit_auth::JwksKeyProvider::new(config.effective_jwks_url())?
                .with_refresh_interval(Duration::from_secs(config.jwks_refresh_interval_seconds)),
        );

        // Initial key fetch — log warning if IdP is unreachable, don't crash
        match modkit_auth::traits::KeyProvider::refresh_keys(key_provider.as_ref()).await {
            Ok(()) => info!("JWKS keys loaded successfully"),
            Err(e) => tracing::warn!(
                error = %e,
                "initial JWKS key fetch failed — will retry in background. \
                 Auth requests will fail until keys are loaded."
            ),
        }

        // Create service (not stored in OnceLock yet — registration may fail)
        let service = Arc::new(OidcService::new(&config, key_provider));

        // Generate plugin instance ID
        let instance_id = AuthNResolverPluginSpecV1::gts_make_instance_id(
            "insight.core.oidc_authn_resolver.plugin.v1",
        );

        // Register plugin instance in types-registry (fallible — do before OnceLock)
        let registry = ctx.client_hub().get::<dyn TypesRegistryClient>()?;
        let instance = BaseModkitPluginV1::<AuthNResolverPluginSpecV1> {
            id: instance_id.clone(),
            vendor: config.vendor.clone(),
            priority: config.priority,
            properties: AuthNResolverPluginSpecV1,
        };
        let instance_json = serde_json::to_value(&instance)?;
        let results = registry.register(vec![instance_json]).await?;
        RegisterResult::ensure_all_ok(&results)?;

        // Registration succeeded — now commit to OnceLock (irreversible)
        self.service
            .set(service.clone())
            .map_err(|_| anyhow::anyhow!("{} module already initialized", Self::MODULE_NAME))?;

        // Register scoped client in ClientHub
        let api: Arc<dyn AuthNResolverPluginClient> = Arc::new(OidcAuthnClient::new(service));
        ctx.client_hub()
            .register_scoped::<dyn AuthNResolverPluginClient>(
                ClientScope::gts_id(&instance_id),
                api,
            );

        info!(instance_id = %instance_id, "OIDC authn plugin initialized");
        Ok(())
    }
}

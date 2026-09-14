//! The MCP server: the custom surfaces, for a client that speaks MCP.
//!
//! It listens on a port of its own because the gear's REST router authenticates
//! against its own audience, which an MCP access token does not carry.

use std::sync::Arc;

use axum::Router;
use axum::middleware;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

use crate::config::McpConfig;

pub(crate) mod auth;
pub(crate) mod tools;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;

const MAX_REQUEST_BODY_BYTES: usize = 1024 * 1024;

pub(crate) fn router(
    config: &McpConfig,
    surfaces: tools::CustomSurfaces,
    cancellation: CancellationToken,
) -> anyhow::Result<Router> {
    let service: StreamableHttpService<tools::CustomSurfaces, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(surfaces.clone()),
            Arc::default(),
            StreamableHttpServerConfig::default()
                .with_legacy_session_mode(false)
                .with_json_response(true)
                .with_allowed_hosts(["localhost"])
                .with_max_request_body_bytes(MAX_REQUEST_BODY_BYTES)
                .with_cancellation_token(cancellation),
        );

    let verifier = auth::TokenVerifier::new(
        &config.public_url,
        &config.jwks_url,
        config.allow_insecure_private_network,
    )?;

    Ok(Router::new()
        .nest_service(auth::MCP_PATH, service)
        .layer(middleware::from_fn_with_state(verifier, auth::authenticate)))
}

pub(crate) async fn start(
    config: &McpConfig,
    surfaces: tools::CustomSurfaces,
    cancellation: CancellationToken,
) -> anyhow::Result<()> {
    if !config.enabled {
        return Ok(());
    }

    let router = router(config, surfaces, cancellation.child_token())?;

    let listener = tokio::net::TcpListener::bind(&config.bind_addr).await?;
    tracing::info!(bind_addr = %config.bind_addr, "custom-surface MCP server listening");

    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router)
            .with_graceful_shutdown(cancellation.cancelled_owned())
            .await
        {
            tracing::error!(%error, "the custom-surface MCP server stopped unexpectedly");
        }
    });

    Ok(())
}

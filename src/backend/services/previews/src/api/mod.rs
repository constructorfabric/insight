//! HTTP API layer — shared state, route table, offline OpenAPI emit.

pub(crate) mod canonical_json;
pub mod error;
pub mod experiments;

use std::sync::Arc;

use axum::Extension;
use axum::Router;
use axum::http::StatusCode;
use toolkit::api::{OpenApiInfo, OpenApiRegistry, OpenApiRegistryImpl, OperationBuilder};

use crate::config::GearConfig;
use crate::infra::cluster::Cluster;

/// Shared application state, injected into handlers via `Extension` (as one
/// `Arc`, never cloned per request).
pub struct AppState {
    /// Kubernetes access, scoped to the one configured namespace.
    pub cluster: Cluster,
    pub config: GearConfig,
    /// Serializes create admission (count-then-create); see the INVARIANT at
    /// its lock site.
    pub create_gate: tokio::sync::Mutex<()>,
}

/// Mount the previews routes onto the host's router. Gateway-JWT identity is
/// enforced entirely by the host authn pipeline (`oidc-authn-plugin` →
/// `SecurityContext`) — no bespoke JWT code.
pub fn register_routes(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let api = build_operations(Router::new(), openapi).layer(Extension(state));

    host_router.merge(api)
}

/// Title/version/description of the emitted document. Kept in step with the
/// `openapi` block of `config/insight.yaml`.
fn openapi_info() -> OpenApiInfo {
    OpenApiInfo {
        title: "Insight Previews API".to_owned(),
        version: "1.0.0".to_owned(),
        description: Some(
            "Self-service preview experiments: each experiment is one frontend \
             build served under /exp/<name> on the shared preview host, backed \
             by a Deployment/Service/HTTPRoute trio in a dedicated namespace. \
             The API Gateway mounts this service at /api/previews."
                .to_owned(),
        ),
        servers: Vec::new(),
    }
}

/// Build the previews `OpenAPI` document **offline** — no `AppState`, cluster
/// or HTTP listener. Backs the `previews openapi` subcommand (committed-doc
/// regeneration + drift gate), reusing the exact `build_operations` route
/// table the live gear serves, so the two cannot diverge.
///
/// # Errors
///
/// Returns an error if the registry cannot assemble the document.
pub fn openapi_document() -> anyhow::Result<utoipa::openapi::OpenApi> {
    let openapi = OpenApiRegistryImpl::new();
    let _ = build_operations(Router::new(), &openapi);

    openapi
        .build_openapi(&openapi_info())
        .map_err(|e| anyhow::anyhow!("failed to build previews OpenAPI document: {e}"))
}

/// Declare each operation via the toolkit `OperationBuilder` (records the
/// route + its OpenAPI spec + auth/error metadata).
fn build_operations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    let router = OperationBuilder::get("/v1/experiments")
        .operation_id("previews.experiments.list")
        .summary("List the live preview experiments")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<experiments::ExperimentListResponse>(
            openapi,
            StatusCode::OK,
            "Every live experiment, read from the cluster",
        )
        .standard_errors(openapi)
        .handler(experiments::list_experiments)
        .register(router, openapi);

    let router = OperationBuilder::post("/v1/experiments")
        .operation_id("previews.experiments.create")
        .summary(
            "Create a preview experiment (name + FE image tag); requires the \
             previews-admin or admin role",
        )
        .authenticated()
        .no_license_required()
        .json_request::<experiments::CreateExperimentRequest>(openapi, "Experiment to create")
        .json_response_with_schema::<experiments::ExperimentResponse>(
            openapi,
            StatusCode::CREATED,
            "Created experiment",
        )
        .standard_errors(openapi)
        .handler(experiments::create_experiment)
        .register(router, openapi);

    OperationBuilder::delete("/v1/experiments/{name}")
        .operation_id("previews.experiments.delete")
        .summary("Delete a preview experiment; requires the previews-admin or admin role")
        .authenticated()
        .path_param("name", "Experiment slug (the `/exp/<name>` segment)")
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Experiment deleted")
        .standard_errors(openapi)
        .handler(experiments::delete_experiment)
        .register(router, openapi)
}

#[cfg(test)]
mod openapi_tests {
    use super::*;

    #[test]
    fn the_document_builds_without_state_or_a_cluster() -> anyhow::Result<()> {
        let document = openapi_document()?;

        for path in ["/v1/experiments", "/v1/experiments/{name}"] {
            assert!(
                document.paths.paths.contains_key(path),
                "missing {path}: {:?}",
                document.paths.paths.keys().collect::<Vec<_>>()
            );
        }
        Ok(())
    }

    #[test]
    fn the_templated_delete_declares_its_path_parameter() -> anyhow::Result<()> {
        let document = openapi_document()?;

        let item = document
            .paths
            .paths
            .get("/v1/experiments/{name}")
            .ok_or_else(|| anyhow::anyhow!("delete path missing"))?;
        let delete = item
            .delete
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("delete operation missing"))?;

        let declared = delete.parameters.as_ref().map_or(0, Vec::len);
        assert_eq!(declared, 1, "the {{name}} parameter must be declared");
        Ok(())
    }
}

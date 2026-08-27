//! Experiment HTTP surface — list / create / delete the per-experiment trio.
//!
//! Thin orchestration only: validate into domain types, call the cluster,
//! map outcomes to canonical errors. The caller is the gateway-JWT subject
//! (`SecurityContext`, verified by the host authn pipeline).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::canonical_json::CanonicalJson;
use super::error::ExperimentError;
use crate::domain::experiment::{
    Experiment, ExperimentName, ExperimentStatus, ImageTag, TtlDays, experiment_url,
};
use crate::domain::objects::{self, ExperimentStamp};
use crate::infra::cluster::{CreateError, DeleteOutcome};

/// Body of `POST /v1/experiments`. The image repository is fixed server-side;
/// only the tag varies.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateExperimentRequest {
    /// The `/exp/<name>` slug: a DNS-1123 label, at most 55 characters.
    pub name: String,
    /// FE image tag: a `preview-…` tag or a CI build tag.
    pub tag: String,
    /// Days until the TTL sweep removes the experiment; server default and
    /// maximum apply.
    pub ttl_days: Option<u32>,
}
impl toolkit::api::api_dto::RequestApiDto for CreateExperimentRequest {}

/// One experiment as served by the API.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentResponse {
    pub name: String,
    pub tag: String,
    /// Where the experiment serves: `https://<host>/exp/<name>/`.
    pub url: String,
    /// The creating gateway-JWT subject.
    pub creator: String,
    pub created_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub status: ExperimentStatus,
}
impl toolkit::api::api_dto::ResponseApiDto for ExperimentResponse {}

/// List wrapper for `GET /v1/experiments`.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExperimentListResponse {
    pub experiments: Vec<ExperimentResponse>,
}
impl toolkit::api::api_dto::ResponseApiDto for ExperimentListResponse {}

fn to_response(experiment: Experiment, state: &AppState) -> ExperimentResponse {
    let url = experiment_url(
        &state.config.route_host,
        &state.config.base_path,
        &experiment.name,
    );
    ExperimentResponse {
        name: experiment.name,
        tag: experiment.tag,
        url,
        creator: experiment.creator,
        created_at: experiment.created_at,
        expires_at: experiment.expires_at,
        status: experiment.status,
    }
}

/// `GET /v1/experiments` — every live experiment, read from the cluster.
pub async fn list_experiments(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_caller(&ctx)?;

    let deployments = state
        .cluster
        .list_experiment_deployments()
        .await
        .map_err(list_err)?;

    let now = Utc::now();
    let mut experiments: Vec<ExperimentResponse> = deployments
        .iter()
        .filter_map(|d| objects::experiment_from_deployment(d, now))
        .map(|e| to_response(e, &state))
        .collect();
    experiments.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(ExperimentListResponse { experiments }))
}

/// `POST /v1/experiments` — create the Deployment/Service/HTTPRoute trio.
pub async fn create_experiment(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    CanonicalJson(req): CanonicalJson<CreateExperimentRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let caller = require_caller(&ctx)?;

    let name = ExperimentName::parse(&req.name).map_err(|why| field_violation("name", &why))?;
    let tag = ImageTag::parse(&req.tag).map_err(|why| field_violation("tag", &why))?;
    let ttl = TtlDays::parse(
        req.ttl_days,
        state.config.default_ttl_days,
        state.config.max_ttl_days,
    )
    .map_err(|why| field_violation("ttlDays", &why))?;

    if state.config.route_host.is_empty() {
        return Err(ExperimentError::failed_precondition()
            .with_precondition_violation(
                "route_host",
                "no preview host is configured; experiments cannot be routed",
                "route_host_unset",
            )
            .create());
    }

    // INVARIANT: held across the list+create awaits below — the hold IS the
    // cap enforcement: concurrent creates would otherwise all pass the count
    // before any write lands and exceed `max_experiments`.
    let _admission = state.create_gate.lock().await;

    let deployments = state
        .cluster
        .list_experiment_deployments()
        .await
        .map_err(list_err)?;
    let live = objects::live_experiment_count(&deployments, Utc::now());
    if live >= state.config.max_experiments {
        return Err(ExperimentError::resource_exhausted(format!(
            "the cap of {} live experiments is reached; delete one first",
            state.config.max_experiments
        ))
        .with_quota_violation("experiments", "the live-experiment cap is reached")
        .create());
    }

    let created_at = Utc::now();
    let stamp = ExperimentStamp {
        creator: caller.to_string(),
        created_at,
        expires_at: ttl.expires_at(created_at),
    };
    let route_target = state.config.route_target();

    let deployment = objects::deployment(&name, &tag, &stamp);
    let service = objects::service(&name, &tag, &stamp);
    let route = objects::http_route(&name, &tag, &stamp, &route_target);

    match state.cluster.create_trio(deployment, service, route).await {
        Ok(()) => {}
        Err(CreateError::AlreadyExists) => {
            return Err(ExperimentError::already_exists(format!(
                "experiment '{}' already exists",
                name.as_str()
            ))
            .with_resource(name.as_str().to_owned())
            .create());
        }
        Err(CreateError::Failed(e)) => {
            tracing::error!(error = %format!("{e:#}"), name = name.as_str(), "experiment create failed");
            return Err(CanonicalError::internal("failed to create the experiment").create());
        }
    }

    tracing::info!(
        name = name.as_str(),
        tag = tag.as_str(),
        creator = %caller,
        expires_at = %stamp.expires_at,
        "experiments.create"
    );

    let record = Experiment {
        name: name.as_str().to_owned(),
        tag: tag.as_str().to_owned(),
        creator: stamp.creator.clone(),
        created_at: Some(stamp.created_at),
        expires_at: Some(stamp.expires_at),
        status: ExperimentStatus::Pending,
    };
    Ok((StatusCode::CREATED, Json(to_response(record, &state))))
}

/// `DELETE /v1/experiments/{name}` — remove the trio.
pub async fn delete_experiment(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Path(raw_name): Path<String>,
) -> Result<impl IntoResponse, CanonicalError> {
    let caller = require_caller(&ctx)?;

    let name = ExperimentName::parse(&raw_name).map_err(|why| field_violation("name", &why))?;

    let outcome = state
        .cluster
        .delete_trio(&name.resource_name())
        .await
        .map_err(|e| {
            tracing::error!(error = %format!("{e:#}"), name = name.as_str(), "experiment delete failed");
            CanonicalError::internal("failed to delete the experiment").create()
        })?;

    match outcome {
        DeleteOutcome::Deleted => {
            tracing::info!(name = name.as_str(), caller = %caller, "experiments.delete");
            Ok(StatusCode::NO_CONTENT)
        }
        DeleteOutcome::NotFound => Err(ExperimentError::not_found("experiment not found")
            .with_resource(name.as_str().to_owned())
            .create()),
    }
}

/// Require an identified caller (gateway-JWT subject); 401 otherwise.
fn require_caller(ctx: &SecurityContext) -> Result<Uuid, CanonicalError> {
    let caller = ctx.subject_id();
    if caller.is_nil() {
        return Err(CanonicalError::unauthenticated()
            .with_reason("caller not identified: the gateway JWT carries no subject")
            .create());
    }
    Ok(caller)
}

fn field_violation(field: &'static str, why: &str) -> CanonicalError {
    ExperimentError::invalid_argument()
        .with_field_violation(field, why, "INVALID")
        .create()
}

#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn list_err(e: anyhow::Error) -> CanonicalError {
    tracing::error!(error = %format!("{e:#}"), "experiment listing failed");
    CanonicalError::internal("failed to list experiments").create()
}

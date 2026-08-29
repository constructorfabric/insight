use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Json;
use axum::extract::Extension;
use axum::http::HeaderMap;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::AppState;
use crate::api::error::ReportError;
use crate::domain::metric_access::authorize_tenant_metrics;
use crate::domain::person_visibility::authorize_and_hydrate_person_profiles;
use crate::domain::reports::dto::{ReportPreviewRequest, ReportPreviewResponse};
use crate::domain::reports::executor::{
    ReportExecutionContext, ReportRowSink, ReportSinkError, execute_report,
};
use crate::domain::reports::planner::{ReportPlan, ReportPlannerLimits, plan_report};
use crate::domain::reports::query::ClickHouseReportQueryRunner;
use crate::domain::reports::row::ReportRow;
use crate::domain::reports::validation::{
    ReportSubjectSelection, ValidatedReportRecipe, validate_preview,
};
use crate::infra::identity::IdentityProfile;

mod export;
pub(super) use export::export_report;

const MAX_PREVIEW_ROWS: usize = 20;

pub async fn preview_report(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Json(request): Json<ReportPreviewRequest>,
) -> Result<Json<ReportPreviewResponse>, CanonicalError> {
    let deadline = tokio::time::Instant::now()
        + Duration::from_secs(state.config.reports.request_timeout_secs);
    let response = tokio::time::timeout_at(
        deadline,
        build_preview(&state, &ctx, &headers, request, deadline),
    )
    .await
    .map_err(|_| report_limit("report preview timed out"))??;

    Ok(Json(response))
}

async fn build_preview(
    state: &Arc<AppState>,
    ctx: &SecurityContext,
    headers: &HeaderMap,
    request: ReportPreviewRequest,
    deadline: tokio::time::Instant,
) -> Result<ReportPreviewResponse, CanonicalError> {
    authorize_tenant_subject(state, &request.subject)?;

    let recipe = validate_preview(&state.db, ctx.subject_tenant_id(), request).await?;
    let _generation = export::acquire_generation(state, deadline).await?;
    let profiles = hydrate_profiles(state, ctx, headers, &recipe).await?;
    let limits = planner_limits(state);
    let full_plan = plan_report(&recipe, &profiles, limits).map_err(map_planning_error)?;
    let (preview_recipe, preview_profiles) = preview_inputs(&recipe, &full_plan, profiles)
        .ok_or_else(|| CanonicalError::internal("report preview could not be planned").create())?;
    let mut preview_plan =
        plan_report(&preview_recipe, &preview_profiles, limits).map_err(map_planning_error)?;
    preview_plan.columns = full_plan.columns.clone();
    let runner = ClickHouseReportQueryRunner::new(&state.ch);
    let rows = execute_report(
        &preview_recipe,
        &preview_plan,
        &preview_profiles,
        ReportExecutionContext {
            tenant_id: ctx.subject_tenant_id(),
            enforce_tenant_scope: state.config.metric_catalog.enforce_tenant_scope,
        },
        &runner,
        PreviewSink::default(),
    )
    .await
    .map_err(report_internal)?;

    Ok(ReportPreviewResponse {
        columns: full_plan
            .columns
            .iter()
            .map(|column| column.metadata.clone())
            .collect(),
        rows,
        total_rows: full_plan.size.total_rows,
    })
}

pub(super) fn authorize_tenant_subject(
    state: &AppState,
    subject: &crate::domain::reports::dto::ReportSubject,
) -> Result<(), CanonicalError> {
    if matches!(
        subject,
        crate::domain::reports::dto::ReportSubject::Tenant {}
    ) {
        authorize_tenant_metrics(state.config.metric_catalog.tenant_metrics_enabled)?;
    }

    Ok(())
}

pub(super) async fn hydrate_profiles(
    state: &AppState,
    ctx: &SecurityContext,
    headers: &HeaderMap,
    recipe: &ValidatedReportRecipe,
) -> Result<Vec<IdentityProfile>, CanonicalError> {
    let ReportSubjectSelection::People { ids } = &recipe.subject else {
        return Ok(Vec::new());
    };

    authorize_and_hydrate_person_profiles(
        &state.identity,
        ctx,
        super::forwarded_authorization(headers),
        ids,
    )
    .await
}

fn planner_limits(state: &AppState) -> ReportPlannerLimits {
    ReportPlannerLimits {
        max_batch_cells: state.config.reports.max_batch_cells,
        max_total_cells: state.config.reports.max_total_cells,
    }
}

pub(super) fn map_planning_error(
    error: crate::domain::reports::planner::ReportPlanningError,
) -> CanonicalError {
    if matches!(
        error,
        crate::domain::reports::planner::ReportPlanningError::CellLimitExceeded
            | crate::domain::reports::planner::ReportPlanningError::BatchLimitTooSmall
            | crate::domain::reports::planner::ReportPlanningError::PeriodLimitExceeded
    ) {
        return ReportError::invalid_argument()
            .with_field_violation("report", error.to_string(), "LIMIT_EXCEEDED")
            .create();
    }

    report_internal(error)
}

fn preview_inputs(
    recipe: &ValidatedReportRecipe,
    full_plan: &ReportPlan,
    profiles: Vec<IdentityProfile>,
) -> Option<(ValidatedReportRecipe, Vec<IdentityProfile>)> {
    let period_count = full_plan.periods.len().min(MAX_PREVIEW_ROWS);
    let preview_to = full_plan.periods.get(period_count.checked_sub(1)?)?.to;
    let (subject, preview_profiles) = match &recipe.subject {
        ReportSubjectSelection::People { ids } => {
            let people = MAX_PREVIEW_ROWS.div_ceil(period_count).min(ids.len());
            (
                ReportSubjectSelection::People {
                    ids: ids[..people].to_vec(),
                },
                profiles.into_iter().take(people).collect(),
            )
        }
        ReportSubjectSelection::Tenant { id } => {
            (ReportSubjectSelection::Tenant { id: *id }, vec![])
        }
    };

    Some((
        ValidatedReportRecipe {
            subject,
            from: recipe.from,
            to: preview_to,
            granularity: recipe.granularity,
            metrics: recipe.metrics.clone(),
        },
        preview_profiles,
    ))
}

pub(super) fn report_internal(error: impl std::fmt::Debug) -> CanonicalError {
    tracing::error!(?error, "report processing failed");
    CanonicalError::internal("report processing failed").create()
}

pub(super) fn report_limit(description: &str) -> CanonicalError {
    ReportError::resource_exhausted("Report exceeded resource limits.")
        .with_quota_violation("report", description)
        .create()
}

#[derive(Debug, Default)]
struct PreviewSink {
    rows: Vec<ReportRow>,
}

#[async_trait]
impl ReportRowSink for PreviewSink {
    type Output = Vec<ReportRow>;

    async fn write_rows(&mut self, rows: &[ReportRow]) -> Result<(), ReportSinkError> {
        let remaining = MAX_PREVIEW_ROWS.saturating_sub(self.rows.len());

        self.rows.extend(rows.iter().take(remaining).cloned());
        Ok(())
    }

    async fn finish(self) -> Result<Self::Output, ReportSinkError> {
        Ok(self.rows)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use chrono::NaiveDate;
    use uuid::Uuid;

    use super::*;
    use crate::domain::reports::dto::ReportGranularity;

    #[tokio::test]
    async fn preview_keeps_full_columns_and_caps_rows_at_twenty() {
        let profiles = vec![
            profile(1, []),
            profile(2, []),
            profile(3, [("department", "Engineering")]),
        ];
        let recipe = ValidatedReportRecipe {
            subject: ReportSubjectSelection::People {
                ids: profiles.iter().map(|profile| profile.person_id).collect(),
            },
            from: date("2026-01-01"),
            to: date("2026-12-31"),
            granularity: ReportGranularity::Month,
            metrics: vec![],
        };
        let full_plan = plan_report(
            &recipe,
            &profiles,
            ReportPlannerLimits {
                max_batch_cells: usize::MAX,
                max_total_cells: u64::MAX,
            },
        )
        .unwrap_or_else(|error| panic!("full report should plan: {error}"));
        let (preview_recipe, preview_profiles) = preview_inputs(&recipe, &full_plan, profiles)
            .unwrap_or_else(|| panic!("preview inputs should exist"));
        let mut preview_plan = plan_report(
            &preview_recipe,
            &preview_profiles,
            ReportPlannerLimits {
                max_batch_cells: usize::MAX,
                max_total_cells: u64::MAX,
            },
        )
        .unwrap_or_else(|error| panic!("preview report should plan: {error}"));
        preview_plan.columns = full_plan.columns.clone();

        assert_eq!(preview_plan.size.total_rows, 24);
        assert_eq!(preview_profiles.len(), 2);
        assert!(
            preview_plan
                .columns
                .iter()
                .any(|column| column.metadata.key == "department")
        );

        let mut sink = PreviewSink::default();
        sink.write_rows(&vec![ReportRow::from(Vec::new()); 12])
            .await
            .unwrap_or_else(|error| panic!("first preview batch should write: {error}"));
        sink.write_rows(&vec![ReportRow::from(Vec::new()); 12])
            .await
            .unwrap_or_else(|error| panic!("second preview batch should write: {error}"));
        let rows = sink
            .finish()
            .await
            .unwrap_or_else(|error| panic!("preview should finish: {error}"));

        assert_eq!(rows.len(), MAX_PREVIEW_ROWS);
    }

    #[test]
    fn deterministic_planner_limits_are_invalid_arguments() {
        for error in [
            crate::domain::reports::planner::ReportPlanningError::CellLimitExceeded,
            crate::domain::reports::planner::ReportPlanningError::BatchLimitTooSmall,
            crate::domain::reports::planner::ReportPlanningError::PeriodLimitExceeded,
        ] {
            assert_eq!(
                map_planning_error(error).into_response().status(),
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[test]
    fn tenant_preview_stops_after_twenty_periods() {
        let tenant_id = Uuid::from_u128(9);
        let recipe = ValidatedReportRecipe {
            subject: ReportSubjectSelection::Tenant { id: tenant_id },
            from: date("2026-01-01"),
            to: date("2026-02-28"),
            granularity: ReportGranularity::Day,
            metrics: vec![],
        };
        let full_plan = plan_report(&recipe, &[], test_planner_limits())
            .unwrap_or_else(|error| panic!("full report should plan: {error}"));
        let (preview_recipe, preview_profiles) = preview_inputs(&recipe, &full_plan, vec![])
            .unwrap_or_else(|| panic!("preview inputs should exist"));
        let preview_plan = plan_report(&preview_recipe, &preview_profiles, test_planner_limits())
            .unwrap_or_else(|error| panic!("preview report should plan: {error}"));

        assert_eq!(preview_plan.size.total_rows, MAX_PREVIEW_ROWS as u64);
        assert_eq!(
            preview_plan.periods.last().map(|period| period.to),
            Some(date("2026-01-20"))
        );
    }

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .unwrap_or_else(|error| panic!("fixture date must parse: {error}"))
    }

    const fn test_planner_limits() -> ReportPlannerLimits {
        ReportPlannerLimits {
            max_batch_cells: usize::MAX,
            max_total_cells: u64::MAX,
        }
    }

    fn profile<const N: usize>(id: u128, attributes: [(&str, &str); N]) -> IdentityProfile {
        IdentityProfile {
            person_id: Uuid::from_u128(id),
            attributes: attributes
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect::<BTreeMap<_, _>>(),
            supervisor: None,
        }
    }
}

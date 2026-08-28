use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::Extension;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, Response};
use futures::StreamExt;
use tokio::sync::OwnedSemaphorePermit;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::{AppState, authorize_tenant_subject, hydrate_profiles, report_internal};
use crate::api::error::ReportError;
use crate::domain::reports::dto::{ReportExportFormat, ReportExportRequest};
use crate::domain::reports::executor::{ReportExecutionContext, execute_report};
use crate::domain::reports::export::{ReportArtifact, ReportExportError, start_report_writer};
use crate::domain::reports::planner::{ReportPlannerLimits, plan_report};
use crate::domain::reports::query::ClickHouseReportQueryRunner;
use crate::domain::reports::validation::validate_export;

pub(crate) async fn export_report(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Json(request): Json<ReportExportRequest>,
) -> Result<Response<Body>, CanonicalError> {
    let deadline = tokio::time::Instant::now()
        + Duration::from_secs(state.config.reports.request_timeout_secs);
    authorize_tenant_subject(&state, &request.recipe)?;

    let format = request.format;
    let recipe = tokio::time::timeout_at(
        deadline,
        validate_export(&state.db, ctx.subject_tenant_id(), request),
    )
    .await
    .map_err(|_| report_limit("report export timed out"))??;
    let profiles =
        tokio::time::timeout_at(deadline, hydrate_profiles(&state, &ctx, &headers, &recipe))
            .await
            .map_err(|_| report_limit("report export timed out"))??;
    let plan = plan_report(
        &recipe,
        &profiles,
        ReportPlannerLimits {
            max_batch_cells: state.config.reports.max_batch_cells,
        },
    )
    .map_err(report_internal)?;
    let artifact_permit = acquire_artifact(&state, deadline).await?;
    let generation = acquire_generation(&state, deadline).await?;
    let (sink, writer) = start_report_writer(
        format,
        &plan,
        state.config.reports.temp_dir.clone(),
        state.config.reports.max_generated_bytes,
        state.config.reports.writer_channel_batches,
    )
    .map_err(map_export_error)?;
    let runner = ClickHouseReportQueryRunner::new(&state.ch);
    let execution = execute_report(
        &recipe,
        &plan,
        &profiles,
        ReportExecutionContext {
            tenant_id: ctx.subject_tenant_id(),
            enforce_tenant_scope: state.config.metric_catalog.enforce_tenant_scope,
        },
        &runner,
        sink,
    );

    match tokio::time::timeout_at(deadline, execution).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            stop_writer(writer, deadline).await;
            return Err(report_internal(error));
        }
        Err(_) => {
            stop_writer(writer, deadline).await;
            return Err(report_limit("report export timed out"));
        }
    }
    let artifact = tokio::time::timeout_at(deadline, writer)
        .await
        .map_err(|_| report_limit("report export timed out"))?
        .map_err(|_| report_internal("report writer task failed"))?
        .map_err(map_export_error)?;
    drop(generation);

    response_for_artifact(
        artifact,
        artifact_permit,
        report_filename(&recipe, format),
        format,
    )
    .await
}

async fn stop_writer(
    mut writer: tokio::task::JoinHandle<Result<ReportArtifact, ReportExportError>>,
    deadline: tokio::time::Instant,
) {
    if tokio::time::timeout_at(deadline, &mut writer)
        .await
        .is_err()
    {
        writer.abort();
    }
}

async fn acquire_generation(
    state: &AppState,
    deadline: tokio::time::Instant,
) -> Result<OwnedSemaphorePermit, CanonicalError> {
    tokio::time::timeout_at(
        capacity_deadline(state, deadline),
        state.report_generations.clone().acquire_owned(),
    )
    .await
    .map_err(|_| {
        report_busy(
            "report generations",
            state.config.reports.capacity_wait_secs,
        )
    })?
    .map_err(|_| {
        report_busy(
            "report generations",
            state.config.reports.capacity_wait_secs,
        )
    })
}

async fn acquire_artifact(
    state: &AppState,
    deadline: tokio::time::Instant,
) -> Result<OwnedSemaphorePermit, CanonicalError> {
    tokio::time::timeout_at(
        capacity_deadline(state, deadline),
        state.report_artifacts.clone().acquire_owned(),
    )
    .await
    .map_err(|_| report_busy("report downloads", state.config.reports.capacity_wait_secs))?
    .map_err(|_| report_busy("report downloads", state.config.reports.capacity_wait_secs))
}

fn capacity_deadline(state: &AppState, deadline: tokio::time::Instant) -> tokio::time::Instant {
    deadline.min(
        tokio::time::Instant::now() + Duration::from_secs(state.config.reports.capacity_wait_secs),
    )
}

async fn response_for_artifact(
    artifact: ReportArtifact,
    artifact_permit: OwnedSemaphorePermit,
    filename: String,
    format: ReportExportFormat,
) -> Result<Response<Body>, CanonicalError> {
    let artifact_path = artifact.path().map_err(map_export_error)?.to_path_buf();
    let content_length = artifact.content_length();
    let file = tokio::fs::File::open(&artifact_path)
        .await
        .map_err(|_| report_internal("report artifact could not be opened"))?;
    if tokio::fs::remove_file(&artifact_path).await.is_err() {
        drop(file);
        let _ = tokio::fs::remove_file(&artifact_path).await;
        return Err(report_internal("report artifact could not be removed"));
    }
    let _ = artifact.disarm().map_err(map_export_error)?;

    let content_type = match format {
        ReportExportFormat::Csv => "text/csv; charset=utf-8",
        ReportExportFormat::Xlsx => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        }
    };
    let stream = tokio_util::io::ReaderStream::new(file).map(move |result| {
        let _ = &artifact_permit;
        result
    });
    let content_length = HeaderValue::from_str(&content_length.to_string())
        .map_err(|_| report_internal("report artifact length is invalid"))?;

    Response::builder()
        .header(CONTENT_TYPE, HeaderValue::from_static(content_type))
        .header(CONTENT_LENGTH, content_length)
        .header(
            CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
                .map_err(|_| report_internal("report filename is invalid"))?,
        )
        .body(Body::from_stream(stream))
        .map_err(|_| report_internal("report response could not be built"))
}

fn report_filename(
    recipe: &crate::domain::reports::validation::ValidatedReportRecipe,
    format: ReportExportFormat,
) -> String {
    let subject = match &recipe.subject {
        crate::domain::reports::validation::ReportSubjectSelection::People { .. } => "people",
        crate::domain::reports::validation::ReportSubjectSelection::Tenant { .. } => "tenant",
    };
    let granularity = match recipe.granularity {
        crate::domain::reports::dto::ReportGranularity::Day => "day",
        crate::domain::reports::dto::ReportGranularity::Week => "week",
        crate::domain::reports::dto::ReportGranularity::Month => "month",
        crate::domain::reports::dto::ReportGranularity::Quarter => "quarter",
        crate::domain::reports::dto::ReportGranularity::Year => "year",
    };
    let extension = match format {
        ReportExportFormat::Csv => "csv",
        ReportExportFormat::Xlsx => "xlsx",
    };

    format!(
        "insight-report_{subject}_{granularity}_{}_{}.{}",
        recipe.from, recipe.to, extension
    )
}

fn map_export_error(error: ReportExportError) -> CanonicalError {
    match error {
        ReportExportError::Csv(
            crate::domain::reports::csv::ReportCsvError::GeneratedByteLimit { .. },
        )
        | ReportExportError::Xlsx(
            crate::domain::reports::xlsx::ReportXlsxError::OutputLimitExceeded,
        ) => report_limit("report output exceeds the configured size limit"),
        ReportExportError::XlsxDimensions => report_limit("report exceeds XLSX dimensions"),
        error => {
            tracing::error!(?error, "report export failed");
            report_internal("report export failed")
        }
    }
}

fn report_limit(description: &str) -> CanonicalError {
    ReportError::resource_exhausted("Report export exceeded resource limits.")
        .with_quota_violation("report export", description)
        .create()
}

fn report_busy(resource: &str, retry_after_secs: u64) -> CanonicalError {
    ReportError::resource_exhausted("Report export capacity is busy.")
        .with_quota_violation(resource, "concurrency limit reached")
        .with_quota_violation_retry_after_seconds(retry_after_secs)
        .create()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn response_unlinks_artifact_before_streaming() {
        let path = std::env::temp_dir().join(format!("report-download-{}", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, b"report")
            .await
            .unwrap_or_else(|error| panic!("artifact must write: {error}"));
        let artifact = ReportArtifact::from_completed(path.clone(), 6);
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .unwrap_or_else(|_| panic!("permit must acquire"));

        let response = response_for_artifact(
            artifact,
            permit,
            "insight-report_people_day_2026-01-01_2026-01-01.csv".to_owned(),
            ReportExportFormat::Csv,
        )
        .await
        .unwrap_or_else(|error| panic!("response must build: {error}"));

        assert!(!path.exists());
        assert_eq!(response.headers()[CONTENT_LENGTH], "6");
        assert_eq!(
            response.headers()[CONTENT_DISPOSITION],
            "attachment; filename=\"insight-report_people_day_2026-01-01_2026-01-01.csv\""
        );
        assert_eq!(semaphore.available_permits(), 0);
        drop(response);
        assert_eq!(semaphore.available_permits(), 1);
    }
}

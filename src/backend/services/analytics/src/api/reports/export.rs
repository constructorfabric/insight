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

use super::{
    AppState, authorize_tenant_subject, hydrate_profiles, map_planning_error, report_internal,
    report_limit,
};
use crate::api::error::ReportError;
use crate::domain::reports::dto::{ReportExportFormat, ReportExportRequest};
use crate::domain::reports::executor::{
    ReportExecutionContext, ReportExecutionError, execute_report,
};
use crate::domain::reports::export::{
    ReportArtifact, ReportExportError, ReportWriterLimits, start_report_writer,
};
use crate::domain::reports::planner::{ReportPlannerLimits, plan_report};
use crate::domain::reports::query::ClickHouseReportQueryRunner;
use crate::domain::reports::telemetry::{ReportCleanupOutcome, ReportTelemetry};
use crate::domain::reports::validation::validate_export;

pub(crate) async fn export_report(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Json(request): Json<ReportExportRequest>,
) -> Result<Response<Body>, CanonicalError> {
    let telemetry = ReportTelemetry::new(&request.subject, request.format);
    let response = export_report_inner(&state, &ctx, &headers, request, &telemetry).await;
    if response.is_err() {
        telemetry.fail();
    }

    response
}

async fn export_report_inner(
    state: &Arc<AppState>,
    ctx: &SecurityContext,
    headers: &HeaderMap,
    request: ReportExportRequest,
    telemetry: &ReportTelemetry,
) -> Result<Response<Body>, CanonicalError> {
    let deadline = tokio::time::Instant::now()
        + Duration::from_secs(state.config.reports.request_timeout_secs);
    authorize_tenant_subject(state, &request.subject)?;

    let format = request.format;
    let recipe = tokio::time::timeout_at(
        deadline,
        validate_export(&state.db, ctx.subject_tenant_id(), request),
    )
    .await
    .map_err(|_| report_limit("report export timed out"))??;
    let admission_started = std::time::Instant::now();
    let artifact_permit = acquire_artifact(state, deadline).await?;
    let generation = acquire_generation(state, deadline).await?;
    telemetry.record_admission_wait(admission_started.elapsed());
    let identity_started = std::time::Instant::now();
    let profiles =
        tokio::time::timeout_at(deadline, hydrate_profiles(state, ctx, headers, &recipe))
            .await
            .map_err(|_| report_limit("report export timed out"))??;
    telemetry.record_identity_duration(identity_started.elapsed());
    let plan = plan_report(
        &recipe,
        &profiles,
        ReportPlannerLimits {
            max_batch_cells: state.config.reports.max_batch_cells,
            max_total_cells: state.config.reports.max_total_cells,
        },
    )
    .map_err(map_planning_error)?;
    let (sink, writer) = start_report_writer(
        format,
        &plan,
        state.config.reports.temp_dir.clone(),
        ReportWriterLimits {
            max_generated_bytes: state.config.reports.max_generated_bytes,
            max_xlsx_spool_bytes: state.config.reports.max_xlsx_spool_bytes,
            channel_batches: state.config.reports.writer_channel_batches,
        },
        telemetry.clone(),
        generation,
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

    let query_started = std::time::Instant::now();
    match tokio::time::timeout_at(deadline, execution).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return Err(map_execution_failure(error, writer, deadline).await);
        }
        Err(_) => {
            stop_writer(writer, deadline).await;
            return Err(report_limit("report export timed out"));
        }
    }
    telemetry.record_query_duration(query_started.elapsed());
    let artifact = tokio::time::timeout_at(deadline, writer)
        .await
        .map_err(|_| report_limit("report export timed out"))?
        .map_err(|_| report_internal("report writer task failed"))?
        .map_err(map_export_error)?;
    let rows = plan.size.total_rows;
    let columns = plan.size.worksheet_columns;
    let bytes = artifact.content_length();
    let response = response_for_artifact(
        artifact,
        artifact_permit,
        report_filename(&recipe, format),
        format,
        telemetry,
    )
    .await?;

    telemetry.succeed(rows, columns, bytes);

    Ok(response)
}

async fn stop_writer(
    writer: tokio::task::JoinHandle<Result<ReportArtifact, ReportExportError>>,
    deadline: tokio::time::Instant,
) {
    let _ = tokio::time::timeout_at(deadline, writer).await;
}

async fn map_execution_failure(
    error: ReportExecutionError,
    writer: tokio::task::JoinHandle<Result<ReportArtifact, ReportExportError>>,
    deadline: tokio::time::Instant,
) -> CanonicalError {
    if !matches!(&error, ReportExecutionError::Sink(_)) {
        stop_writer(writer, deadline).await;
        return report_internal(error);
    }

    match tokio::time::timeout_at(deadline, writer).await {
        Err(_) => report_limit("report export timed out"),
        Ok(Err(_)) => report_internal("report writer task failed"),
        Ok(Ok(Ok(artifact))) => {
            drop(artifact);
            report_internal(error)
        }
        Ok(Ok(Err(error))) => map_export_error(error),
    }
}

pub(super) async fn acquire_generation(
    state: &AppState,
    deadline: tokio::time::Instant,
) -> Result<OwnedSemaphorePermit, CanonicalError> {
    acquire_capacity(
        state.report_generations.clone(),
        capacity_deadline(state, deadline),
        "report generations",
        state.config.reports.capacity_wait_secs,
    )
    .await
}

async fn acquire_artifact(
    state: &AppState,
    deadline: tokio::time::Instant,
) -> Result<OwnedSemaphorePermit, CanonicalError> {
    acquire_capacity(
        state.report_artifacts.clone(),
        capacity_deadline(state, deadline),
        "report downloads",
        state.config.reports.capacity_wait_secs,
    )
    .await
}

async fn acquire_capacity(
    semaphore: Arc<tokio::sync::Semaphore>,
    deadline: tokio::time::Instant,
    resource: &'static str,
    retry_after_secs: u64,
) -> Result<OwnedSemaphorePermit, CanonicalError> {
    tokio::time::timeout_at(deadline, semaphore.acquire_owned())
        .await
        .map_err(|_| report_busy(resource, retry_after_secs))?
        .map_err(|_| report_busy(resource, retry_after_secs))
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
    telemetry: &ReportTelemetry,
) -> Result<Response<Body>, CanonicalError> {
    let artifact_path = artifact.path().map_err(map_export_error)?.to_path_buf();
    let file = tokio::fs::File::open(&artifact_path)
        .await
        .map_err(|_| report_internal("report artifact could not be opened"))?;
    let (_, content_length) = artifact.disarm().map_err(map_export_error)?;
    if tokio::fs::remove_file(&artifact_path).await.is_err() {
        drop(file);
        let removed = tokio::fs::remove_file(&artifact_path).await.is_ok();
        telemetry.record_cleanup(if removed {
            ReportCleanupOutcome::Removed
        } else {
            ReportCleanupOutcome::Failed
        });
        return Err(report_internal("report artifact could not be removed"));
    }
    telemetry.record_cleanup(ReportCleanupOutcome::Removed);

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
        ReportExportError::Xlsx(
            crate::domain::reports::xlsx::ReportXlsxError::SpoolLimitExceeded,
        ) => ReportError::invalid_argument()
            .with_field_violation(
                "format",
                "report exceeds the XLSX temporary serialization limit",
                "LIMIT_EXCEEDED",
            )
            .create(),
        error => {
            tracing::error!(?error, "report export failed");
            report_internal("report export failed")
        }
    }
}

fn report_busy(resource: &str, retry_after_secs: u64) -> CanonicalError {
    ReportError::resource_exhausted("Report export capacity is busy.")
        .with_quota_violation(resource, "concurrency limit reached")
        .with_quota_violation_retry_after_seconds(retry_after_secs)
        .create()
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use super::*;

    #[tokio::test]
    async fn responses_stream_complete_csv_and_xlsx_artifacts_then_release_capacity() {
        for (format, extension, content_type) in [
            (ReportExportFormat::Csv, "csv", "text/csv; charset=utf-8"),
            (
                ReportExportFormat::Xlsx,
                "xlsx",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ),
        ] {
            let temp_dir = tempfile::tempdir()
                .unwrap_or_else(|error| panic!("temporary directory must create: {error}"));
            let path = temp_dir.path().join(format!(
                "report-download-{}.{}",
                uuid::Uuid::new_v4(),
                extension
            ));
            tokio::fs::write(&path, b"report")
                .await
                .unwrap_or_else(|error| panic!("artifact must write: {error}"));
            let artifact = ReportArtifact::from_completed(path.clone(), 6);
            let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
            let permit = Arc::clone(&semaphore)
                .acquire_owned()
                .await
                .unwrap_or_else(|_| panic!("permit must acquire"));
            let filename = format!("insight-report_people_day_2026-01-01_2026-01-01.{extension}");
            let telemetry = ReportTelemetry::new(
                &crate::domain::reports::dto::ReportSubject::People {
                    ids: vec![uuid::Uuid::from_u128(1)],
                },
                format,
            );

            let response =
                response_for_artifact(artifact, permit, filename.clone(), format, &telemetry)
                    .await
                    .unwrap_or_else(|error| panic!("response must build: {error}"));

            assert!(!path.exists());
            assert_eq!(response.headers()[CONTENT_LENGTH], "6");
            assert_eq!(response.headers()[CONTENT_TYPE], content_type);
            assert_eq!(
                response.headers()[CONTENT_DISPOSITION],
                format!("attachment; filename=\"{filename}\"")
            );
            assert_eq!(semaphore.available_permits(), 0);
            let body = axum::body::to_bytes(response.into_body(), 6)
                .await
                .unwrap_or_else(|error| panic!("artifact body must stream: {error}"));
            assert_eq!(body.as_ref(), b"report");
            assert_eq!(semaphore.available_permits(), 1);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn capacity_wait_returns_resource_exhausted_at_deadline() {
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let _held = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .unwrap_or_else(|_| panic!("first permit must acquire"));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);

        let result = acquire_capacity(semaphore, deadline, "report generations", 2).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn sink_failure_returns_the_writer_limit_error() {
        let writer = tokio::spawn(async {
            Err(ReportExportError::Csv(
                crate::domain::reports::csv::ReportCsvError::GeneratedByteLimit { limit: 1 },
            ))
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);

        let error = map_execution_failure(
            ReportExecutionError::Sink(crate::domain::reports::executor::ReportSinkError::new(
                "report writer stopped",
            )),
            writer,
            deadline,
        )
        .await;

        assert_eq!(
            error.into_response().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}

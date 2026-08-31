use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, Meter};

use super::dto::{ReportExportFormat, ReportSubject};

#[derive(Debug, Clone, Copy)]
pub(crate) enum ReportSubjectType {
    People,
    Tenant,
}

impl ReportSubjectType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::People => "people",
            Self::Tenant => "tenant",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ReportOutcome {
    Success,
    Error,
    Cancelled,
}

impl ReportOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ReportCleanupOutcome {
    Removed,
    Failed,
}

impl ReportCleanupOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug)]
struct Instruments {
    completions: Counter<u64>,
    durations: Histogram<f64>,
    rows: Histogram<u64>,
    columns: Histogram<u64>,
    bytes: Histogram<u64>,
    cleanup: Counter<u64>,
}

fn instruments() -> &'static Instruments {
    static INSTRUMENTS: OnceLock<Instruments> = OnceLock::new();
    INSTRUMENTS.get_or_init(|| {
        let meter: Meter = opentelemetry::global::meter("analytics.reports");
        Instruments {
            completions: meter
                .u64_counter("analytics.reports.completed")
                .with_description("Completed report exports by terminal outcome.")
                .build(),
            durations: meter
                .f64_histogram("analytics.reports.duration")
                .with_unit("s")
                .with_description("Report export stage duration.")
                .build(),
            rows: meter
                .u64_histogram("analytics.reports.rows")
                .with_description("Rows in completed report exports.")
                .build(),
            columns: meter
                .u64_histogram("analytics.reports.columns")
                .with_description("Columns in completed report exports.")
                .build(),
            bytes: meter
                .u64_histogram("analytics.reports.bytes")
                .with_unit("By")
                .with_description("Bytes in completed report exports.")
                .build(),
            cleanup: meter
                .u64_counter("analytics.reports.cleanup")
                .with_description("Completed report artifact cleanup attempts.")
                .build(),
        }
    })
}

#[derive(Debug, Clone)]
pub(crate) struct ReportTelemetry {
    started_at: Instant,
    subject_type: ReportSubjectType,
    format: ReportExportFormat,
    finished: std::sync::Arc<AtomicBool>,
}

impl ReportTelemetry {
    pub(crate) fn new(subject: &ReportSubject, format: ReportExportFormat) -> Self {
        let subject_type = match subject {
            ReportSubject::People { .. } => ReportSubjectType::People,
            ReportSubject::Tenant {} => ReportSubjectType::Tenant,
        };

        Self {
            started_at: Instant::now(),
            subject_type,
            format,
            finished: std::sync::Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn record_admission_wait(&self, duration: Duration) {
        self.record_duration("admission", duration);
    }

    pub(crate) fn record_identity_duration(&self, duration: Duration) {
        self.record_duration("identity", duration);
    }

    pub(crate) fn record_query_duration(&self, duration: Duration) {
        self.record_duration("query", duration);
    }

    pub(crate) fn record_serialization_duration(&self, duration: Duration) {
        self.record_duration("serialization", duration);
    }

    pub(crate) fn record_cleanup(&self, outcome: ReportCleanupOutcome) {
        instruments().cleanup.add(
            1,
            &[
                KeyValue::new("subject_type", self.subject_type.as_str()),
                KeyValue::new("format", self.format.as_str()),
                KeyValue::new("outcome", outcome.as_str()),
            ],
        );
    }

    pub(crate) fn succeed(&self, rows: u64, columns: u64, bytes: u64) {
        self.finish(ReportOutcome::Success, rows, columns, bytes);
    }

    pub(crate) fn fail(&self) {
        self.finish(ReportOutcome::Error, 0, 0, 0);
    }

    fn finish(&self, outcome: ReportOutcome, rows: u64, columns: u64, bytes: u64) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }

        let attributes = [
            KeyValue::new("subject_type", self.subject_type.as_str()),
            KeyValue::new("format", self.format.as_str()),
            KeyValue::new("outcome", outcome.as_str()),
        ];
        instruments().completions.add(1, &attributes);
        instruments().durations.record(
            self.started_at.elapsed().as_secs_f64(),
            &self.stage_attributes("total"),
        );
        instruments().rows.record(rows, &attributes);
        instruments().columns.record(columns, &attributes);
        instruments().bytes.record(bytes, &attributes);

        match outcome {
            ReportOutcome::Success => tracing::info!(
                event = "report_export_completed",
                outcome = outcome.as_str(),
                subject_type = self.subject_type.as_str(),
                format = self.format.as_str(),
                rows,
                columns,
                bytes,
                "report export completed"
            ),
            ReportOutcome::Error | ReportOutcome::Cancelled => tracing::warn!(
                event = "report_export_completed",
                outcome = outcome.as_str(),
                subject_type = self.subject_type.as_str(),
                format = self.format.as_str(),
                "report export did not complete"
            ),
        }
    }

    fn record_duration(&self, stage: &'static str, duration: Duration) {
        instruments()
            .durations
            .record(duration.as_secs_f64(), &self.stage_attributes(stage));
    }

    fn stage_attributes(&self, stage: &'static str) -> [KeyValue; 3] {
        [
            KeyValue::new("subject_type", self.subject_type.as_str()),
            KeyValue::new("format", self.format.as_str()),
            KeyValue::new("stage", stage),
        ]
    }
}

impl Drop for ReportTelemetry {
    fn drop(&mut self) {
        if std::sync::Arc::strong_count(&self.finished) == 1
            && !self.finished.swap(true, Ordering::AcqRel)
        {
            let attributes = [
                KeyValue::new("subject_type", self.subject_type.as_str()),
                KeyValue::new("format", self.format.as_str()),
                KeyValue::new("outcome", ReportOutcome::Cancelled.as_str()),
            ];
            instruments().completions.add(1, &attributes);
            instruments().durations.record(
                self.started_at.elapsed().as_secs_f64(),
                &self.stage_attributes("total"),
            );
            tracing::warn!(
                event = "report_export_completed",
                outcome = ReportOutcome::Cancelled.as_str(),
                subject_type = self.subject_type.as_str(),
                format = self.format.as_str(),
                "report export was cancelled"
            );
        }
    }
}

impl ReportExportFormat {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_fixed_report_dimensions() {
        assert_eq!(ReportSubjectType::People.as_str(), "people");
        assert_eq!(ReportSubjectType::Tenant.as_str(), "tenant");
        assert_eq!(ReportOutcome::Success.as_str(), "success");
        assert_eq!(ReportOutcome::Error.as_str(), "error");
        assert_eq!(ReportOutcome::Cancelled.as_str(), "cancelled");
        assert_eq!(ReportCleanupOutcome::Removed.as_str(), "removed");
        assert_eq!(ReportCleanupOutcome::Failed.as_str(), "failed");
    }
}

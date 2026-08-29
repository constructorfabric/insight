use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::columns::PlannedColumn;
use super::csv::{ReportCsvError, ReportCsvWriter};
use super::dto::ReportExportFormat;
use super::executor::{ReportRowSink, ReportSinkError};
use super::planner::{ReportPlan, XlsxDimensions};
use super::row::ReportRow;
use super::telemetry::ReportTelemetry;
use super::xlsx::{ReportXlsxError, ReportXlsxSerializer};

const ROWS_PER_MESSAGE: usize = 512;
const FILE_CREATE_ATTEMPTS: usize = 4;

#[derive(Debug)]
pub(crate) struct ReportArtifact {
    path: Option<PathBuf>,
    content_length: u64,
    telemetry: Option<ReportTelemetry>,
}

impl ReportArtifact {
    #[cfg(test)]
    pub(crate) fn from_completed(path: PathBuf, content_length: u64) -> Self {
        Self {
            path: Some(path),
            content_length,
            telemetry: None,
        }
    }

    pub(crate) fn path(&self) -> Result<&Path, ReportExportError> {
        self.path.as_deref().ok_or(ReportExportError::WriterStopped)
    }

    pub(crate) fn content_length(&self) -> u64 {
        self.content_length
    }

    pub(crate) fn disarm(mut self) -> Result<(PathBuf, u64), ReportExportError> {
        let path = self.path.take().ok_or(ReportExportError::WriterStopped)?;
        Ok((path, self.content_length))
    }
}

impl Drop for ReportArtifact {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            remove_artifact(path, self.telemetry.take());
        }
    }
}

fn remove_artifact(path: PathBuf, telemetry: Option<ReportTelemetry>) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(move || {
            record_cleanup(std::fs::remove_file(path).is_ok(), telemetry.as_ref());
        });
    } else {
        record_cleanup(std::fs::remove_file(path).is_ok(), telemetry.as_ref());
    }
}

fn record_cleanup(removed: bool, telemetry: Option<&ReportTelemetry>) {
    let Some(telemetry) = telemetry else {
        return;
    };
    telemetry.record_cleanup(if removed {
        super::telemetry::ReportCleanupOutcome::Removed
    } else {
        super::telemetry::ReportCleanupOutcome::Failed
    });
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReportExportError {
    #[error("report temporary directory is unavailable")]
    TemporaryDirectory(#[source] std::io::Error),
    #[error("report temporary file could not be created")]
    TemporaryFile(#[source] std::io::Error),
    #[error("report CSV serialization failed")]
    Csv(#[source] ReportCsvError),
    #[error("report XLSX serialization failed")]
    Xlsx(#[source] ReportXlsxError),
    #[error("report writer stopped before completion")]
    WriterStopped,
    #[error("report exceeds XLSX dimensions")]
    XlsxDimensions,
    #[error("report output could not be finalized")]
    Finalize(#[source] std::io::Error),
}

#[derive(Debug)]
pub(crate) struct ReportExportSink {
    sender: mpsc::Sender<WriterMessage>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ReportWriterLimits {
    pub(crate) max_generated_bytes: usize,
    pub(crate) max_xlsx_spool_bytes: usize,
    pub(crate) channel_batches: usize,
}

pub(crate) fn start_report_writer(
    format: ReportExportFormat,
    plan: &ReportPlan,
    temp_dir: PathBuf,
    limits: ReportWriterLimits,
    telemetry: ReportTelemetry,
    generation_permit: OwnedSemaphorePermit,
) -> Result<
    (
        ReportExportSink,
        JoinHandle<Result<ReportArtifact, ReportExportError>>,
    ),
    ReportExportError,
> {
    let dimensions = match format {
        ReportExportFormat::Csv => None,
        ReportExportFormat::Xlsx => Some(
            plan.size
                .xlsx_dimensions()
                .map_err(|_| ReportExportError::XlsxDimensions)?,
        ),
    };
    let specification = WriterSpecification {
        format,
        columns: plan.columns.clone(),
        dimensions,
        temp_dir,
        max_generated_bytes: limits.max_generated_bytes,
        max_xlsx_spool_bytes: limits.max_xlsx_spool_bytes,
        telemetry,
    };
    let (sender, receiver) = mpsc::channel(limits.channel_batches);
    let task = spawn_report_writer(specification, receiver, generation_permit);

    Ok((ReportExportSink { sender }, task))
}

fn spawn_report_writer(
    specification: WriterSpecification,
    receiver: mpsc::Receiver<WriterMessage>,
    generation_permit: OwnedSemaphorePermit,
) -> JoinHandle<Result<ReportArtifact, ReportExportError>> {
    tokio::task::spawn_blocking(move || {
        let _generation_permit = generation_permit;
        write_report(&specification, receiver)
    })
}

#[async_trait]
impl ReportRowSink for ReportExportSink {
    type Output = ();

    async fn write_rows(&mut self, rows: &[ReportRow]) -> Result<(), ReportSinkError> {
        for rows in rows.chunks(ROWS_PER_MESSAGE) {
            self.sender
                .send(WriterMessage::Rows(rows.to_vec()))
                .await
                .map_err(|_| ReportSinkError::new("report writer stopped"))?;
        }
        Ok(())
    }

    async fn finish(self) -> Result<Self::Output, ReportSinkError> {
        self.sender
            .send(WriterMessage::Finish)
            .await
            .map_err(|_| ReportSinkError::new("report writer stopped"))?;
        Ok(())
    }
}

#[derive(Debug)]
enum WriterMessage {
    Rows(Vec<ReportRow>),
    Finish,
}

#[derive(Debug)]
struct WriterSpecification {
    format: ReportExportFormat,
    columns: Vec<PlannedColumn>,
    dimensions: Option<XlsxDimensions>,
    temp_dir: PathBuf,
    max_generated_bytes: usize,
    max_xlsx_spool_bytes: usize,
    telemetry: ReportTelemetry,
}

fn write_report(
    specification: &WriterSpecification,
    mut receiver: mpsc::Receiver<WriterMessage>,
) -> Result<ReportArtifact, ReportExportError> {
    std::fs::create_dir_all(&specification.temp_dir)
        .map_err(ReportExportError::TemporaryDirectory)?;

    let (file, path) = create_file(&specification.temp_dir, specification.format)?;
    let artifact = TemporaryArtifact {
        path: Some(path),
        telemetry: specification.telemetry.clone(),
    };
    let mut writer = FileWriter::new(file, specification)?;
    let mut completed = false;

    while let Some(message) = receiver.blocking_recv() {
        match message {
            WriterMessage::Rows(rows) => {
                let started_at = std::time::Instant::now();
                let result = writer.write_rows(&rows);
                specification
                    .telemetry
                    .record_serialization_duration(started_at.elapsed());
                result?;
            }
            WriterMessage::Finish => {
                completed = true;
                break;
            }
        }
    }
    if !completed {
        return Err(ReportExportError::WriterStopped);
    }

    let started_at = std::time::Instant::now();
    let result = writer.finish();
    specification
        .telemetry
        .record_serialization_duration(started_at.elapsed());
    result?;
    let path = artifact
        .path
        .as_ref()
        .ok_or(ReportExportError::WriterStopped)?;
    let content_length = std::fs::metadata(path)
        .map_err(ReportExportError::Finalize)?
        .len();
    let path = artifact.persist()?;

    Ok(ReportArtifact {
        path: Some(path),
        content_length,
        telemetry: Some(specification.telemetry.clone()),
    })
}

fn create_file(
    temp_dir: &Path,
    format: ReportExportFormat,
) -> Result<(File, PathBuf), ReportExportError> {
    let extension = match format {
        ReportExportFormat::Csv => "csv",
        ReportExportFormat::Xlsx => "xlsx",
    };
    for _ in 0..FILE_CREATE_ATTEMPTS {
        let path = temp_dir.join(format!("{}.{}", Uuid::new_v4(), extension));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(ReportExportError::TemporaryFile(error)),
        }
    }

    Err(ReportExportError::TemporaryFile(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "report temporary file name collision",
    )))
}

#[derive(Debug)]
struct TemporaryArtifact {
    path: Option<PathBuf>,
    telemetry: ReportTelemetry,
}

impl TemporaryArtifact {
    fn persist(mut self) -> Result<PathBuf, ReportExportError> {
        self.path.take().ok_or(ReportExportError::WriterStopped)
    }
}

impl Drop for TemporaryArtifact {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            record_cleanup(std::fs::remove_file(path).is_ok(), Some(&self.telemetry));
        }
    }
}

enum FileWriter {
    Csv(Box<ReportCsvWriter<File>>),
    Xlsx {
        serializer: Box<ReportXlsxSerializer>,
        file: File,
        max_generated_bytes: usize,
    },
}

impl FileWriter {
    fn new(file: File, specification: &WriterSpecification) -> Result<Self, ReportExportError> {
        match specification.format {
            ReportExportFormat::Csv => ReportCsvWriter::new(
                file,
                &specification.columns,
                specification.max_generated_bytes,
            )
            .map(|writer| Self::Csv(Box::new(writer)))
            .map_err(ReportExportError::Csv),
            ReportExportFormat::Xlsx => ReportXlsxSerializer::new(
                &specification.columns,
                specification
                    .dimensions
                    .ok_or(ReportExportError::WriterStopped)?,
                &specification.temp_dir,
                specification.max_xlsx_spool_bytes,
            )
            .map(|serializer| Self::Xlsx {
                serializer: Box::new(serializer),
                file,
                max_generated_bytes: specification.max_generated_bytes,
            })
            .map_err(ReportExportError::Xlsx),
        }
    }

    fn write_rows(&mut self, rows: &[ReportRow]) -> Result<(), ReportExportError> {
        match self {
            Self::Csv(writer) => writer.write_batch(rows).map_err(ReportExportError::Csv),
            Self::Xlsx { serializer, .. } => {
                serializer.write_rows(rows).map_err(ReportExportError::Xlsx)
            }
        }
    }

    fn finish(self) -> Result<(), ReportExportError> {
        match self {
            Self::Csv(writer) => {
                let _file = writer.finish().map_err(ReportExportError::Csv)?;
                Ok(())
            }
            Self::Xlsx {
                serializer,
                file,
                max_generated_bytes,
            } => serializer
                .finish_into(file, max_generated_bytes)
                .map_err(ReportExportError::Xlsx),
        }
    }
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;

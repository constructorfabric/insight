use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::columns::PlannedColumn;
use super::csv::{ReportCsvError, ReportCsvWriter};
use super::dto::ReportExportFormat;
use super::executor::{ReportRowSink, ReportSinkError};
use super::planner::{ReportPlan, XlsxDimensions};
use super::row::ReportRow;
use super::xlsx::{ReportXlsxError, ReportXlsxSerializer};

const ROWS_PER_MESSAGE: usize = 512;
const FILE_CREATE_ATTEMPTS: usize = 4;

#[derive(Debug)]
pub(crate) struct ReportArtifact {
    path: Option<PathBuf>,
    content_length: u64,
}

impl ReportArtifact {
    #[cfg(test)]
    pub(crate) fn from_completed(path: PathBuf, content_length: u64) -> Self {
        Self {
            path: Some(path),
            content_length,
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
        if let Some(path) = &self.path {
            remove_artifact(path.clone());
        }
    }
}

fn remove_artifact(path: PathBuf) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(move || {
            let _ = std::fs::remove_file(path);
        });
    } else {
        let _ = std::fs::remove_file(path);
    }
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

pub(crate) fn start_report_writer(
    format: ReportExportFormat,
    plan: &ReportPlan,
    temp_dir: PathBuf,
    max_generated_bytes: usize,
    channel_batches: usize,
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
        max_generated_bytes,
    };
    let (sender, receiver) = mpsc::channel(channel_batches);
    let task = tokio::task::spawn_blocking(move || write_report(&specification, receiver));

    Ok((ReportExportSink { sender }, task))
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
}

fn write_report(
    specification: &WriterSpecification,
    mut receiver: mpsc::Receiver<WriterMessage>,
) -> Result<ReportArtifact, ReportExportError> {
    std::fs::create_dir_all(&specification.temp_dir)
        .map_err(ReportExportError::TemporaryDirectory)?;

    let (file, path) = create_file(&specification.temp_dir, specification.format)?;
    let artifact = TemporaryArtifact { path: Some(path) };
    let mut writer = FileWriter::new(file, specification)?;
    let mut completed = false;

    while let Some(message) = receiver.blocking_recv() {
        match message {
            WriterMessage::Rows(rows) => writer.write_rows(&rows)?,
            WriterMessage::Finish => {
                completed = true;
                break;
            }
        }
    }
    if !completed {
        return Err(ReportExportError::WriterStopped);
    }

    writer.finish()?;
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
}

impl TemporaryArtifact {
    fn persist(mut self) -> Result<PathBuf, ReportExportError> {
        self.path.take().ok_or(ReportExportError::WriterStopped)
    }
}

impl Drop for TemporaryArtifact {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
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
mod tests {
    use super::*;
    use crate::domain::reports::columns::{
        PlannedColumnSource, ReportColumnDataType, ReportColumnMetadata,
    };

    #[tokio::test]
    async fn writer_removes_incomplete_artifact() {
        let temp_dir = temporary_directory();
        let specification = specification(ReportExportFormat::Csv, temp_dir.clone());
        let (sender, receiver) = mpsc::channel(1);
        let task = tokio::task::spawn_blocking(move || write_report(&specification, receiver));

        drop(sender);
        let result = task
            .await
            .unwrap_or_else(|error| panic!("writer task must join: {error}"));

        assert!(matches!(result, Err(ReportExportError::WriterStopped)));
        assert!(
            std::fs::read_dir(&temp_dir)
                .unwrap_or_else(|error| panic!("temporary directory must remain readable: {error}"))
                .next()
                .is_none()
        );
        std::fs::remove_dir(&temp_dir)
            .unwrap_or_else(|error| panic!("temporary directory must remove: {error}"));
    }

    #[tokio::test]
    async fn writer_completes_csv_before_exposing_artifact() {
        let temp_dir = temporary_directory();
        let specification = specification(ReportExportFormat::Csv, temp_dir.clone());
        let (sender, receiver) = mpsc::channel(1);
        let task = tokio::task::spawn_blocking(move || write_report(&specification, receiver));

        sender
            .send(WriterMessage::Rows(vec![vec![Some(
                super::super::row::ReportCell::Text("example".to_owned()),
            )]]))
            .await
            .unwrap_or_else(|_| panic!("writer must accept rows"));
        sender
            .send(WriterMessage::Finish)
            .await
            .unwrap_or_else(|_| panic!("writer must accept completion"));
        drop(sender);

        let artifact = task
            .await
            .unwrap_or_else(|error| panic!("writer task must join: {error}"))
            .unwrap_or_else(|error| panic!("writer must finish: {error}"));
        let output = std::fs::read(
            artifact
                .path()
                .unwrap_or_else(|error| panic!("artifact path must exist: {error}")),
        )
        .unwrap_or_else(|error| panic!("artifact must be readable: {error}"));

        assert_eq!(
            artifact.content_length(),
            u64::try_from(output.len()).unwrap_or(u64::MAX)
        );
        assert!(output.ends_with(b"example\r\n"));
        std::fs::remove_file(
            artifact
                .path()
                .unwrap_or_else(|error| panic!("artifact path must exist: {error}")),
        )
        .unwrap_or_else(|error| panic!("artifact must remove: {error}"));
        std::fs::remove_dir(&temp_dir)
            .unwrap_or_else(|error| panic!("temporary directory must remove: {error}"));
    }

    #[test]
    fn artifact_drop_removes_unclaimed_file() {
        let path = std::env::temp_dir().join(format!("report-orphan-{}", Uuid::new_v4()));
        std::fs::write(&path, b"report")
            .unwrap_or_else(|error| panic!("artifact must write: {error}"));

        let artifact = ReportArtifact::from_completed(path.clone(), 6);
        drop(artifact);

        assert!(!path.exists());
    }

    fn specification(format: ReportExportFormat, temp_dir: PathBuf) -> WriterSpecification {
        WriterSpecification {
            format,
            columns: vec![PlannedColumn {
                metadata: ReportColumnMetadata {
                    key: "name".to_owned(),
                    label: "Name".to_owned(),
                    data_type: ReportColumnDataType::Text,
                    format: None,
                    unit: None,
                },
                source: PlannedColumnSource::PersonDisplay,
            }],
            dimensions: Some(XlsxDimensions {
                rows: 2,
                columns: 1,
            }),
            temp_dir,
            max_generated_bytes: 1024,
        }
    }

    fn temporary_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!("reports-export-test-{}", Uuid::new_v4()));
        std::fs::create_dir(&path)
            .unwrap_or_else(|error| panic!("temporary directory must create: {error}"));
        path
    }
}

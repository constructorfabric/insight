use std::path::PathBuf;

use super::*;
use crate::domain::reports::columns::{
    PlannedColumn, PlannedColumnSource, ReportColumnDataType, ReportColumnMetadata,
};
use crate::domain::reports::dto::{ReportExportFormat, ReportSubject};
use crate::domain::reports::planner::XlsxDimensions;
use crate::domain::reports::row::ReportCell;
use crate::domain::reports::telemetry::ReportTelemetry;
use tokio::sync::mpsc;

#[tokio::test]
async fn writer_removes_incomplete_artifact() {
    let temporary_directory = temporary_directory();
    let temp_dir = temporary_directory.path().to_path_buf();
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
}

#[tokio::test]
async fn blocking_writer_holds_generation_capacity_until_it_stops() {
    let temporary_directory = temporary_directory();
    let temp_dir = temporary_directory.path().to_path_buf();
    let specification = specification(ReportExportFormat::Csv, temp_dir.clone());
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
    let permit = std::sync::Arc::clone(&semaphore)
        .acquire_owned()
        .await
        .unwrap_or_else(|_| panic!("generation permit must acquire"));
    let (sender, receiver) = mpsc::channel(1);
    let task = spawn_report_writer(specification, receiver, permit);

    assert_eq!(semaphore.available_permits(), 0);
    drop(sender);
    let result = task
        .await
        .unwrap_or_else(|error| panic!("writer task must join: {error}"));

    assert!(matches!(result, Err(ReportExportError::WriterStopped)));
    assert_eq!(semaphore.available_permits(), 1);
}

#[tokio::test]
async fn writer_completes_csv_before_exposing_artifact() {
    let temporary_directory = temporary_directory();
    let temp_dir = temporary_directory.path().to_path_buf();
    let specification = specification(ReportExportFormat::Csv, temp_dir.clone());
    let (sender, receiver) = mpsc::channel(1);
    let task = tokio::task::spawn_blocking(move || write_report(&specification, receiver));

    sender
        .send(WriterMessage::Rows(vec![
            vec![Some(ReportCell::Text("example".to_owned()))].into(),
        ]))
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
}

#[test]
fn artifact_drop_removes_unclaimed_file() {
    let fixture = tempfile::NamedTempFile::new()
        .unwrap_or_else(|error| panic!("artifact must create: {error}"));
    let (file, path) = fixture
        .keep()
        .unwrap_or_else(|error| panic!("artifact must persist: {error}"));
    drop(file);
    std::fs::write(&path, b"report").unwrap_or_else(|error| panic!("artifact must write: {error}"));

    let artifact = ReportArtifact::from_completed(path.clone(), 6);
    drop(artifact);

    assert!(!path.exists());
}

#[test]
fn writer_rejects_a_temporary_path_that_is_not_a_directory() {
    let fixture = tempfile::NamedTempFile::new()
        .unwrap_or_else(|error| panic!("fixture must create: {error}"));
    let path = fixture.path().to_path_buf();
    std::fs::write(&path, b"not a directory")
        .unwrap_or_else(|error| panic!("fixture must write: {error}"));
    let specification = specification(ReportExportFormat::Csv, path.clone());
    let (_sender, receiver) = mpsc::channel(1);

    let result = write_report(&specification, receiver);

    assert!(matches!(
        result,
        Err(ReportExportError::TemporaryDirectory(_))
    ));
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
        max_xlsx_spool_bytes: usize::MAX,
        telemetry: ReportTelemetry::new(&ReportSubject::Tenant {}, ReportExportFormat::Csv),
    }
}

fn temporary_directory() -> tempfile::TempDir {
    tempfile::tempdir().unwrap_or_else(|error| panic!("temporary directory must create: {error}"))
}

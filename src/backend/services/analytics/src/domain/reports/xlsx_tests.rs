use std::io::{Cursor, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::domain::metric_definitions::MetricFormat;
use crate::domain::reports::columns::{
    PlannedColumn, PlannedColumnSource, ReportColumnDataType, ReportColumnMetadata,
};
use crate::domain::reports::planner::XlsxDimensions;
use crate::domain::reports::row::{ReportCell, ReportRow};

#[test]
fn writes_typed_cells_nulls_and_dates_in_row_order() {
    let columns = vec![
        column("=Name", ReportColumnDataType::Text),
        column("From", ReportColumnDataType::Date),
        column("Metric", ReportColumnDataType::Number),
        column("Blank metric", ReportColumnDataType::Number),
    ];
    let bytes = write_workbook(
        &columns,
        XlsxDimensions {
            rows: 2,
            columns: 4,
        },
        &[vec![
            Some(ReportCell::Text("=formula".to_owned())),
            Some(ReportCell::Text("2026-01-02".to_owned())),
            Some(ReportCell::Number(12.5)),
            None,
        ]
        .into()],
        usize::MAX,
    )
    .unwrap_or_else(|error| panic!("workbook must serialize: {error}"));

    let xml = worksheet_xml(&bytes);
    assert!(xml.contains("r=\"A1\" s=\""));
    assert!(xml.contains("r=\"A2\" t=\"inlineStr\""));
    assert!(xml.contains("<t>=formula</t>"));
    assert!(xml.contains("r=\"B2\" s=\""));
    assert!(xml.contains("<v>46024</v>"));
    assert!(xml.contains("r=\"C2\"><v>12.5</v>"));
    let blank_cell = xml.split("<c ").find(|cell| cell.starts_with("r=\"D2\""));
    assert!(blank_cell.is_none_or(|cell| !cell.contains("<v>")));
    assert!(xml.contains("<pane ySplit=\"1\" topLeftCell=\"A2\""));
    assert!(xml.contains("<autoFilter ref=\"A1:D2\"/>"));
    assert!(xml.contains("<cols>"));
}

#[test]
fn supports_more_than_fifty_metrics() {
    let columns = (0..64)
        .map(|index| column(&format!("Metric {index}"), ReportColumnDataType::Number))
        .collect::<Vec<_>>();
    let row = (0..64)
        .map(|index| Some(ReportCell::Number(f64::from(index))))
        .collect::<ReportRow>();
    let bytes = write_workbook(
        &columns,
        XlsxDimensions {
            rows: 2,
            columns: 64,
        },
        &[row],
        usize::MAX,
    )
    .unwrap_or_else(|error| panic!("workbook must serialize: {error}"));

    let xml = worksheet_xml(&bytes);
    assert!(xml.contains("r=\"BL1\" s=\""));
    assert!(xml.contains("r=\"BL2\"><v>63</v>"));
}

#[test]
fn applies_metric_number_formats_without_rounding_stored_values() {
    let formats = [
        MetricFormat::Integer,
        MetricFormat::Decimal,
        MetricFormat::Currency,
        MetricFormat::Percent,
    ];
    let columns = formats
        .into_iter()
        .enumerate()
        .map(|(index, format)| formatted_column(&format!("Metric {index}"), format))
        .collect::<Vec<_>>();
    let bytes = write_workbook(
        &columns,
        XlsxDimensions {
            rows: 2,
            columns: 4,
        },
        &[vec![
            Some(ReportCell::Number(1.234_567_890_123_456_7)),
            Some(ReportCell::Number(2.5)),
            Some(ReportCell::Number(3.75)),
            Some(ReportCell::Number(42.4)),
        ]
        .into()],
        usize::MAX,
    )
    .unwrap_or_else(|error| panic!("workbook must serialize: {error}"));

    let worksheet = worksheet_xml(&bytes);
    let styles = archive_text(&bytes, "xl/styles.xml");
    assert!(worksheet.contains("<v>1.2345678901234567</v>"));
    assert!(styles.contains("formatCode=\"#,##0.00\""));
    assert!(styles.contains("formatCode=\"$#,##0\""));
    assert!(styles.contains("formatCode=\"#,##0&quot;%&quot;\""));
}

#[test]
fn caps_generated_output_bytes() {
    let output_limit = Arc::new(AtomicBool::new(false));
    let mut writer = CappedWriter {
        inner: Vec::new(),
        limit: 3,
        written: 0,
        output_limit: Arc::clone(&output_limit),
    };

    writer
        .write_all(b"123")
        .unwrap_or_else(|error| panic!("bytes within cap must write: {error}"));
    assert!(writer.write_all(b"4").is_err());
    assert!(output_limit.load(Ordering::Relaxed));
}

#[test]
fn rejects_oversized_cells_and_incomplete_dimensions() {
    let columns = vec![column("Metric", ReportColumnDataType::Text)];
    let temp_dir = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("temporary directory must create: {error}"));
    let mut serializer = ReportXlsxSerializer::new(
        &columns,
        XlsxDimensions {
            rows: 3,
            columns: 1,
        },
        temp_dir.path(),
        usize::MAX,
    )
    .unwrap_or_else(|error| panic!("writer must initialize: {error}"));
    let oversized_row = vec![
        vec![Some(ReportCell::Text(
            "x".repeat(MAX_REPORT_XLSX_CELL_CHARACTERS + 1),
        ))]
        .into(),
    ];
    assert!(matches!(
        serializer.write_rows(&oversized_row),
        Err(ReportXlsxError::CellTooLarge)
    ));
    assert!(matches!(
        serializer.finish_into(Vec::new(), usize::MAX),
        Err(ReportXlsxError::IncompleteRows)
    ));
}

#[test]
fn rejects_rows_before_their_estimated_spool_exceeds_the_budget() {
    let columns = vec![column("Text", ReportColumnDataType::Text)];
    let temp_dir = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("temporary directory must create: {error}"));
    let header_budget = estimated_text_row_spool_bytes(["Text"].into_iter())
        .unwrap_or_else(|| panic!("header estimate must fit"));
    let row = ReportRow::from(vec![Some(ReportCell::Text("compressible".repeat(32)))]);
    let row_budget =
        estimated_row_spool_bytes(&row).unwrap_or_else(|| panic!("row estimate must fit"));
    let mut serializer = ReportXlsxSerializer::new(
        &columns,
        XlsxDimensions {
            rows: 3,
            columns: 1,
        },
        temp_dir.path(),
        XLSX_SPOOL_FIXED_BYTES + header_budget + row_budget,
    )
    .unwrap_or_else(|error| panic!("writer must initialize: {error}"));

    serializer
        .write_rows(std::slice::from_ref(&row))
        .unwrap_or_else(|error| panic!("first row must fit: {error}"));
    assert!(matches!(
        serializer.write_rows(&[row]),
        Err(ReportXlsxError::SpoolLimitExceeded)
    ));
}

#[test]
fn accepts_excel_maximum_ascii_cell_length() {
    let columns = vec![column("Text", ReportColumnDataType::Text)];
    let bytes = write_workbook(
        &columns,
        XlsxDimensions {
            rows: 2,
            columns: 1,
        },
        &[vec![Some(ReportCell::Text(
            "x".repeat(MAX_REPORT_XLSX_CELL_CHARACTERS),
        ))]
        .into()],
        usize::MAX,
    )
    .unwrap_or_else(|error| panic!("maximum Excel string must serialize: {error}"));

    assert!(bytes.starts_with(b"PK"));
}

fn write_workbook(
    columns: &[PlannedColumn],
    dimensions: XlsxDimensions,
    rows: &[ReportRow],
    max_output_bytes: usize,
) -> Result<Vec<u8>, ReportXlsxError> {
    let temp_dir = tempfile::tempdir()
        .unwrap_or_else(|error| panic!("temporary directory must create: {error}"));
    let mut serializer =
        ReportXlsxSerializer::new(columns, dimensions, temp_dir.path(), usize::MAX)?;
    serializer.write_rows(rows)?;
    let mut bytes = Vec::new();
    serializer.finish_into(&mut bytes, max_output_bytes)?;
    Ok(bytes)
}

fn worksheet_xml(bytes: &[u8]) -> String {
    archive_text(bytes, "xl/worksheets/sheet1.xml")
}

fn archive_text(bytes: &[u8], path: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .unwrap_or_else(|error| panic!("workbook must reopen as ZIP: {error}"));
    let mut sheet = archive
        .by_name(path)
        .unwrap_or_else(|error| panic!("workbook must contain {path}: {error}"));
    let mut xml = String::new();
    sheet
        .read_to_string(&mut xml)
        .unwrap_or_else(|error| panic!("worksheet XML must be UTF-8: {error}"));
    xml
}

fn column(label: &str, data_type: ReportColumnDataType) -> PlannedColumn {
    PlannedColumn {
        metadata: ReportColumnMetadata {
            key: label.to_owned(),
            label: label.to_owned(),
            data_type,
            format: None,
            unit: None,
        },
        source: PlannedColumnSource::Metric(0),
    }
}

fn formatted_column(label: &str, format: MetricFormat) -> PlannedColumn {
    let mut column = column(label, ReportColumnDataType::Number);
    column.metadata.format = Some(format);
    column
}

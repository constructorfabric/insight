use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Datelike, NaiveDate};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook, XlsxError};

use super::columns::{PlannedColumn, ReportColumnDataType};
use super::planner::XlsxDimensions;
use super::row::{ReportCell, ReportRow};

pub(crate) const MAX_REPORT_XLSX_CELL_CHARACTERS: usize = 32_767;
const BYTE_LIMIT_MARKER: &str = "report XLSX byte limit exceeded";

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReportXlsxError {
    #[error("report XLSX dimensions do not match columns")]
    ColumnDimensionMismatch,
    #[error("report XLSX row count exceeds planned dimensions")]
    RowDimensionExceeded,
    #[error("report XLSX row count does not match planned dimensions")]
    IncompleteRows,
    #[error("report XLSX row does not match planned columns")]
    RowShapeMismatch,
    #[error("report XLSX cell exceeds byte limit")]
    CellTooLarge,
    #[error("report XLSX cell does not match its column type")]
    CellTypeMismatch,
    #[error("report XLSX contains a non-finite number")]
    NonFiniteNumber,
    #[error("report XLSX serialization exceeds byte limit")]
    OutputLimitExceeded,
    #[error("report XLSX serialization failed")]
    Serialization(#[source] XlsxError),
}

pub(crate) struct ReportXlsxSerializer {
    workbook: Workbook,
    column_types: Vec<ReportColumnDataType>,
    dimensions: XlsxDimensions,
    next_row: u32,
}

impl ReportXlsxSerializer {
    pub(crate) fn new(
        columns: &[PlannedColumn],
        dimensions: XlsxDimensions,
        temp_dir: &Path,
    ) -> Result<Self, ReportXlsxError> {
        validate_dimensions(columns, dimensions)?;
        validate_headers(columns)?;

        let mut workbook = Workbook::new();
        workbook
            .set_tempdir(temp_dir)
            .map_err(ReportXlsxError::Serialization)?;
        let worksheet = workbook.add_worksheet_with_constant_memory();
        for (column_index, column) in columns.iter().enumerate() {
            let column_index = u16::try_from(column_index)
                .map_err(|_| ReportXlsxError::ColumnDimensionMismatch)?;
            worksheet
                .write_string(0, column_index, &column.metadata.label)
                .map_err(ReportXlsxError::Serialization)?;
        }

        Ok(Self {
            workbook,
            column_types: columns
                .iter()
                .map(|column| column.metadata.data_type)
                .collect(),
            dimensions,
            next_row: 1,
        })
    }

    pub(crate) fn write_rows(&mut self, rows: &[ReportRow]) -> Result<(), ReportXlsxError> {
        self.validate_rows(rows)?;

        let worksheet = self
            .workbook
            .worksheet_from_index(0)
            .map_err(ReportXlsxError::Serialization)?;
        let date_format = Format::new().set_num_format("yyyy-mm-dd");
        let blank_format = Format::new();
        for row in rows {
            for (column_index, cell) in row.iter().enumerate() {
                let column_index = u16::try_from(column_index)
                    .map_err(|_| ReportXlsxError::ColumnDimensionMismatch)?;
                write_cell(
                    worksheet,
                    self.next_row,
                    column_index,
                    self.column_types[column_index as usize],
                    cell.as_ref(),
                    &date_format,
                    &blank_format,
                )?;
            }
            self.next_row = self
                .next_row
                .checked_add(1)
                .ok_or(ReportXlsxError::RowDimensionExceeded)?;
        }

        Ok(())
    }

    pub(crate) fn finish_into<W>(
        mut self,
        output: W,
        max_output_bytes: usize,
    ) -> Result<(), ReportXlsxError>
    where
        W: Write + Send,
    {
        if self.next_row != self.dimensions.rows {
            return Err(ReportXlsxError::IncompleteRows);
        }

        let output_limit = Arc::new(AtomicBool::new(false));
        let writer = CappedWriter {
            inner: output,
            limit: max_output_bytes,
            written: 0,
            output_limit: Arc::clone(&output_limit),
        };
        match self.workbook.save_to_writer(writer) {
            Ok(()) => Ok(()),
            Err(_error) if output_limit.load(Ordering::Relaxed) => {
                Err(ReportXlsxError::OutputLimitExceeded)
            }
            Err(error) => Err(ReportXlsxError::Serialization(error)),
        }
    }

    fn validate_rows(&self, rows: &[ReportRow]) -> Result<(), ReportXlsxError> {
        let row_count =
            u32::try_from(rows.len()).map_err(|_| ReportXlsxError::RowDimensionExceeded)?;
        let final_row = self
            .next_row
            .checked_add(row_count)
            .ok_or(ReportXlsxError::RowDimensionExceeded)?;
        if final_row > self.dimensions.rows {
            return Err(ReportXlsxError::RowDimensionExceeded);
        }

        for row in rows {
            if row.len() != self.column_types.len() {
                return Err(ReportXlsxError::RowShapeMismatch);
            }
            for (column_type, cell) in self.column_types.iter().zip(row) {
                validate_cell(*column_type, cell.as_ref())?;
            }
        }
        Ok(())
    }
}

fn validate_dimensions(
    columns: &[PlannedColumn],
    dimensions: XlsxDimensions,
) -> Result<(), ReportXlsxError> {
    let column_count =
        u16::try_from(columns.len()).map_err(|_| ReportXlsxError::ColumnDimensionMismatch)?;
    if dimensions.rows == 0 || dimensions.columns != column_count {
        return Err(ReportXlsxError::ColumnDimensionMismatch);
    }
    Ok(())
}

fn validate_headers(columns: &[PlannedColumn]) -> Result<(), ReportXlsxError> {
    if columns
        .iter()
        .any(|column| column.metadata.label.chars().count() > MAX_REPORT_XLSX_CELL_CHARACTERS)
    {
        return Err(ReportXlsxError::CellTooLarge);
    }
    Ok(())
}

fn validate_cell(
    column_type: ReportColumnDataType,
    cell: Option<&ReportCell>,
) -> Result<(), ReportXlsxError> {
    let Some(cell) = cell else {
        return Ok(());
    };
    match (column_type, cell) {
        (ReportColumnDataType::Text, ReportCell::Text(text)) => validate_text(text),
        (ReportColumnDataType::Date, ReportCell::Text(text)) => {
            validate_text(text)?;
            parse_date(text).map(|_| ())
        }
        (ReportColumnDataType::Number, ReportCell::Number(number)) if number.is_finite() => Ok(()),
        (ReportColumnDataType::Number, ReportCell::Number(_)) => {
            Err(ReportXlsxError::NonFiniteNumber)
        }
        (ReportColumnDataType::Text | ReportColumnDataType::Date, ReportCell::Number(_))
        | (ReportColumnDataType::Number, ReportCell::Text(_)) => {
            Err(ReportXlsxError::CellTypeMismatch)
        }
    }
}

fn validate_text(text: &str) -> Result<(), ReportXlsxError> {
    if text.chars().count() > MAX_REPORT_XLSX_CELL_CHARACTERS {
        return Err(ReportXlsxError::CellTooLarge);
    }
    Ok(())
}

fn write_cell(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    row: u32,
    column: u16,
    column_type: ReportColumnDataType,
    cell: Option<&ReportCell>,
    date_format: &Format,
    blank_format: &Format,
) -> Result<(), ReportXlsxError> {
    match cell {
        None => {
            worksheet
                .write_blank(row, column, blank_format)
                .map_err(ReportXlsxError::Serialization)?;
            Ok(())
        }
        Some(ReportCell::Text(text)) if column_type == ReportColumnDataType::Text => {
            worksheet
                .write_string(row, column, text)
                .map_err(ReportXlsxError::Serialization)?;
            Ok(())
        }
        Some(ReportCell::Text(text)) if column_type == ReportColumnDataType::Date => {
            let date = parse_date(text)?;
            let date = ExcelDateTime::from_ymd(
                u16::try_from(date.year()).map_err(|_| ReportXlsxError::CellTypeMismatch)?,
                u8::try_from(date.month()).map_err(|_| ReportXlsxError::CellTypeMismatch)?,
                u8::try_from(date.day()).map_err(|_| ReportXlsxError::CellTypeMismatch)?,
            )
            .map_err(ReportXlsxError::Serialization)?;
            worksheet
                .write_datetime_with_format(row, column, &date, date_format)
                .map_err(ReportXlsxError::Serialization)?;
            Ok(())
        }
        Some(ReportCell::Number(number)) if column_type == ReportColumnDataType::Number => {
            worksheet
                .write_number(row, column, *number)
                .map_err(ReportXlsxError::Serialization)?;
            Ok(())
        }
        Some(ReportCell::Text(_) | ReportCell::Number(_)) => Err(ReportXlsxError::CellTypeMismatch),
    }
}

fn parse_date(text: &str) -> Result<NaiveDate, ReportXlsxError> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| ReportXlsxError::CellTypeMismatch)
}

struct CappedWriter<W> {
    inner: W,
    limit: usize,
    written: usize,
    output_limit: Arc<AtomicBool>,
}

impl<W: Write> Write for CappedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let next = self
            .written
            .checked_add(bytes.len())
            .ok_or_else(|| self.output_limit_error())?;
        if next > self.limit {
            return Err(self.output_limit_error());
        }
        let written = self.inner.write(bytes)?;
        self.written = self
            .written
            .checked_add(written)
            .ok_or_else(|| self.output_limit_error())?;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl<W> CappedWriter<W> {
    fn output_limit_error(&self) -> std::io::Error {
        self.output_limit.store(true, Ordering::Relaxed);
        std::io::Error::other(BYTE_LIMIT_MARKER)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use super::*;
    use crate::domain::reports::columns::{PlannedColumnSource, ReportColumnMetadata};

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
            ]],
            usize::MAX,
        )
        .unwrap_or_else(|error| panic!("workbook must serialize: {error}"));

        let xml = worksheet_xml(&bytes);
        assert!(xml.contains("r=\"A1\" t=\"inlineStr\""));
        assert!(xml.contains("r=\"A2\" t=\"inlineStr\""));
        assert!(xml.contains("<t>=formula</t>"));
        assert!(xml.contains("r=\"B2\" s=\"1\"><v>46024</v>"));
        assert!(xml.contains("r=\"C2\"><v>12.5</v>"));
        let blank_cell = xml.split("<c ").find(|cell| cell.starts_with("r=\"D2\""));
        assert!(blank_cell.is_none_or(|cell| !cell.contains("<v>")));
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
        assert!(xml.contains("r=\"BL1\" t=\"inlineStr\""));
        assert!(xml.contains("r=\"BL2\"><v>63</v>"));
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
        let mut serializer = ReportXlsxSerializer::new(
            &columns,
            XlsxDimensions {
                rows: 3,
                columns: 1,
            },
            std::env::temp_dir().as_path(),
        )
        .unwrap_or_else(|error| panic!("writer must initialize: {error}"));
        let oversized_row = vec![vec![Some(ReportCell::Text(
            "x".repeat(MAX_REPORT_XLSX_CELL_CHARACTERS + 1),
        ))]];
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
            ))]],
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
        let mut serializer =
            ReportXlsxSerializer::new(columns, dimensions, std::env::temp_dir().as_path())?;
        serializer.write_rows(rows)?;
        let mut bytes = Vec::new();
        serializer.finish_into(&mut bytes, max_output_bytes)?;
        Ok(bytes)
    }

    fn worksheet_xml(bytes: &[u8]) -> String {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
            .unwrap_or_else(|error| panic!("workbook must reopen as ZIP: {error}"));
        let mut sheet = archive
            .by_name("xl/worksheets/sheet1.xml")
            .unwrap_or_else(|error| panic!("workbook must contain first sheet: {error}"));
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
}

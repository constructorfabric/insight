use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Datelike, NaiveDate};
use rust_xlsxwriter::{ExcelDateTime, Format, Workbook, XlsxError};

use super::columns::{PlannedColumn, ReportColumnDataType};
use super::planner::XlsxDimensions;
use super::row::{ReportCell, ReportRow};
use crate::domain::metric_definitions::MetricFormat;

pub(crate) const MAX_REPORT_XLSX_CELL_CHARACTERS: usize = 32_767;
const BYTE_LIMIT_MARKER: &str = "report XLSX byte limit exceeded";
const XLSX_SPOOL_FIXED_BYTES: usize = 4096;
const XLSX_SPOOL_ROW_BYTES: usize = 64;
const XLSX_SPOOL_CELL_BYTES: usize = 256;
const XLSX_SPOOL_TEXT_BYTE_EXPANSION: usize = 8;
const MIN_COLUMN_WIDTH: usize = 10;
const MAX_COLUMN_WIDTH: usize = 40;

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
    #[error("report XLSX temporary serialization exceeds byte limit")]
    SpoolLimitExceeded,
    #[error("report XLSX serialization failed")]
    Serialization(#[source] XlsxError),
}

pub(crate) struct ReportXlsxSerializer {
    workbook: Workbook,
    column_types: Vec<ReportColumnDataType>,
    dimensions: XlsxDimensions,
    next_row: u32,
    column_widths: Vec<usize>,
    estimated_spool_bytes: usize,
    max_spool_bytes: usize,
}

impl ReportXlsxSerializer {
    pub(crate) fn new(
        columns: &[PlannedColumn],
        dimensions: XlsxDimensions,
        temp_dir: &Path,
        max_spool_bytes: usize,
    ) -> Result<Self, ReportXlsxError> {
        validate_dimensions(columns, dimensions)?;
        validate_headers(columns)?;
        let estimated_spool_bytes = XLSX_SPOOL_FIXED_BYTES
            .checked_add(
                estimated_text_row_spool_bytes(
                    columns.iter().map(|column| column.metadata.label.as_str()),
                )
                .ok_or(ReportXlsxError::SpoolLimitExceeded)?,
            )
            .ok_or(ReportXlsxError::SpoolLimitExceeded)?;
        if estimated_spool_bytes > max_spool_bytes {
            return Err(ReportXlsxError::SpoolLimitExceeded);
        }

        let mut workbook = Workbook::new();
        workbook
            .set_tempdir(temp_dir)
            .map_err(ReportXlsxError::Serialization)?;
        let worksheet = workbook.add_worksheet_with_constant_memory();
        let header_format = Format::new().set_bold().set_background_color("D9EAF7");
        for (column_index, column) in columns.iter().enumerate() {
            let column_index = u16::try_from(column_index)
                .map_err(|_| ReportXlsxError::ColumnDimensionMismatch)?;
            worksheet
                .write_string_with_format(0, column_index, &column.metadata.label, &header_format)
                .map_err(ReportXlsxError::Serialization)?;
            if let Some(format) = column.metadata.format.map(metric_format) {
                worksheet
                    .set_column_format(column_index, &format)
                    .map_err(ReportXlsxError::Serialization)?;
            }
        }
        let last_column = dimensions
            .columns
            .checked_sub(1)
            .ok_or(ReportXlsxError::ColumnDimensionMismatch)?;
        let last_row = dimensions
            .rows
            .checked_sub(1)
            .ok_or(ReportXlsxError::ColumnDimensionMismatch)?;
        worksheet
            .set_freeze_panes(1, 0)
            .map_err(ReportXlsxError::Serialization)?;
        worksheet
            .autofilter(0, 0, last_row, last_column)
            .map_err(ReportXlsxError::Serialization)?;

        Ok(Self {
            workbook,
            column_types: columns
                .iter()
                .map(|column| column.metadata.data_type)
                .collect(),
            dimensions,
            next_row: 1,
            column_widths: columns
                .iter()
                .map(|column| column.metadata.label.chars().count())
                .collect(),
            estimated_spool_bytes,
            max_spool_bytes,
        })
    }

    pub(crate) fn write_rows(&mut self, rows: &[ReportRow]) -> Result<(), ReportXlsxError> {
        self.validate_rows(rows)?;
        let batch_spool_bytes = rows.iter().try_fold(0usize, |total, row| {
            total.checked_add(estimated_row_spool_bytes(row)?)
        });
        let estimated_spool_bytes = self
            .estimated_spool_bytes
            .checked_add(batch_spool_bytes.ok_or(ReportXlsxError::SpoolLimitExceeded)?)
            .ok_or(ReportXlsxError::SpoolLimitExceeded)?;
        if estimated_spool_bytes > self.max_spool_bytes {
            return Err(ReportXlsxError::SpoolLimitExceeded);
        }
        self.estimated_spool_bytes = estimated_spool_bytes;

        let worksheet = self
            .workbook
            .worksheet_from_index(0)
            .map_err(ReportXlsxError::Serialization)?;
        let date_format = Format::new().set_num_format("yyyy-mm-dd");
        let blank_format = Format::new();
        for row in rows {
            for (column_index, cell) in row.iter().enumerate() {
                self.column_widths[column_index] = self.column_widths[column_index]
                    .max(cell_width(cell.as_ref(), self.column_types[column_index]));
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

        let worksheet = self
            .workbook
            .worksheet_from_index(0)
            .map_err(ReportXlsxError::Serialization)?;
        for (column_index, width) in self.column_widths.iter().enumerate() {
            let column_index = u16::try_from(column_index)
                .map_err(|_| ReportXlsxError::ColumnDimensionMismatch)?;
            let width = width
                .saturating_add(2)
                .clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
            let width = u32::try_from(width).unwrap_or(40);
            worksheet
                .set_column_width(column_index, f64::from(width))
                .map_err(ReportXlsxError::Serialization)?;
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

fn estimated_row_spool_bytes(row: &[Option<ReportCell>]) -> Option<usize> {
    row.iter().try_fold(XLSX_SPOOL_ROW_BYTES, |total, cell| {
        let text_bytes = match cell {
            Some(ReportCell::Text(value)) => value.len(),
            None | Some(ReportCell::Number(_)) => 0,
        };
        add_estimated_cell_spool_bytes(total, text_bytes)
    })
}

fn estimated_text_row_spool_bytes<'a>(mut texts: impl Iterator<Item = &'a str>) -> Option<usize> {
    texts.try_fold(XLSX_SPOOL_ROW_BYTES, |total, text| {
        add_estimated_cell_spool_bytes(total, text.len())
    })
}

fn add_estimated_cell_spool_bytes(total: usize, text_bytes: usize) -> Option<usize> {
    let cell_bytes = text_bytes
        .checked_mul(XLSX_SPOOL_TEXT_BYTE_EXPANSION)?
        .checked_add(XLSX_SPOOL_CELL_BYTES)?;

    total.checked_add(cell_bytes)
}

fn metric_format(format: MetricFormat) -> Format {
    let code = match format {
        MetricFormat::Integer => "#,##0",
        MetricFormat::Decimal => "#,##0.00",
        MetricFormat::Currency => "$#,##0",
        MetricFormat::Percent => "#,##0\"%\"",
    };

    Format::new().set_num_format(code)
}

fn cell_width(cell: Option<&ReportCell>, column_type: ReportColumnDataType) -> usize {
    match cell {
        Some(ReportCell::Text(value)) => value.chars().count(),
        Some(ReportCell::Number(value)) if column_type == ReportColumnDataType::Number => {
            value.to_string().chars().count()
        }
        None | Some(ReportCell::Number(_)) => 0,
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
#[path = "xlsx_tests.rs"]
mod tests;

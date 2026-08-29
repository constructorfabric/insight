use std::io::Write;

use super::columns::PlannedColumn;
use super::row::{ReportCell, ReportRow};
use crate::domain::spreadsheet::csv_safe_cell;

const MAX_CELL_BYTES: usize = 32 * 1024;
const BYTE_LIMIT_MARKER: &str = "report CSV byte limit exceeded";

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReportCsvError {
    #[error("report row does not match planned columns")]
    RowShapeMismatch,
    #[error("report CSV contains a value exceeding the {limit} byte limit")]
    CellTooLarge { limit: usize },
    #[error("report CSV exceeds the {limit} byte limit")]
    GeneratedByteLimit { limit: usize },
    #[error("report CSV contains a non-finite numeric value")]
    NonFiniteNumber,
    #[error("report CSV serialization failed")]
    Write(#[source] std::io::Error),
}

#[derive(Debug)]
pub(crate) struct ReportCsvWriter<W: Write> {
    writer: csv::Writer<LimitedWriter<W>>,
    column_count: usize,
    byte_limit: usize,
}

impl<W: Write> ReportCsvWriter<W> {
    pub(crate) fn new(
        output: W,
        columns: &[PlannedColumn],
        byte_limit: usize,
    ) -> Result<Self, ReportCsvError> {
        let mut output = LimitedWriter::new(output, byte_limit);
        output
            .write_all(b"\xEF\xBB\xBF")
            .map_err(|error| map_write_error(error, byte_limit))?;
        let mut writer = csv::WriterBuilder::new()
            .terminator(csv::Terminator::CRLF)
            .from_writer(output);

        let headers = columns
            .iter()
            .map(|column| csv_cell(&column.metadata.label))
            .collect::<Result<Vec<_>, _>>()?;
        writer
            .write_record(headers)
            .map_err(|error| map_csv_error(error, byte_limit))?;
        writer
            .flush()
            .map_err(|error| map_write_error(error, byte_limit))?;

        Ok(Self {
            writer,
            column_count: columns.len(),
            byte_limit,
        })
    }

    pub(crate) fn write_batch(&mut self, rows: &[ReportRow]) -> Result<(), ReportCsvError> {
        for row in rows {
            if row.len() != self.column_count {
                return Err(ReportCsvError::RowShapeMismatch);
            }

            let values = row
                .iter()
                .map(|cell| report_cell(cell.as_ref()))
                .collect::<Result<Vec<_>, _>>()?;
            self.writer
                .write_record(values)
                .map_err(|error| map_csv_error(error, self.byte_limit))?;
        }
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<W, ReportCsvError> {
        self.writer
            .into_inner()
            .map(LimitedWriter::into_inner)
            .map_err(|error| map_write_error(error.into_error(), self.byte_limit))
    }
}

fn report_cell(cell: Option<&ReportCell>) -> Result<String, ReportCsvError> {
    match cell {
        None => Ok(String::new()),
        Some(ReportCell::Text(value)) => csv_cell(value),
        Some(ReportCell::Number(value)) if value.is_finite() => Ok(value.to_string()),
        Some(ReportCell::Number(_)) => Err(ReportCsvError::NonFiniteNumber),
    }
}

fn csv_cell(value: &str) -> Result<String, ReportCsvError> {
    let value = csv_safe_cell(value.to_owned());
    if value.len() > MAX_CELL_BYTES {
        return Err(ReportCsvError::CellTooLarge {
            limit: MAX_CELL_BYTES,
        });
    }
    Ok(value)
}

fn map_csv_error(error: csv::Error, byte_limit: usize) -> ReportCsvError {
    match error.into_kind() {
        csv::ErrorKind::Io(error) => map_write_error(error, byte_limit),
        _ => ReportCsvError::Write(std::io::Error::other("CSV serialization failed")),
    }
}

fn map_write_error(error: std::io::Error, byte_limit: usize) -> ReportCsvError {
    if error.to_string() == BYTE_LIMIT_MARKER {
        ReportCsvError::GeneratedByteLimit { limit: byte_limit }
    } else {
        ReportCsvError::Write(error)
    }
}

#[derive(Debug)]
struct LimitedWriter<W> {
    inner: W,
    limit: usize,
    written: usize,
}

impl<W> LimitedWriter<W> {
    fn new(inner: W, limit: usize) -> Self {
        Self {
            inner,
            limit,
            written: 0,
        }
    }

    fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let total = self
            .written
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other(BYTE_LIMIT_MARKER))?;
        if total > self.limit {
            return Err(std::io::Error::other(BYTE_LIMIT_MARKER));
        }

        let count = self.inner.write(bytes)?;
        self.written = self.written.checked_add(count).ok_or_else(|| {
            std::io::Error::other("report CSV byte count overflowed after successful write")
        })?;
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::super::columns::{
        PlannedColumn, PlannedColumnSource, ReportColumnDataType, ReportColumnMetadata,
    };
    use super::super::row::{ReportCell, ReportRow};
    use super::{MAX_CELL_BYTES, ReportCsvError, ReportCsvWriter};

    fn column(label: &str) -> PlannedColumn {
        PlannedColumn {
            metadata: ReportColumnMetadata {
                key: label.to_lowercase(),
                label: label.to_owned(),
                data_type: ReportColumnDataType::Text,
                format: None,
                unit: None,
            },
            source: PlannedColumnSource::PeriodLabel,
        }
    }

    #[test]
    fn writes_bom_crlf_formula_safe_headers_and_rows() {
        let columns = [column("=Header"), column("Number"), column("Missing")];
        let rows: [ReportRow; 1] = [vec![
            Some(ReportCell::Text("=formula".to_owned())),
            Some(ReportCell::Number(1.234_567_890_123_456_7)),
            None,
        ]
        .into()];
        let mut writer = ReportCsvWriter::new(Vec::new(), &columns, 1_024)
            .unwrap_or_else(|error| panic!("writer should start: {error}"));

        writer
            .write_batch(&rows)
            .unwrap_or_else(|error| panic!("rows should write: {error}"));
        let output = writer
            .finish()
            .unwrap_or_else(|error| panic!("writer should finish: {error}"));

        assert_eq!(
            String::from_utf8(output)
                .unwrap_or_else(|error| panic!("output should be UTF-8: {error}")),
            "\u{feff}'=Header,Number,Missing\r\n'=formula,1.2345678901234567,\r\n"
        );
    }

    #[test]
    fn rejects_rows_that_do_not_match_planned_columns() {
        let columns = [column("Only")];
        let mut writer = ReportCsvWriter::new(Vec::new(), &columns, 1_024)
            .unwrap_or_else(|error| panic!("writer should start: {error}"));

        let Err(error) = writer.write_batch(&[vec![None, None].into()]) else {
            panic!("mismatched row must fail");
        };

        assert!(matches!(error, ReportCsvError::RowShapeMismatch));
    }

    #[test]
    fn rejects_cells_after_formula_neutralization_exceeds_cell_limit() {
        let columns = [column("Name")];
        let mut writer = ReportCsvWriter::new(Vec::new(), &columns, 1_024)
            .unwrap_or_else(|error| panic!("writer should start: {error}"));

        let result = writer.write_batch(&[vec![Some(ReportCell::Text(format!(
            "={}",
            "x".repeat(MAX_CELL_BYTES - 1)
        )))]
        .into()]);
        let Err(error) = result else {
            panic!("cell past limit must fail");
        };

        assert!(matches!(
            error,
            ReportCsvError::CellTooLarge {
                limit: MAX_CELL_BYTES
            }
        ));
    }

    #[test]
    fn rejects_generated_output_past_byte_limit() {
        let columns = [column("Header")];

        let Err(error) = ReportCsvWriter::new(Vec::new(), &columns, 3) else {
            panic!("BOM and header must exceed limit");
        };

        assert!(matches!(
            error,
            ReportCsvError::GeneratedByteLimit { limit: 3 }
        ));
    }

    #[test]
    fn rejects_rows_that_push_generated_output_past_byte_limit() {
        let columns = [column("Header")];
        let mut writer = ReportCsvWriter::new(Vec::new(), &columns, 12)
            .unwrap_or_else(|error| panic!("header should fit: {error}"));

        writer
            .write_batch(&[vec![Some(ReportCell::Text("value".to_owned()))].into()])
            .unwrap_or_else(|error| panic!("buffered row should write: {error}"));
        let Err(error) = writer.finish() else {
            panic!("row past generated byte limit must fail");
        };

        assert!(matches!(
            error,
            ReportCsvError::GeneratedByteLimit { limit: 12 }
        ));
    }

    #[test]
    fn rejects_non_finite_numeric_cells() {
        let columns = [column("Metric")];
        let mut writer = ReportCsvWriter::new(Vec::new(), &columns, 1_024)
            .unwrap_or_else(|error| panic!("writer should start: {error}"));

        let Err(error) = writer.write_batch(&[vec![Some(ReportCell::Number(f64::NAN))].into()])
        else {
            panic!("non-finite metric must fail");
        };

        assert!(matches!(error, ReportCsvError::NonFiniteNumber));
    }
}

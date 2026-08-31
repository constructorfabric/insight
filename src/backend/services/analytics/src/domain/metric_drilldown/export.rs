use std::io::{Cursor, Seek, SeekFrom, Write};

use rust_xlsxwriter::{ExcelDateTime, Format, Table, TableStyle, Workbook};
use toolkit_canonical_errors::CanonicalError;

use crate::domain::spreadsheet::csv_safe_cell;

use super::dto::{
    MetricDrilldownColumn, MetricDrilldownColumnType, MetricDrilldownExportFormat,
    MetricDrilldownRow,
};
use super::error::{export_internal, export_limit};

pub const BYTE_LIMIT_MARKER: &str = "export byte limit exceeded";
const DEADLINE_CHECK_EVERY_ROWS: usize = 512;
const MAX_FILENAME_SLUG_BYTES: usize = 80;
pub(crate) const MAX_EXPORT_BYTES: usize = 25 * 1024 * 1024;
const MAX_CELL_BYTES: usize = 32 * 1024;

pub fn build_export(
    format: MetricDrilldownExportFormat,
    columns: &[MetricDrilldownColumn],
    rows: &[MetricDrilldownRow],
    deadline: std::time::Instant,
) -> Result<(Vec<u8>, &'static str, &'static str), CanonicalError> {
    match format {
        MetricDrilldownExportFormat::Csv => {
            let formatted_rows = rows
                .iter()
                .map(|row| export_values(columns, row))
                .collect::<Result<Vec<_>, _>>()?;

            let mut budget = ExportInputBudget::new(columns)?;
            for row in &formatted_rows {
                budget.add_row(row)?;
            }
            Ok((
                build_csv(columns, formatted_rows, deadline)?,
                "text/csv; charset=utf-8",
                "csv",
            ))
        }
        MetricDrilldownExportFormat::Xlsx => {
            let mut budget = ExportInputBudget::new(columns)?;
            for row in rows {
                budget.add_row(&export_values(columns, row)?)?;
            }
            Ok((
                build_xlsx(columns, rows, deadline)?,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "xlsx",
            ))
        }
    }
}

fn build_csv(
    columns: &[MetricDrilldownColumn],
    rows: Vec<Vec<String>>,
    deadline: std::time::Instant,
) -> Result<Vec<u8>, CanonicalError> {
    let mut writer = csv::Writer::from_writer(LimitedBuffer::new(MAX_EXPORT_BYTES));
    let headers = columns
        .iter()
        .map(|column| column.label.as_str())
        .collect::<Vec<_>>();
    writer
        .write_record(&headers)
        .map_err(|_| export_limit("CSV export exceeds the byte limit."))?;
    for (row_index, row) in rows.into_iter().enumerate() {
        check_export_deadline(row_index, deadline)?;
        let values = row.into_iter().map(csv_safe_cell).collect::<Vec<_>>();
        writer
            .write_record(values)
            .map_err(|_| export_limit("CSV export exceeds the byte limit."))?;
    }
    writer
        .into_inner()
        .map(LimitedBuffer::into_inner)
        .map_err(|_| export_limit("CSV export exceeds the byte limit."))
}

fn build_xlsx(
    columns: &[MetricDrilldownColumn],
    rows: &[MetricDrilldownRow],
    deadline: std::time::Instant,
) -> Result<Vec<u8>, CanonicalError> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    let date_format = Format::new().set_num_format("yyyy-mm-dd");
    let blank_format = Format::new();

    for (column, header) in columns.iter().enumerate() {
        let column = u16::try_from(column).map_err(|_| xlsx_error("header column index"))?;
        worksheet
            .write_string(0, column, &header.label)
            .map_err(|_| xlsx_error("header cell"))?;
    }

    for (row_index, row) in rows.iter().enumerate() {
        check_export_deadline(row_index, deadline)?;
        let row_index = u32::try_from(row_index + 1).map_err(|_| xlsx_error("row index"))?;
        for (column_index, column) in columns.iter().enumerate() {
            let column_index =
                u16::try_from(column_index).map_err(|_| xlsx_error("column index"))?;
            let value = row
                .values
                .get(&column.key)
                .unwrap_or(&serde_json::Value::Null);
            match column.r#type {
                MetricDrilldownColumnType::Number => match value {
                    serde_json::Value::Null => worksheet
                        .write_blank(row_index, column_index, &blank_format)
                        .map_err(|_| xlsx_error("blank cell"))?,
                    value => match value.as_f64() {
                        Some(number) => worksheet
                            .write_number(row_index, column_index, number)
                            .map_err(|_| xlsx_error("number cell"))?,
                        None => worksheet
                            .write_string(row_index, column_index, cell_text(value)?)
                            .map_err(|_| xlsx_error("number cell as text"))?,
                    },
                },
                MetricDrilldownColumnType::Date => match value {
                    serde_json::Value::Null => worksheet
                        .write_blank(row_index, column_index, &blank_format)
                        .map_err(|_| xlsx_error("blank cell"))?,
                    serde_json::Value::String(text) => match ExcelDateTime::parse_from_str(text) {
                        Ok(date) => worksheet
                            .write_datetime_with_format(
                                row_index,
                                column_index,
                                &date,
                                &date_format,
                            )
                            .map_err(|_| xlsx_error("date cell"))?,
                        Err(_) => worksheet
                            .write_string(row_index, column_index, text)
                            .map_err(|_| xlsx_error("date cell as text"))?,
                    },
                    value => worksheet
                        .write_string(row_index, column_index, cell_text(value)?)
                        .map_err(|_| xlsx_error("date cell as text"))?,
                },
                MetricDrilldownColumnType::String => match value {
                    serde_json::Value::Null => worksheet
                        .write_blank(row_index, column_index, &blank_format)
                        .map_err(|_| xlsx_error("blank cell"))?,
                    serde_json::Value::Bool(flag) => worksheet
                        .write_boolean(row_index, column_index, *flag)
                        .map_err(|_| xlsx_error("boolean cell"))?,
                    value => worksheet
                        .write_string(row_index, column_index, cell_text(value)?)
                        .map_err(|_| xlsx_error("string cell"))?,
                },
            };
        }
    }

    if !rows.is_empty() && !columns.is_empty() {
        let last_row = u32::try_from(rows.len()).map_err(|_| xlsx_error("table last row"))?;
        let last_column =
            u16::try_from(columns.len() - 1).map_err(|_| xlsx_error("table last column"))?;
        let table = Table::new()
            .set_style(TableStyle::None)
            .set_autofilter(false)
            .set_banded_rows(false);
        worksheet
            .add_table(0, 0, last_row, last_column, &table)
            .map_err(|_| xlsx_error("table"))?;
    }

    let mut output = LimitedBuffer::new(MAX_EXPORT_BYTES);
    workbook.save_to_writer(&mut output).map_err(|error| {
        let message = error.to_string();
        if message.contains(BYTE_LIMIT_MARKER) {
            tracing::warn!(error = %error, "metric drilldown XLSX exceeded the byte limit");
            return export_limit("XLSX export exceeds the byte limit.");
        }
        tracing::error!(error = %error, "metric drilldown XLSX serialization failed");
        export_internal()
    })?;
    Ok(output.into_inner())
}

// INVARIANT: serialization stops at the deadline so the concurrency permit is
// released at the client-visible limit, not when the blocking task finishes.
fn check_export_deadline(
    row_index: usize,
    deadline: std::time::Instant,
) -> Result<(), CanonicalError> {
    if row_index.is_multiple_of(DEADLINE_CHECK_EVERY_ROWS) && std::time::Instant::now() >= deadline
    {
        return Err(export_limit("Export exceeded the execution time limit."));
    }
    Ok(())
}

fn cell_text(value: &serde_json::Value) -> Result<String, CanonicalError> {
    match value {
        serde_json::Value::String(text) => Ok(text.clone()),
        value => serde_json::to_string(value).map_err(|_| xlsx_error("cell encode")),
    }
}

fn xlsx_error(operation: &'static str) -> CanonicalError {
    tracing::error!(operation, "metric drilldown XLSX serialization failed");
    export_internal()
}

fn export_values(
    columns: &[MetricDrilldownColumn],
    row: &MetricDrilldownRow,
) -> Result<Vec<String>, CanonicalError> {
    let values = columns
        .iter()
        .map(|column| {
            let value = row
                .values
                .get(&column.key)
                .unwrap_or(&serde_json::Value::Null);
            match value {
                serde_json::Value::Null => Ok(String::new()),
                serde_json::Value::String(value) => Ok(value.clone()),
                serde_json::Value::Bool(value) => Ok(value.to_string()),
                serde_json::Value::Number(value) => Ok(value.to_string()),
                value => serde_json::to_string(value).map_err(|_| export_internal()),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.iter().any(|value| value.len() > MAX_CELL_BYTES) {
        return Err(export_limit(format!(
            "Export contains a value exceeding the {MAX_CELL_BYTES} byte limit."
        )));
    }
    Ok(values)
}

struct ExportInputBudget {
    bytes: usize,
}

impl ExportInputBudget {
    fn new(columns: &[MetricDrilldownColumn]) -> Result<Self, CanonicalError> {
        let bytes = columns
            .iter()
            .try_fold(0usize, |total, column| {
                total.checked_add(column.label.len() + 1)
            })
            .ok_or_else(input_too_large)?;
        Ok(Self { bytes })
    }

    fn add_row(&mut self, values: &[String]) -> Result<(), CanonicalError> {
        for value in values {
            self.bytes = self
                .bytes
                .checked_add(value.len() + 1)
                .ok_or_else(input_too_large)?;
            if self.bytes > MAX_EXPORT_BYTES {
                return Err(input_too_large());
            }
        }
        Ok(())
    }
}

fn input_too_large() -> CanonicalError {
    export_limit("Export input exceeds the byte limit.")
}

#[derive(Debug)]
struct LimitedBuffer {
    inner: Cursor<Vec<u8>>,
    limit: usize,
}

impl LimitedBuffer {
    fn new(limit: usize) -> Self {
        Self {
            inner: Cursor::new(Vec::new()),
            limit,
        }
    }

    fn into_inner(self) -> Vec<u8> {
        self.inner.into_inner()
    }
}

impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let end = self
            .inner
            .position()
            .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
            .ok_or_else(|| std::io::Error::other(BYTE_LIMIT_MARKER))?;
        if end > self.limit as u64 {
            return Err(std::io::Error::other(BYTE_LIMIT_MARKER));
        }
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for LimitedBuffer {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let offset = self.inner.seek(position)?;
        if offset > self.limit as u64 {
            return Err(std::io::Error::other(BYTE_LIMIT_MARKER));
        }
        Ok(offset)
    }
}

pub fn export_filename(
    metric_label: &str,
    metric_key: &str,
    from: &str,
    to: &str,
    filtered: bool,
    extension: &str,
) -> String {
    let metric = filename_slug(metric_label);
    let metric = if metric.is_empty() {
        filename_slug(metric_key)
    } else {
        metric
    };
    let suffix = if filtered { "_filtered" } else { "" };
    format!("{metric}_{from}_{to}{suffix}.{extension}")
}

fn filename_slug(value: &str) -> String {
    let mut slug = String::with_capacity(value.len().min(MAX_FILENAME_SLUG_BYTES));
    let mut separated = true;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if slug.len() == MAX_FILENAME_SLUG_BYTES {
                break;
            }
            slug.push(character.to_ascii_lowercase());
            separated = false;
        } else if !separated && slug.len() < MAX_FILENAME_SLUG_BYTES {
            slug.push('-');
            separated = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn far_deadline() -> std::time::Instant {
        std::time::Instant::now() + std::time::Duration::from_mins(1)
    }

    fn columns() -> Vec<MetricDrilldownColumn> {
        vec![
            MetricDrilldownColumn {
                key: "ref".to_owned(),
                label: "Ref".to_owned(),
                r#type: MetricDrilldownColumnType::String,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "date".to_owned(),
                label: "Date".to_owned(),
                r#type: MetricDrilldownColumnType::Date,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "value".to_owned(),
                label: "Value".to_owned(),
                r#type: MetricDrilldownColumnType::Number,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "active".to_owned(),
                label: "Active".to_owned(),
                r#type: MetricDrilldownColumnType::String,
                sortable: true,
            },
        ]
    }

    fn row() -> MetricDrilldownRow {
        MetricDrilldownRow {
            values: BTreeMap::from([
                ("ref".to_owned(), json!("=formula")),
                ("date".to_owned(), json!("2026-07-28")),
                ("value".to_owned(), json!(12.5)),
                ("active".to_owned(), json!(true)),
            ]),
            links: BTreeMap::new(),
        }
    }

    #[test]
    fn csv_export_is_bounded_and_formula_safe() {
        let (bytes, content_type, extension) = build_export(
            MetricDrilldownExportFormat::Csv,
            &columns(),
            &[row()],
            far_deadline(),
        )
        .unwrap_or_else(|error| panic!("CSV export must succeed: {error}"));
        let csv = String::from_utf8(bytes)
            .unwrap_or_else(|error| panic!("CSV export must be UTF-8: {error}"));
        assert_eq!(content_type, "text/csv; charset=utf-8");
        assert_eq!(extension, "csv");
        assert!(csv.contains("'=formula"));
        assert!(csv.contains("2026-07-28"));
        assert!(csv.contains("12.5"));
    }

    #[test]
    fn export_input_over_the_byte_limit_is_rejected() {
        let columns = vec![MetricDrilldownColumn {
            key: "value".to_owned(),
            label: "Value".to_owned(),
            r#type: MetricDrilldownColumnType::String,
            sortable: true,
        }];
        let row = vec!["x".repeat(MAX_CELL_BYTES)];

        let mut budget = ExportInputBudget::new(&columns)
            .unwrap_or_else(|error| panic!("header budget must fit: {error}"));
        let rejected =
            (0..=MAX_EXPORT_BYTES / MAX_CELL_BYTES).any(|_| budget.add_row(&row).is_err());
        assert!(rejected, "input past the byte limit must be rejected");

        let mut small = ExportInputBudget::new(&columns)
            .unwrap_or_else(|error| panic!("header budget must fit: {error}"));
        assert!(small.add_row(&["small".to_owned()]).is_ok());
    }

    #[test]
    fn xlsx_export_contains_typed_cells() {
        let (bytes, content_type, extension) = build_export(
            MetricDrilldownExportFormat::Xlsx,
            &columns(),
            &[row()],
            far_deadline(),
        )
        .unwrap_or_else(|error| panic!("XLSX export must succeed: {error}"));
        assert_eq!(
            content_type,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
        assert_eq!(extension, "xlsx");
        assert!(bytes.starts_with(b"PK"));
        assert!(bytes.len() > 1_000);
    }

    #[test]
    fn export_values_serialize_supported_json_values() {
        let columns = vec![
            MetricDrilldownColumn {
                key: "missing".to_owned(),
                label: "Missing".to_owned(),
                r#type: MetricDrilldownColumnType::String,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "object".to_owned(),
                label: "Object".to_owned(),
                r#type: MetricDrilldownColumnType::String,
                sortable: true,
            },
        ];
        let row = MetricDrilldownRow {
            values: BTreeMap::from([("object".to_owned(), json!({"key": "value"}))]),
            links: BTreeMap::new(),
        };
        assert_eq!(
            export_values(&columns, &row)
                .unwrap_or_else(|error| panic!("export values must serialize: {error}")),
            ["", r#"{"key":"value"}"#]
        );
    }

    #[test]
    fn oversized_export_cells_are_rejected() {
        let columns = vec![MetricDrilldownColumn {
            key: "value".to_owned(),
            label: "Value".to_owned(),
            r#type: MetricDrilldownColumnType::String,
            sortable: true,
        }];
        let row = MetricDrilldownRow {
            values: BTreeMap::from([("value".to_owned(), json!("x".repeat(MAX_CELL_BYTES + 1)))]),
            links: BTreeMap::new(),
        };
        assert!(export_values(&columns, &row).is_err());
    }

    #[test]
    fn limited_buffer_enforces_write_and_seek_bounds() {
        let mut buffer = LimitedBuffer::new(4);
        assert_eq!(
            buffer
                .write(b"1234")
                .unwrap_or_else(|error| panic!("bounded write must succeed: {error}")),
            4
        );
        assert!(buffer.write(b"5").is_err());
        assert!(buffer.seek(SeekFrom::Start(5)).is_err());
        assert_eq!(buffer.into_inner(), b"1234");
    }

    #[test]
    fn filenames_are_human_readable_and_bounded() {
        assert_eq!(
            export_filename(
                "Issues closed",
                "tasks.closed",
                "2025-07-28",
                "2026-07-27",
                true,
                "xlsx"
            ),
            "issues-closed_2025-07-28_2026-07-27_filtered.xlsx"
        );
        assert_eq!(filename_slug("***"), "");
        assert!(filename_slug(&"a".repeat(100)).len() <= 80);
    }

    #[test]
    fn xlsx_writes_every_cell_shape_the_contract_allows() {
        let columns = vec![
            MetricDrilldownColumn {
                key: "number".to_owned(),
                label: "Number".to_owned(),
                r#type: MetricDrilldownColumnType::Number,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "unnumeric".to_owned(),
                label: "Not a number".to_owned(),
                r#type: MetricDrilldownColumnType::Number,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "date".to_owned(),
                label: "Date".to_owned(),
                r#type: MetricDrilldownColumnType::Date,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "text".to_owned(),
                label: "Text".to_owned(),
                r#type: MetricDrilldownColumnType::String,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "flag".to_owned(),
                label: "Flag".to_owned(),
                r#type: MetricDrilldownColumnType::String,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "nested".to_owned(),
                label: "Nested".to_owned(),
                r#type: MetricDrilldownColumnType::String,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "absent".to_owned(),
                label: "Absent".to_owned(),
                r#type: MetricDrilldownColumnType::String,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "numeric_date".to_owned(),
                label: "Numeric date".to_owned(),
                r#type: MetricDrilldownColumnType::Date,
                sortable: true,
            },
            MetricDrilldownColumn {
                key: "numeric_text".to_owned(),
                label: "Numeric text".to_owned(),
                r#type: MetricDrilldownColumnType::String,
                sortable: true,
            },
        ];
        let row = MetricDrilldownRow {
            values: BTreeMap::from([
                ("number".to_owned(), json!(12.5)),
                ("unnumeric".to_owned(), json!("not numeric")),
                ("date".to_owned(), json!("2026-07-28")),
                ("text".to_owned(), json!("plain")),
                ("flag".to_owned(), json!(true)),
                ("nested".to_owned(), json!({"key": "value"})),
                ("numeric_date".to_owned(), json!(20_260_728)),
                ("numeric_text".to_owned(), json!(7)),
            ]),
            links: BTreeMap::new(),
        };

        let bytes = build_xlsx(&columns, &[row], far_deadline())
            .unwrap_or_else(|error| panic!("every cell shape must serialize: {error}"));
        assert!(bytes.starts_with(b"PK"), "XLSX is a zip container");
    }

    #[test]
    fn an_unparseable_date_is_written_as_text_not_an_error() {
        let columns = vec![MetricDrilldownColumn {
            key: "date".to_owned(),
            label: "Date".to_owned(),
            r#type: MetricDrilldownColumnType::Date,
            sortable: true,
        }];
        let row = MetricDrilldownRow {
            values: BTreeMap::from([("date".to_owned(), json!("not-a-date"))]),
            links: BTreeMap::new(),
        };
        let bytes = build_xlsx(&columns, &[row], far_deadline()).unwrap_or_else(|error| {
            panic!("unparseable warehouse dates must not fail the export: {error}")
        });
        assert!(bytes.starts_with(b"PK"));
    }

    #[test]
    fn number_column_text_is_written_unquoted_like_the_csv_path() {
        let columns = vec![MetricDrilldownColumn {
            key: "count".to_owned(),
            label: "Count".to_owned(),
            r#type: MetricDrilldownColumnType::Number,
            sortable: true,
        }];
        let value = json!("not numeric");
        assert_eq!(
            cell_text(&value).unwrap_or_else(|error| panic!("text must encode: {error}")),
            "not numeric",
            "a string in a Number column keeps its bare text, no JSON quotes"
        );
        assert_eq!(
            cell_text(&json!({"key": "value"}))
                .unwrap_or_else(|error| panic!("json must encode: {error}")),
            r#"{"key":"value"}"#,
            "non-string values stay JSON-encoded"
        );

        let row = MetricDrilldownRow {
            values: BTreeMap::from([("count".to_owned(), value)]),
            links: BTreeMap::new(),
        };
        assert!(build_xlsx(&columns, &[row], far_deadline()).is_ok());
    }

    #[test]
    fn an_elapsed_deadline_aborts_serialization() {
        let Some(elapsed) =
            std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1))
        else {
            panic!("clock must support a past instant");
        };
        for format in [
            MetricDrilldownExportFormat::Csv,
            MetricDrilldownExportFormat::Xlsx,
        ] {
            assert!(
                build_export(format, &columns(), &[row()], elapsed).is_err(),
                "should abort past the deadline: {format:?}"
            );
        }
        assert!(
            check_export_deadline(1, elapsed).is_ok(),
            "rows between checkpoints skip the clock read"
        );
    }

    #[test]
    fn export_format_strings_are_stable() {
        assert_eq!(MetricDrilldownExportFormat::Csv.as_str(), "csv");
        assert_eq!(MetricDrilldownExportFormat::Xlsx.as_str(), "xlsx");
    }
}

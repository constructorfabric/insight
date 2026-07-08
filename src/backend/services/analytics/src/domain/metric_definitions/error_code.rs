use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetricSchemaErrorCode {
    TableNotFound,
    ColumnNotFound,
    DimensionNotCovered,
    Unknown,
}

#[cfg(test)]
pub const ALL_METRIC_SCHEMA_ERROR_CODES: &[MetricSchemaErrorCode] = &[
    MetricSchemaErrorCode::TableNotFound,
    MetricSchemaErrorCode::ColumnNotFound,
    MetricSchemaErrorCode::DimensionNotCovered,
    MetricSchemaErrorCode::Unknown,
];

impl MetricSchemaErrorCode {
    #[must_use]
    pub fn as_db(self) -> &'static str {
        match self {
            Self::TableNotFound => "table_not_found",
            Self::ColumnNotFound => "column_not_found",
            Self::DimensionNotCovered => "dimension_not_covered",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for MetricSchemaErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_db())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaStatus {
    Ok,
    Error,
    Unchecked,
}

impl SchemaStatus {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Unchecked => "unchecked",
        }
    }

    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "ok" => Some(Self::Ok),
            "error" => Some(Self::Error),
            "unchecked" => Some(Self::Unchecked),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_status_round_trips() {
        for status in [
            SchemaStatus::Ok,
            SchemaStatus::Error,
            SchemaStatus::Unchecked,
        ] {
            assert_eq!(SchemaStatus::from_db(status.as_db()), Some(status));
        }
        assert_eq!(SchemaStatus::from_db("nope"), None);
    }

    #[test]
    fn all_codes_listed_once() {
        let mut strings = ALL_METRIC_SCHEMA_ERROR_CODES
            .iter()
            .map(|code| code.as_db())
            .collect::<Vec<_>>();
        strings.sort_unstable();
        strings.dedup();
        assert_eq!(strings.len(), ALL_METRIC_SCHEMA_ERROR_CODES.len());
    }
}

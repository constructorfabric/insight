//! `metric_key` (`table_name.column_name`) parser.
//!
//! The DB-side CHECK `chk_metric_catalog_metric_key_shape` constrains the wire
//! format to `^[a-z][a-z0-9_]*[.][a-z][a-z0-9_]*$`; this parser is the
//! application-layer mirror per the dual-validate principle. It rejects every
//! shape the regex would reject so a row that somehow slipped past the DB CHECK
//! (a DBA dropping the constraint, a future MariaDB downgrade losing CHECK
//! enforcement) cannot bypass the validator and reach the ClickHouse query as
//! an unconstrained string.

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("metric_key is empty")]
    Empty,
    #[error("metric_key must contain exactly one '.' separating table and column")]
    DotCount,
    #[error("metric_key table segment is empty")]
    EmptyTable,
    #[error("metric_key column segment is empty")]
    EmptyColumn,
    #[error(
        "metric_key contains characters outside [a-z0-9_]; must be lowercase snake_case both sides"
    )]
    BadCharacters,
    #[error("metric_key segment must start with [a-z]")]
    BadLeadingChar,
}

/// Parsed `metric_key` halves. Both are non-empty, lowercase `snake_case`
/// starting with `[a-z]` — same alphabet as the DB-side CHECK regex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedKey<'a> {
    pub table: &'a str,
    pub column: &'a str,
}

/// Parse `"table.column"` from a row's `metric_key`.
///
/// # Errors
///
/// Returns [`ParseError`] for any shape that would also fail
/// `chk_metric_catalog_metric_key_shape`.
pub fn parse_metric_key(metric_key: &str) -> Result<ParsedKey<'_>, ParseError> {
    if metric_key.is_empty() {
        return Err(ParseError::Empty);
    }
    if metric_key.matches('.').count() != 1 {
        return Err(ParseError::DotCount);
    }
    let Some((table, column)) = metric_key.split_once('.') else {
        return Err(ParseError::DotCount);
    };
    if table.is_empty() {
        return Err(ParseError::EmptyTable);
    }
    if column.is_empty() {
        return Err(ParseError::EmptyColumn);
    }
    validate_segment(table)?;
    validate_segment(column)?;
    Ok(ParsedKey { table, column })
}

fn validate_segment(segment: &str) -> Result<(), ParseError> {
    // First char `[a-z]`; rest `[a-z0-9_]`. Matches the DB CHECK alphabet exactly.
    // Caller (`parse_metric_key`) pre-checks `is_empty()` for both halves, so
    // `chars.next()` must yield Some; if a future refactor calls this directly
    // we'd rather panic in dev than return a misleading `EmptyTable` for a
    // column segment.
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        unreachable!("validate_segment called with empty segment — caller must pre-check");
    };
    if !first.is_ascii_lowercase() {
        return Err(ParseError::BadLeadingChar);
    }
    for c in chars {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return Err(ParseError::BadCharacters);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_or_panic(key: &str) -> ParsedKey<'_> {
        // Test helper — clippy's `expect_used = deny` covers test code too,
        // so we panic via `unwrap_or_else` (the convention used in this repo;
        // see `migration/live_tests.rs` for the same pattern).
        parse_metric_key(key).unwrap_or_else(|e| panic!("must parse {key:?}: {e}"))
    }

    #[test]
    fn happy_path_two_segments() {
        let p = parse_or_panic("analytics_metrics.tasks_closed");
        assert_eq!(p.table, "analytics_metrics");
        assert_eq!(p.column, "tasks_closed");
    }

    #[test]
    fn single_char_segments_are_legal() {
        // Mirrors `live_tests::catalog_schema_end_to_end` invariant 6 (positive case):
        // the regex `^[a-z][a-z0-9_]*[.][a-z][a-z0-9_]*$` accepts segments of length ≥ 1.
        let p = parse_or_panic("a.b");
        assert_eq!(p.table, "a");
        assert_eq!(p.column, "b");
    }

    #[test]
    fn empty_string_rejected() {
        assert_eq!(parse_metric_key(""), Err(ParseError::Empty));
    }

    #[test]
    fn no_dot_rejected() {
        assert_eq!(parse_metric_key("no_dot_here"), Err(ParseError::DotCount));
    }

    #[test]
    fn multiple_dots_rejected() {
        assert_eq!(parse_metric_key("a.b.c"), Err(ParseError::DotCount));
    }

    #[test]
    fn empty_table_rejected() {
        assert_eq!(parse_metric_key(".column"), Err(ParseError::EmptyTable));
    }

    #[test]
    fn empty_column_rejected() {
        assert_eq!(parse_metric_key("table."), Err(ParseError::EmptyColumn));
    }

    #[test]
    fn uppercase_rejected() {
        // Mirrors `live_tests::catalog_schema_end_to_end` invariant 6 (negative case).
        assert_eq!(
            parse_metric_key("Analytics.tasks"),
            Err(ParseError::BadLeadingChar)
        );
    }

    #[test]
    fn leading_digit_rejected() {
        assert_eq!(
            parse_metric_key("1table.column"),
            Err(ParseError::BadLeadingChar)
        );
    }

    #[test]
    fn leading_underscore_rejected() {
        assert_eq!(
            parse_metric_key("_table.column"),
            Err(ParseError::BadLeadingChar)
        );
    }

    #[test]
    fn hyphen_rejected() {
        assert_eq!(
            parse_metric_key("my-table.col"),
            Err(ParseError::BadCharacters)
        );
    }

    #[test]
    fn whitespace_rejected() {
        assert_eq!(
            parse_metric_key("my table.col"),
            Err(ParseError::BadCharacters)
        );
    }

    #[test]
    fn sql_injection_shaped_input_rejected() {
        // Defense-in-depth: even though every bind is parameterized, the parser
        // refuses to hand the ClickHouse probe a "table" that contains a quote
        // or backtick. The probe is the only consumer of these strings and the
        // bound-parameter path is safe, but rejecting at the source is cheap.
        assert!(parse_metric_key("a';--.b").is_err());
        assert!(parse_metric_key("a.b`drop").is_err());
    }
}

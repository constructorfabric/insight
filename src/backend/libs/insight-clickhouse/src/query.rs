//! Tenant-scoped, parameterized query builder.
//!
//! All values are passed via bind parameters — no string interpolation.
//! The builder always starts with `WHERE insight_tenant_id = ?` to enforce tenant isolation.

use clickhouse::{RowOwned, RowRead};
use uuid::Uuid;

use crate::{Client, Error};

/// A parameterized query builder that enforces tenant isolation.
///
/// Created via [`Client::tenant_query`]. Automatically scopes all queries
/// to a single tenant and applies query timeout.
///
/// # Example
///
/// ```rust,ignore
/// let rows: Vec<Metric> = client
///     .tenant_query("gold.pr_cycle_time", tenant_id)?
///     .filter("org_unit_id = ?", org_unit_id)?
///     .filter("metric_date >= ?", "2026-01-01")?
///     .order_by("metric_date DESC")?
///     .limit(100)
///     .fetch_all()
///     .await?;
/// ```
pub struct QueryBuilder {
    client: Client,
    table: String,
    tenant_id: Uuid,
    filters: Vec<String>,
    bind_values: Vec<serde_json::Value>,
    order_by: Option<String>,
    limit: Option<u64>,
    offset: Option<u64>,
    select: Option<String>,
}

impl core::fmt::Debug for QueryBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("QueryBuilder")
            .field("table", &self.table)
            .field("tenant_id", &self.tenant_id)
            .field("filters", &self.filters)
            .field("bind_values_count", &self.bind_values.len())
            .field("order_by", &self.order_by)
            .field("limit", &self.limit)
            .field("offset", &self.offset)
            .field("select", &self.select)
            .field("client", &"<Client>")
            .finish()
    }
}

impl QueryBuilder {
    /// Creates a new query builder for the given table and tenant.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] if `table` contains unsafe characters.
    pub(crate) fn new(client: Client, table: &str, tenant_id: Uuid) -> Result<Self, Error> {
        if !is_safe_identifier(table) {
            tracing::warn!(table = %table, "rejected unsafe table name");
            return Err(Error::InvalidQuery(format!(
                "table name must be non-empty and contain only alphanumeric, '_', or '.' characters, \
                 got: {table}"
            )));
        }
        Ok(Self {
            client,
            table: table.to_owned(),
            tenant_id,
            filters: Vec::new(),
            bind_values: Vec::new(),
            order_by: None,
            limit: None,
            offset: None,
            select: None,
        })
    }

    /// Adds a filter condition with a serializable value.
    ///
    /// The condition **must** contain exactly one `?` placeholder and only
    /// safe SQL characters (identifiers, operators, parentheses for `IN(?)`).
    ///
    /// Accepts any type that implements `Serialize`: UUID, String, integers,
    /// floats, dates, booleans, etc.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] if the condition has wrong placeholder
    /// count, contains unsafe characters, or the value cannot be serialized.
    pub fn filter(self, condition: &str, value: impl serde::Serialize) -> Result<Self, Error> {
        let json_value = serde_json::to_value(value).map_err(|e| {
            tracing::warn!(error = %e, "failed to serialize filter value");
            Error::InvalidQuery(format!("failed to serialize filter value: {e}"))
        })?;
        self.push_filter(condition, json_value)
    }

    fn push_filter(mut self, condition: &str, value: serde_json::Value) -> Result<Self, Error> {
        let placeholder_count = condition.matches('?').count();
        if placeholder_count != 1 {
            tracing::warn!(
                condition = %condition,
                placeholder_count,
                "filter condition must have exactly one '?' placeholder"
            );
            return Err(Error::InvalidQuery(format!(
                "filter condition must contain exactly one '?' placeholder, \
                 got {placeholder_count} in: {condition}"
            )));
        }
        if !validate_filter_condition(condition) {
            tracing::warn!(condition = %condition, "filter condition contains unsafe characters");
            return Err(Error::InvalidQuery(format!(
                "filter condition contains unsafe characters: {condition}"
            )));
        }
        self.filters.push(format!("({condition})"));
        self.bind_values.push(value);
        Ok(self)
    }

    /// Sets the ORDER BY clause.
    ///
    /// Accepts comma-separated column references with optional `ASC`/`DESC`.
    /// Example: `"metric_date DESC, person_id ASC"`
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] if the clause contains unsafe characters.
    pub fn order_by(mut self, clause: &str) -> Result<Self, Error> {
        if !validate_order_by(clause) {
            tracing::warn!(clause = %clause, "order_by clause contains unsafe characters");
            return Err(Error::InvalidQuery(format!(
                "order_by clause contains unsafe characters: {clause}"
            )));
        }
        self.order_by = Some(clause.to_owned());
        Ok(self)
    }

    /// Sets the LIMIT.
    #[must_use]
    pub fn limit(mut self, n: u64) -> Self {
        self.limit = Some(n);
        self
    }

    /// Sets the OFFSET.
    #[must_use]
    pub fn offset(mut self, n: u64) -> Self {
        self.offset = Some(n);
        self
    }

    /// Sets the SELECT columns. Default is `*`.
    ///
    /// Accepts comma-separated column names.
    /// Example: `"person_id, avg_hours, metric_date"`
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] if any column name contains unsafe characters.
    pub fn select(mut self, columns: &str) -> Result<Self, Error> {
        if !validate_select(columns) {
            tracing::warn!(columns = %columns, "select columns contain unsafe characters");
            return Err(Error::InvalidQuery(format!(
                "select columns contain unsafe characters: {columns}"
            )));
        }
        self.select = Some(columns.to_owned());
        Ok(self)
    }

    /// Builds the SQL string (for debugging). Values are shown as `?`.
    #[must_use]
    pub fn to_sql(&self) -> String {
        use std::fmt::Write;

        let select = self.select.as_deref().unwrap_or("*");
        let mut sql = format!(
            "SELECT {select} FROM {} WHERE insight_tenant_id = ?",
            self.table
        );

        for filter in &self.filters {
            let _ = write!(sql, " AND {filter}");
        }

        if let Some(order) = &self.order_by {
            let _ = write!(sql, " ORDER BY {order}");
        }

        if let Some(limit) = self.limit {
            let _ = write!(sql, " LIMIT {limit}");
        }

        if let Some(offset) = self.offset {
            let _ = write!(sql, " OFFSET {offset}");
        }

        sql
    }

    /// Executes the query and returns all matching rows.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if the query fails or times out.
    pub async fn fetch_all<T>(self) -> Result<Vec<T>, Error>
    where
        T: RowOwned + RowRead,
    {
        let sql = self.to_sql();
        tracing::debug!(sql = %sql, "executing tenant-scoped query");

        let mut query = self.client.query(&sql);

        // Bind tenant_id first (always the first `?`)
        query = query.bind(self.tenant_id);

        // Bind additional filter values in order
        for value in self.bind_values {
            query = query.bind(value);
        }

        let rows = query.fetch_all().await?;
        Ok(rows)
    }
}

/// Validates a SELECT column list. Allows: alphanumeric, `_`, `.`, `,`, `*`, spaces.
fn validate_select(columns: &str) -> bool {
    !columns.is_empty()
        && columns
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ',' | '*' | ' '))
}

/// Validates an ORDER BY clause. Each part must be `column [ASC|DESC]`.
fn validate_order_by(clause: &str) -> bool {
    if clause.is_empty() {
        return false;
    }
    clause.split(',').all(|part| {
        let tokens: Vec<&str> = part.split_whitespace().collect();
        match tokens.len() {
            1 => is_safe_identifier(tokens[0]),
            2 => {
                is_safe_identifier(tokens[0])
                    && matches!(tokens[1].to_ascii_uppercase().as_str(), "ASC" | "DESC")
            }
            _ => false,
        }
    })
}

/// Validates a filter condition fragment.
/// Rejects: comments (`--`, `/*`), semicolons, quotes.
fn validate_filter_condition(condition: &str) -> bool {
    if condition.is_empty() {
        return false;
    }
    if condition.contains("--") || condition.contains("/*") || condition.contains(';') {
        return false;
    }
    if condition.contains('\'') || condition.contains('"') {
        return false;
    }
    condition.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                '_' | '.' | ' ' | '?' | '=' | '<' | '>' | '!' | '(' | ')' | ','
            )
    })
}

/// Checks if a string is a safe SQL identifier (alphanumeric + `_` + `.`).
fn is_safe_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    type R = Result<(), Box<dyn std::error::Error>>;

    fn test_client() -> Client {
        Client::new(Config::new("http://localhost:8123", "test_db"))
    }

    fn test_tenant_id() -> Uuid {
        Uuid::parse_str("11111111-1111-1111-1111-111111111111")
            .unwrap_or_else(|e| panic!("invalid test UUID: {e}"))
    }

    #[test]
    fn bare_query_has_tenant_filter() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .to_sql();

        assert_eq!(
            sql,
            "SELECT * FROM gold.metrics WHERE insight_tenant_id = ?"
        );
        Ok(())
    }

    #[test]
    fn select_columns() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .select("name, value, created_at")?
            .to_sql();

        assert_eq!(
            sql,
            "SELECT name, value, created_at FROM gold.metrics WHERE insight_tenant_id = ?"
        );
        Ok(())
    }

    #[test]
    fn single_uuid_filter() -> R {
        let org_id = Uuid::parse_str("22222222-2222-2222-2222-222222222222")?;

        let sql = test_client()
            .tenant_query("silver.class_commits", test_tenant_id())?
            .filter("org_unit_id = ?", org_id)?
            .to_sql();

        assert_eq!(
            sql,
            "SELECT * FROM silver.class_commits WHERE insight_tenant_id = ? AND (org_unit_id = ?)"
        );
        Ok(())
    }

    #[test]
    fn multiple_filters_appended_with_and() -> R {
        let org_id = Uuid::parse_str("22222222-2222-2222-2222-222222222222")?;

        let sql = test_client()
            .tenant_query("gold.pr_cycle_time", test_tenant_id())?
            .filter("org_unit_id = ?", org_id)?
            .filter("metric_date >= ?", "2026-01-01")?
            .filter("metric_date < ?", "2026-04-01")?
            .to_sql();

        assert_eq!(
            sql,
            "SELECT * FROM gold.pr_cycle_time WHERE insight_tenant_id = ? \
             AND (org_unit_id = ?) AND (metric_date >= ?) AND (metric_date < ?)"
        );
        Ok(())
    }

    #[test]
    fn order_by_clause() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .order_by("created_at DESC")?
            .to_sql();

        assert_eq!(
            sql,
            "SELECT * FROM gold.metrics WHERE insight_tenant_id = ? ORDER BY created_at DESC"
        );
        Ok(())
    }

    #[test]
    fn limit_and_offset() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .limit(25)
            .offset(50)
            .to_sql();

        assert_eq!(
            sql,
            "SELECT * FROM gold.metrics WHERE insight_tenant_id = ? LIMIT 25 OFFSET 50"
        );
        Ok(())
    }

    #[test]
    fn full_query_with_all_clauses() -> R {
        let org_id = Uuid::parse_str("33333333-3333-3333-3333-333333333333")?;

        let sql = test_client()
            .tenant_query("gold.pr_cycle_time", test_tenant_id())?
            .select("person_id, avg_hours, metric_date")?
            .filter("org_unit_id = ?", org_id)?
            .filter("metric_date >= ?", "2026-01-01")?
            .filter("avg_hours > ?", 48)?
            .order_by("avg_hours DESC")?
            .limit(100)
            .offset(0)
            .to_sql();

        assert_eq!(
            sql,
            "SELECT person_id, avg_hours, metric_date \
             FROM gold.pr_cycle_time WHERE insight_tenant_id = ? \
             AND (org_unit_id = ?) AND (metric_date >= ?) AND (avg_hours > ?) \
             ORDER BY avg_hours DESC LIMIT 100 OFFSET 0"
        );
        Ok(())
    }

    #[test]
    fn tenant_id_is_always_first_filter() -> R {
        let sql = test_client()
            .tenant_query("silver.class_people", test_tenant_id())?
            .to_sql();

        assert!(sql.contains("WHERE insight_tenant_id = ?"));
        assert!(!sql.contains("AND"));
        Ok(())
    }

    #[test]
    fn float_filter() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .filter("value > ?", 99.5)?
            .to_sql();

        assert_eq!(
            sql,
            "SELECT * FROM gold.metrics WHERE insight_tenant_id = ? AND (value > ?)"
        );
        Ok(())
    }

    #[test]
    fn limit_only_no_offset() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .limit(10)
            .to_sql();

        assert_eq!(
            sql,
            "SELECT * FROM gold.metrics WHERE insight_tenant_id = ? LIMIT 10"
        );
        assert!(!sql.contains("OFFSET"));
        Ok(())
    }

    #[test]
    fn order_before_limit() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .order_by("name ASC")?
            .limit(50)
            .to_sql();

        let order_pos = sql.find("ORDER BY").ok_or("missing ORDER BY")?;
        let limit_pos = sql.find("LIMIT").ok_or("missing LIMIT")?;
        assert!(order_pos < limit_pos, "ORDER BY must come before LIMIT");
        Ok(())
    }

    #[test]
    fn different_tables_produce_different_sql() -> R {
        let tenant = test_tenant_id();
        let client = test_client();

        let sql_silver = client
            .tenant_query("silver.class_commits", tenant)?
            .to_sql();
        let sql_gold = client.tenant_query("gold.pr_cycle_time", tenant)?.to_sql();

        assert!(sql_silver.contains("silver.class_commits"));
        assert!(sql_gold.contains("gold.pr_cycle_time"));
        assert_ne!(sql_silver, sql_gold);
        Ok(())
    }

    // --- validation returns errors, not panics ---

    #[test]
    fn filter_no_placeholder_returns_error() -> R {
        let result = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .filter("status = 'active'", "unused");
        assert!(result.is_err());
        let err = result.err().ok_or("expected error")?;
        assert!(err.to_string().contains("placeholder"));
        Ok(())
    }

    #[test]
    fn filter_two_placeholders_returns_error() -> R {
        let result = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .filter("value BETWEEN ? AND ?", "unused");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn table_with_semicolon_returns_error() {
        let result = test_client().tenant_query("gold.metrics; DROP TABLE --", test_tenant_id());
        assert!(result.is_err());
    }

    #[test]
    fn table_with_spaces_returns_error() {
        let result = test_client().tenant_query("gold.metrics WHERE 1=1", test_tenant_id());
        assert!(result.is_err());
    }

    #[test]
    fn empty_table_returns_error() {
        let result = test_client().tenant_query("", test_tenant_id());
        assert!(result.is_err());
    }

    #[test]
    fn valid_dotted_table_name() -> R {
        let sql = test_client()
            .tenant_query("silver.class_commits", test_tenant_id())?
            .to_sql();
        assert!(sql.contains("FROM silver.class_commits"));
        Ok(())
    }

    #[test]
    fn filter_with_one_placeholder_ok() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .filter("status = ?", "active")?
            .to_sql();

        assert!(sql.contains("(status = ?)"));
        Ok(())
    }

    // --- order_by validation ---

    #[test]
    fn order_by_simple_column() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .order_by("metric_date DESC")?
            .to_sql();
        assert!(sql.contains("ORDER BY metric_date DESC"));
        Ok(())
    }

    #[test]
    fn order_by_multiple_columns() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .order_by("metric_date DESC, person_id ASC")?
            .to_sql();
        assert!(sql.contains("ORDER BY metric_date DESC, person_id ASC"));
        Ok(())
    }

    #[test]
    fn order_by_injection_semicolon_returns_error() -> R {
        let result = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .order_by("1; DROP TABLE gold.metrics --");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn order_by_injection_subquery_returns_error() -> R {
        let result = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .order_by("(SELECT 1)");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn order_by_injection_comment_returns_error() -> R {
        let result = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .order_by("metric_date -- comment");
        assert!(result.is_err());
        Ok(())
    }

    // --- select validation ---

    #[test]
    fn select_simple_columns() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .select("person_id, avg_hours")?
            .to_sql();
        assert!(sql.starts_with("SELECT person_id, avg_hours FROM"));
        Ok(())
    }

    #[test]
    fn select_star() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .select("*")?
            .to_sql();
        assert!(sql.starts_with("SELECT * FROM"));
        Ok(())
    }

    #[test]
    fn select_injection_subquery_returns_error() -> R {
        let result = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .select("*, (SELECT password FROM users) as x");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn select_injection_quotes_returns_error() -> R {
        let result = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .select("'1' as hack");
        assert!(result.is_err());
        Ok(())
    }

    // --- filter condition validation ---

    #[test]
    fn filter_injection_comment_returns_error() -> R {
        let result = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .filter("status = ? -- bypass", "active");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn filter_injection_semicolon_returns_error() -> R {
        let result = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .filter("status = ?; DROP TABLE gold.metrics", "x");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn filter_injection_quotes_returns_error() -> R {
        let result = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .filter("status = 'admin' OR '1'=?", "1");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn filter_with_in_clause() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .filter("org_unit_id IN (?)", "uuid1")?
            .to_sql();
        assert!(sql.contains("(org_unit_id IN (?))"));
        Ok(())
    }

    #[test]
    fn filter_with_comparison_operators() -> R {
        let sql = test_client()
            .tenant_query("gold.metrics", test_tenant_id())?
            .filter("value >= ?", 100)?
            .to_sql();
        assert!(sql.contains("(value >= ?)"));
        Ok(())
    }
}

use std::ops::ControlFlow;

use sqlparser::ast::{Query, Statement, Visit as _, Visitor};
use sqlparser::dialect::ClickHouseDialect;
use sqlparser::keywords::Keyword;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Token, Tokenizer};

const DENIED_TABLE_FUNCTIONS: &[&str] = &[
    "remote",
    "remotesecure",
    "cluster",
    "clusterallreplicas",
    "url",
    "urlcluster",
    "file",
    "filecluster",
    "s3",
    "s3cluster",
    "gcs",
    "hdfs",
    "hdfscluster",
    "azureblobstorage",
    "azureblobstoragecluster",
    "mysql",
    "postgresql",
    "jdbc",
    "odbc",
    "mongodb",
    "redis",
    "sqlite",
    "deltalake",
    "deltalakecluster",
    "hudi",
    "hudicluster",
    "iceberg",
    "icebergcluster",
    "executable",
    "merge",
    "dictionary",
];

const ADMIN_ONLY_DATABASES: &[&str] = &["product_usage"];

const ADMIN_ONLY_DATABASE_PREFIXES: &[&str] = &["bronze_"];

pub fn validate_single_select(sql: &str) -> Result<(), String> {
    let statements = parse_read_statements(sql)?;

    match statements.as_slice() {
        [] => Err("query is empty".to_owned()),
        [single] => match single {
            Statement::Query(_) => Ok(()),
            _ => Err("query must be a single SELECT or WITH statement".to_owned()),
        },
        _ => Err("only one statement is allowed on the query path".to_owned()),
    }
}

pub fn validate_mcp_sql(sql: &str) -> Result<(), String> {
    let statements = parse_read_statements(sql)?;

    let query = match statements.as_slice() {
        [] => return Err("query is empty".to_owned()),
        [Statement::Query(query)] => query,
        [_] => return Err("query must be a single SELECT or WITH statement".to_owned()),
        _ => return Err("only one statement is allowed on the query path".to_owned()),
    };

    match query_output_control(query) {
        Some(QueryOutputControl::Settings) => {
            return Err("query-level SETTINGS are not allowed".to_owned());
        }
        Some(QueryOutputControl::Format) => {
            return Err("query-level FORMAT is not allowed".to_owned());
        }
        None => {}
    }
    if let Some(name) = first_denied_table_function(sql) {
        return Err(format!("table function `{name}` is not allowed"));
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum QueryOutputControl {
    Settings,
    Format,
}

struct QueryOutputControlVisitor;

impl Visitor for QueryOutputControlVisitor {
    type Break = QueryOutputControl;

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        if query.settings.is_some() {
            return ControlFlow::Break(QueryOutputControl::Settings);
        }
        if query.format_clause.is_some() {
            return ControlFlow::Break(QueryOutputControl::Format);
        }
        ControlFlow::Continue(())
    }
}

fn query_output_control(query: &Query) -> Option<QueryOutputControl> {
    match query.visit(&mut QueryOutputControlVisitor) {
        ControlFlow::Break(control) => Some(control),
        ControlFlow::Continue(()) => None,
    }
}

pub fn validate_custom_observation_sql(sql: &str) -> Result<(), String> {
    validate_single_select(sql)?;

    if let Some(name) = admin_only_database(sql) {
        return Err(format!(
            "database `{name}` is not readable by a custom observation source"
        ));
    }

    if let Some(name) = first_denied_table_function(sql) {
        return Err(format!(
            "table function `{name}` is not allowed in a custom observation source"
        ));
    }

    Ok(())
}

// WORKAROUND: sqlparser (through 0.62) rejects ClickHouse's `FINAL` table
// modifier when it follows a table alias, so a parse failure is retried with
// the standalone `FINAL` keyword tokens blanked out. Only the retry input is
// altered — the SQL that reaches ClickHouse keeps its `FINAL`.
fn parse_read_statements(sql: &str) -> Result<Vec<Statement>, String> {
    let rejection = match Parser::parse_sql(&ClickHouseDialect {}, sql) {
        Ok(statements) => return Ok(statements),
        Err(e) => format!("query must be a single SELECT statement: {e}"),
    };

    let Some(stripped) = strip_final_modifiers(sql) else {
        return Err(rejection);
    };
    Parser::parse_sql(&ClickHouseDialect {}, &stripped).map_err(|_| rejection)
}

fn strip_final_modifiers(sql: &str) -> Option<String> {
    let tokens = Tokenizer::new(&ClickHouseDialect {}, sql).tokenize().ok()?;

    let mut stripped = String::with_capacity(sql.len());
    let mut found = false;
    let mut previous: Option<&Token> = None;
    for token in &tokens {
        if matches!(token, Token::Whitespace(_)) {
            stripped.push_str(&token.to_string());
            continue;
        }
        if is_final_keyword(token) && previous.is_some_and(ends_table_factor) {
            found = true;
            previous = Some(token);
            continue;
        }
        stripped.push_str(&token.to_string());
        previous = Some(token);
    }

    found.then_some(stripped)
}

fn is_final_keyword(token: &Token) -> bool {
    matches!(token, Token::Word(word)
        if word.keyword == Keyword::FINAL && word.quote_style.is_none())
}

fn ends_table_factor(token: &Token) -> bool {
    match token {
        Token::Word(word) => word.quote_style.is_some() || word.keyword == Keyword::NoKeyword,
        Token::RParen => true,
        _ => false,
    }
}

pub fn admin_only_database(sql: &str) -> Option<String> {
    let lowered = sql.to_ascii_lowercase();

    if let Some(database) = ADMIN_ONLY_DATABASES
        .iter()
        .copied()
        .find(|database| names(&lowered, database))
    {
        return Some(database.to_owned());
    }

    admin_only_prefixed_database(&lowered)
}

fn admin_only_prefixed_database(lowered: &str) -> Option<String> {
    let bytes = lowered.as_bytes();

    ADMIN_ONLY_DATABASE_PREFIXES.iter().find_map(|prefix| {
        lowered.match_indices(prefix).find_map(|(at, hit)| {
            let before = at.checked_sub(1).map(|index| bytes[index]);
            if before.is_some_and(|byte| continues_identifier(byte) || byte == b'.') {
                return None;
            }
            let mut end = at + hit.len();
            while bytes.get(end).copied().is_some_and(continues_identifier) {
                end += 1;
            }
            (end > at + hit.len()).then(|| lowered[at..end].to_owned())
        })
    })
}

fn names(lowered: &str, name: &str) -> bool {
    let bytes = lowered.as_bytes();

    lowered.match_indices(name).any(|(at, hit)| {
        let before = at.checked_sub(1).map(|index| bytes[index]);
        let after = bytes.get(at + hit.len()).copied();
        !before.is_some_and(continues_identifier) && !after.is_some_and(continues_identifier)
    })
}

fn continues_identifier(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn first_denied_table_function(sql: &str) -> Option<String> {
    let tokens = Tokenizer::new(&ClickHouseDialect {}, sql).tokenize().ok()?;

    let mut significant = tokens
        .iter()
        .filter(|token| !matches!(token, Token::Whitespace(_)));

    let mut previous: Option<&Token> = None;
    for token in significant.by_ref() {
        if !matches!(token, Token::LParen) {
            previous = Some(token);
            continue;
        }
        if let Some(Token::Word(word)) = previous {
            let lowered = word.value.to_ascii_lowercase();
            if DENIED_TABLE_FUNCTIONS.contains(&lowered.as_str()) {
                return Some(word.value.clone());
            }
        }
        previous = Some(token);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::admin_only_database as admin_only;
    use super::validate_custom_observation_sql as custom;
    use super::validate_mcp_sql as mcp;
    use super::validate_single_select as check;

    #[test]
    fn mcp_gate_rejects_query_output_controls() {
        assert_eq!(
            mcp("SELECT * FROM system.one SETTINGS max_threads = 1"),
            Err("query-level SETTINGS are not allowed".to_owned())
        );
        assert_eq!(
            mcp("SELECT * FROM system.one FORMAT JSON"),
            Err("query-level FORMAT is not allowed".to_owned())
        );
    }

    #[test]
    fn mcp_gate_rejects_settings_in_nested_queries() {
        for sql in [
            "WITH rows AS (SELECT id FROM silver.events SETTINGS max_threads = 1) SELECT * FROM rows",
            "SELECT * FROM (SELECT id FROM silver.events SETTINGS max_threads = 1)",
        ] {
            assert_eq!(
                mcp(sql),
                Err("query-level SETTINGS are not allowed".to_owned()),
                "should reject: {sql}"
            );
        }
    }

    #[test]
    fn mcp_gate_rejects_external_functions_and_non_queries() {
        assert_eq!(
            mcp("SELECT * FROM file('/etc/passwd', CSV)"),
            Err("table function `file` is not allowed".to_owned())
        );
        assert!(mcp("DROP TABLE silver.events").is_err());
        assert!(mcp("SELECT 1; SELECT 2").is_err());
    }

    #[test]
    fn mcp_gate_accepts_nested_read_queries() {
        for sql in [
            "SELECT name FROM system.tables LIMIT 5",
            "WITH rows AS (SELECT id FROM silver.events) SELECT * FROM rows",
            "SELECT * FROM (SELECT 1 AS value)",
        ] {
            assert!(mcp(sql).is_ok(), "should accept: {sql}");
        }
    }

    #[test]
    fn a_read_of_the_usage_event_store_is_admin_only() {
        for sql in [
            "SELECT person_id, path, ts FROM product_usage.usage_events LIMIT 5",
            "SELECT * FROM Product_Usage.Usage_Events",
            "SELECT * FROM `product_usage.usage_events`",
            "SELECT * FROM merge('product_usage', 'usage_events')",
        ] {
            assert_eq!(
                admin_only(sql),
                Some("product_usage".to_owned()),
                "should be admin-only: {sql:?}"
            );
        }
    }

    #[test]
    fn a_read_of_raw_bronze_is_admin_only() {
        for (sql, expected) in [
            ("SELECT * FROM bronze_jira.issues", "bronze_jira"),
            ("SELECT * FROM Bronze_Jira.Issues", "bronze_jira"),
            ("SELECT * FROM `bronze_slack.messages`", "bronze_slack"),
            (
                "SELECT * FROM bronze_not_yet_invented.things",
                "bronze_not_yet_invented",
            ),
        ] {
            assert_eq!(
                admin_only(sql),
                Some(expected.to_owned()),
                "should be admin-only: {sql:?}"
            );
        }
    }

    #[test]
    fn the_bare_bronze_prefix_is_not_a_database() {
        assert_eq!(admin_only("SELECT bronze_ FROM silver.events"), None);
    }

    #[test]
    fn a_custom_observation_source_may_not_read_raw_bronze() {
        let refusal = custom("SELECT ts FROM bronze_jira.issues").err();
        assert!(
            refusal
                .as_deref()
                .is_some_and(|reason| reason.contains("bronze_jira")),
            "a bronze read should be refused, naming the database: {refusal:?}"
        );
    }

    #[test]
    fn a_read_of_the_warehouse_contract_needs_no_role() {
        for sql in [
            "SELECT 1",
            "SELECT * FROM silver.events",
            "SELECT person_id FROM identity.identity_persons",
            "SELECT usage_events FROM silver.events",
            "SELECT product_usage_score FROM silver.events",
            "SELECT * FROM silver.bronze_backed_events",
        ] {
            assert_eq!(admin_only(sql), None, "should need no role: {sql:?}");
        }
    }

    #[test]
    fn a_custom_observation_source_may_not_read_an_admin_only_database() {
        assert!(custom("SELECT ts FROM product_usage.usage_events").is_err());
    }

    #[test]
    fn custom_gate_accepts_a_contract_shaped_read() {
        assert!(
            custom(
                "SELECT tenant_id, source_key, entity_type, entity_id, metric_date, measure_key, \
                 observed_at, value, subject_key, dimensions FROM silver.a JOIN gold.b USING (id)"
            )
            .is_ok()
        );
    }

    #[test]
    fn custom_gate_rejects_external_table_functions() {
        for sql in [
            "SELECT * FROM remote('host:9000', db.t)",
            "SELECT * FROM url('http://x/y', CSV)",
            "SELECT * FROM file('/etc/passwd', CSV)",
            "SELECT * FROM s3('s3://bucket/key', CSV)",
            "SELECT * FROM mysql('h', 'db', 't', 'u', 'p')",
            "WITH x AS (SELECT * FROM cluster('c', system.one)) SELECT * FROM x",
            "SELECT * FROM MERGE(currentDatabase(), '.*')",
            "SELECT * FROM fileCluster('c', '/x', CSV)",
            "SELECT * FROM s3Cluster('c', 's3://b/k', CSV)",
            "SELECT * FROM azureBlobStorageCluster('c', 'conn', 'ct', 'b')",
            "SELECT * FROM icebergCluster('c', 's3://b/k')",
            "SELECT * FROM deltaLakeCluster('c', 's3://b/k')",
            "SELECT * FROM hudiCluster('c', 's3://b/k')",
        ] {
            assert!(custom(sql).is_err(), "must reject external source: {sql:?}");
        }
    }

    #[test]
    fn custom_gate_allows_a_column_named_like_a_function() {
        assert!(custom("SELECT file FROM gold.events").is_ok());
        assert!(custom("SELECT value AS url FROM gold.events").is_ok());
    }

    #[test]
    fn accepts_final_on_a_replacing_merge_tree_read() {
        for sql in [
            "SELECT a FROM silver.t FINAL",
            "SELECT a FROM silver.t AS r FINAL",
            "SELECT a FROM silver.t r FINAL WHERE a > 1",
            "SELECT a FROM silver.a AS x FINAL JOIN gold.b AS y FINAL USING (id)",
            "WITH d AS (SELECT a FROM silver.t AS r FINAL) SELECT * FROM d",
            "SELECT final FROM silver.t AS r FINAL",
            "SELECT a, final FROM silver.t AS r FINAL WHERE final > 1",
            "SELECT r.final FROM silver.t AS r FINAL GROUP BY final",
        ] {
            assert!(check(sql).is_ok(), "should accept FINAL read: {sql:?}");
            assert!(custom(sql).is_ok(), "custom gate should accept: {sql:?}");
        }
    }

    #[test]
    fn final_as_a_plain_identifier_still_parses() {
        assert!(check("SELECT final FROM gold.events").is_ok());
        assert!(check("SELECT `final` AS f FROM gold.events").is_ok());
    }

    #[test]
    fn final_does_not_relax_the_gate() {
        assert!(check("SELECT a FROM t AS r FINAL; DROP TABLE t").is_err());
        assert!(check("SELECT a FROM t AS r FINAL b").is_err());
        assert!(custom("SELECT * FROM remote('h:9000', db.t) AS r FINAL").is_err());
    }

    #[test]
    fn custom_gate_still_enforces_single_select() {
        assert!(custom("SELECT 1; DROP TABLE t").is_err());
        assert!(custom("INSERT INTO t VALUES (1)").is_err());
    }

    #[test]
    fn accepts_read_queries() {
        for sql in [
            "SELECT 1",
            "select 1",
            "  SELECT * FROM t  ",
            "SELECT 1;",
            "SELECT count(*) FROM silver.events c JOIN person.map p ON c.id = p.id",
            "SELECT 1 UNION ALL SELECT 2",
            "WITH a AS (SELECT 1) SELECT * FROM a",
            "with a as (select 1) select * from a",
            "(SELECT 1)",
        ] {
            assert!(check(sql).is_ok(), "should accept: {sql:?}");
        }
    }

    #[test]
    fn accepts_comments_around_the_statement() {
        assert!(check("-- lead\nSELECT 1").is_ok());
        assert!(check("/* lead */ SELECT 1").is_ok());
        assert!(check("SELECT 1; -- trailing\n").is_ok());
        assert!(check("SELECT 1 /* trailing */").is_ok());
    }

    #[test]
    fn semicolons_and_keywords_inside_literals_are_inert() {
        assert!(check("SELECT ';' AS x").is_ok());
        assert!(check("SELECT 'a; DROP TABLE t' AS x FROM t").is_ok());
        assert!(check("SELECT 'it''s ok' AS x").is_ok());
    }

    #[test]
    fn rejects_empty() {
        for sql in ["", "   ", "\n\t ", "-- only a comment\n", "/* nothing */"] {
            assert!(check(sql).is_err(), "should reject empty: {sql:?}");
        }
    }

    #[test]
    fn rejects_non_read_statements() {
        for sql in [
            "INSERT INTO t VALUES (1)",
            "DROP TABLE t",
            "DELETE FROM t",
            "TRUNCATE TABLE t",
            "ALTER TABLE t ADD COLUMN c Int",
            "CREATE TABLE t (x Int)",
            "SET max_threads = 1",
            "SELECTX 1",
        ] {
            assert!(check(sql).is_err(), "should reject: {sql:?}");
        }
    }

    #[test]
    fn rejects_multiple_statements() {
        for sql in [
            "SELECT 1; SELECT 2",
            "SELECT 1; DROP TABLE t",
            "SELECT 1 ; INSERT INTO t VALUES (1)",
            "SELECT 'it''s'; SELECT 2",
        ] {
            assert!(
                check(sql).is_err(),
                "should reject multi-statement: {sql:?}"
            );
        }
    }
}

//! Single-SELECT gate for the public query path (#1962).
//!
//! Requires the SQL to parse (sqlparser, ClickHouse dialect) to exactly one read
//! statement — a `SELECT`/`WITH` query. Multiple statements, DDL/DML, and
//! unparseable input are rejected. Using a parser (not hand-rolled scanning)
//! keeps a `;` inside a string/comment/identifier from hiding a second
//! statement. Defense in depth: the `presentation_ro` grants (#1963) are the
//! real boundary.

use sqlparser::ast::Statement;
use sqlparser::dialect::ClickHouseDialect;
use sqlparser::keywords::Keyword;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Token, Tokenizer};

/// Table functions a custom observation source may not call. These reach data
/// outside the read-only warehouse contract — remote/clustered nodes, external
/// systems, and the local filesystem — so a custom SQL that used one could
/// exfiltrate to, or read tenant-crossing data from, a source the
/// `presentation_ro` grants and the outer tenant predicate cannot govern.
/// Matched case-insensitively as a bare `name(` call token pair.
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

/// Reject anything that is not a single read statement (`SELECT`/`WITH`).
/// Returns a short, user-facing reason on rejection.
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

/// Gate a custom observation source's SQL: a single read (as above) that calls
/// no external/remote table function. The compiler wraps this SQL as
/// `FROM (<sql>)` and executes it as `presentation_ro`; the outer tenant
/// predicate filters the rows it *emits*, not the tables it *reads*, so denying
/// the functions that escape the warehouse contract is what keeps a custom
/// source inside the same boundary a managed one has. Tenant-row isolation of
/// the warehouse relations themselves is the authorship-trust + experimental
/// gate, the same posture as the saved-query console.
pub fn validate_custom_observation_sql(sql: &str) -> Result<(), String> {
    validate_single_select(sql)?;

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

/// Rewrite `sql` with `FINAL` tokens in table-modifier position removed.
/// The tokenizer classifies keywords context-free, so an unquoted column or
/// alias named `final` also carries `Keyword::FINAL`; a table modifier is told
/// apart by what precedes it — a table name, alias, or closing paren — while
/// an identifier follows `SELECT`, a comma, a dot, an operator, or a clause
/// keyword. Returns `None` when the SQL has no such token (or cannot be
/// tokenized), so the caller retries the parse only when `FINAL` could have
/// been the reason it failed.
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

/// Return the first denied table-function name called in `sql`, if any. A call
/// is a denied identifier token immediately followed by `(`; a column or alias
/// merely *named* like one is not (it is not followed by a paren).
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
    use super::validate_custom_observation_sql as custom;
    use super::validate_single_select as check;

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
            // The *Cluster variants run the same reader across cluster nodes.
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
        // A denied name only matters as a `name(` call; a column or alias that
        // merely shares the name is not a table function.
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
            "SELECT 1 UNION ALL SELECT 2", // one statement, read-only
            "WITH a AS (SELECT 1) SELECT * FROM a",
            "with a as (select 1) select * from a",
            "(SELECT 1)", // a parenthesized query is still one read statement
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
            "SELECTX 1", // not valid SQL -> unparseable -> rejected
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
            "SELECT 'it''s'; SELECT 2", // real `;` after a doubled-quote escape
        ] {
            assert!(
                check(sql).is_err(),
                "should reject multi-statement: {sql:?}"
            );
        }
    }
}

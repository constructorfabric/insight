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
use sqlparser::parser::Parser;

/// Reject anything that is not a single read statement (`SELECT`/`WITH`).
/// Returns a short, user-facing reason on rejection.
pub fn validate_single_select(sql: &str) -> Result<(), String> {
    let statements = Parser::parse_sql(&ClickHouseDialect {}, sql)
        .map_err(|e| format!("query must be a single SELECT statement: {e}"))?;

    match statements.as_slice() {
        [] => Err("query is empty".to_owned()),
        [single] => match single {
            Statement::Query(_) => Ok(()),
            _ => Err("query must be a single SELECT or WITH statement".to_owned()),
        },
        _ => Err("only one statement is allowed on the query path".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_single_select as check;

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

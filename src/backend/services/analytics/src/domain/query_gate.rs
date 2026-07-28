//! Single-SELECT gate for the public query path (Phase A, #1962).
//!
//! The presentation layer accepts exactly one read-only statement on the public
//! query path — a single `SELECT` or `WITH ... SELECT`. This is a
//! parse-and-reject gate applied before any SQL reaches ClickHouse: on
//! saved-query write and on run. Today it also guards every metric `query_ref`
//! (the only raw-SQL surface that exists yet), validated identically on write
//! and run. Paired with the `presentation_ro` grants (#1963) it bounds the blast
//! radius of a broken or LLM-generated query to read paths only.
//!
//! Implemented with a real SQL parser (`sqlparser`, ClickHouse dialect) rather
//! than hand-rolled scanning: string literals, quoted identifiers, comments, and
//! escapes are the parser's concern, which removes the class of lexing bugs a
//! bespoke scanner is prone to (e.g. a `;` hidden past a mis-escaped quote). The
//! gate requires the input to parse to exactly one statement that is a read
//! query; anything else — DDL, DML, multiple statements, an unparseable
//! fragment — is rejected. The ClickHouse dialect may not accept every exotic
//! ClickHouse-only construct; an unparseable-but-valid query is therefore
//! rejected, which errs toward safety. The `presentation_ro` grants remain the
//! real safety boundary.

use sqlparser::ast::Statement;
use sqlparser::dialect::ClickHouseDialect;
use sqlparser::parser::Parser;

/// Reject anything that is not a single read statement (`SELECT` / `WITH ...`).
///
/// `Ok(())` for exactly one query statement; otherwise a short, user-facing
/// reason. A trailing `;` is allowed. Multiple statements, DDL/DML (`INSERT`,
/// `DROP`, `SET`, ...), and unparseable input are rejected.
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
        // A `;`, a `DROP`, or a quote-escape inside a string/identifier must not
        // be mistaken for a second statement — the parser tokenizes correctly.
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

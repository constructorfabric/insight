//! Alert rules and notifications hold a condition and a witness as JSON.
//!
//! A row written by `m20260928_000006_alerts` holds a column, an operator
//! and a threshold, and the one value a check read. Each becomes the
//! condition that means the same - exactly one row, one number test - and
//! the witness of that one value, before the old columns go.
//!
//! INVARIANT: every step is safe to run again. Columns are added and
//! dropped with `IF [NOT] EXISTS`, the rewrite runs only while the old
//! columns are there, and it writes the same JSON from the same row. Every
//! rule is rewritten, so a check or an edit the previous build wrote before
//! a rerun is taken up; notifications are only ever added, so only those
//! without a condition are.
//!
//! INVARIANT: the JSON written here is spelled out, not built by the
//! domain types, so a later change to them never changes what this
//! migration wrote.

use sea_orm::{
    ConnectionTrait as _, DbBackend, DbErr, FromQueryResult, Statement, TransactionTrait as _,
};
use sea_orm_migration::prelude::*;
use serde_json::json;

use crate::domain::alerts::Number;

const ADD_COLUMNS: [&str; 4] = [
    "ALTER TABLE alert_rules ADD COLUMN IF NOT EXISTS rule_condition MEDIUMTEXT NULL AFTER metric",
    "ALTER TABLE alert_rules ADD COLUMN IF NOT EXISTS last_witness MEDIUMTEXT NULL AFTER last_reason",
    "ALTER TABLE alert_notifications ADD COLUMN IF NOT EXISTS rule_condition MEDIUMTEXT NULL AFTER metric",
    "ALTER TABLE alert_notifications ADD COLUMN IF NOT EXISTS witness MEDIUMTEXT NULL AFTER rule_condition",
];

/// A column made NOT NULL over a NULL fails here rather than filling it
/// with an empty string no reader can parse, so a rerun converts the row.
const STRICT_SESSION: &str =
    "SET SESSION sql_mode = CONCAT_WS(',', @@SESSION.sql_mode, 'STRICT_TRANS_TABLES')";

const REQUIRE_COLUMNS: [&str; 2] = [
    "ALTER TABLE alert_rules MODIFY COLUMN rule_condition MEDIUMTEXT NOT NULL",
    "ALTER TABLE alert_notifications MODIFY COLUMN rule_condition MEDIUMTEXT NOT NULL, MODIFY COLUMN witness MEDIUMTEXT NOT NULL",
];

/// A valid check the previous build recorded after the guard and before
/// the drop carries no witness; it is forgotten, as an edit forgets one,
/// and the next check records it again.
const FORGET_CHECKS_WITHOUT_A_WITNESS: &str = "UPDATE alert_rules SET last_evaluated_at = NULL, last_outcome = NULL, last_reason = NULL, last_valid_breached = NULL, breached_since = NULL WHERE last_outcome IN ('breach', 'no_breach') AND last_witness IS NULL";

const DROP_COLUMNS: [&str; 2] = [
    "ALTER TABLE alert_rules DROP COLUMN IF EXISTS column_name, DROP COLUMN IF EXISTS operator, DROP COLUMN IF EXISTS threshold, DROP COLUMN IF EXISTS last_value",
    "ALTER TABLE alert_notifications DROP COLUMN IF EXISTS column_name, DROP COLUMN IF EXISTS operator, DROP COLUMN IF EXISTS threshold, DROP COLUMN IF EXISTS value",
];

/// Held until the rewrite commits, so a write of the previous build waits
/// for it instead of landing between the read and the rewrite.
const SELECT_RULES: &str = "SELECT id, column_name, operator, threshold, last_outcome, last_value FROM alert_rules FOR UPDATE";
const UPDATE_RULE: &str =
    "UPDATE alert_rules SET rule_condition = ?, last_witness = ? WHERE id = ?";
const SELECT_NOTIFICATIONS: &str = "SELECT id, column_name, operator, threshold, value FROM alert_notifications WHERE rule_condition IS NULL";
const UPDATE_NOTIFICATION: &str =
    "UPDATE alert_notifications SET rule_condition = ?, witness = ? WHERE id = ?";
/// Rows the rewrite has not caught up with: a writer of the previous build
/// may have added a rule, or recorded a valid check, after it.
const UNCONVERTED: [&str; 2] = [
    "SELECT COUNT(*) AS total FROM alert_rules WHERE rule_condition IS NULL OR (last_outcome IN ('breach', 'no_breach') AND last_witness IS NULL)",
    "SELECT COUNT(*) AS total FROM alert_notifications WHERE rule_condition IS NULL OR witness IS NULL",
];

/// The operators the previous build wrote.
const OPERATORS: [&str; 4] = [">", ">=", "<", "<="];

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &'static str {
        "m20261014_000007_alert_conditions"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for statement in ADD_COLUMNS {
            db.execute_unprepared(statement).await?;
        }

        let rules_unconverted = manager.has_column("alert_rules", "threshold").await?;
        let notifications_unconverted = manager
            .has_column("alert_notifications", "threshold")
            .await?;
        if rules_unconverted {
            rewrite_rules(manager).await?;
        }
        if notifications_unconverted {
            rewrite_notifications(manager).await?;
        }
        // INVARIANT: the guard only means anything while a drop is still
        // ahead; once the old columns are gone, what it would refuse is
        // what the heal below repairs.
        if rules_unconverted || notifications_unconverted {
            refuse_unconverted(manager).await?;
        }

        // INVARIANT: the session setting holds only on the connection that
        // made it, so the statements that rely on it share that connection.
        let session = db.begin().await?;
        session.execute_unprepared(STRICT_SESSION).await?;
        for statement in REQUIRE_COLUMNS.into_iter().chain(DROP_COLUMNS) {
            session.execute_unprepared(statement).await?;
        }
        session.commit().await?;

        db.execute_unprepared(FORGET_CHECKS_WITHOUT_A_WITNESS)
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "a condition cannot be written back as one column and one threshold".to_owned(),
        ))
    }
}

/// Stops before the columns are required while any row lacks a value: a
/// server outside strict mode would otherwise fill it with an empty string
/// no reader can parse. Running again rewrites it.
async fn refuse_unconverted(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for sql in UNCONVERTED {
        let counted = TotalRow::find_by_statement(Statement::from_string(DbBackend::MySql, sql))
            .one(manager.get_connection())
            .await?;
        if counted.is_some_and(|row| row.total > 0) {
            return Err(DbErr::Custom(format!(
                "rows were written without a condition while this ran; run it again: {sql}"
            )));
        }
    }

    Ok(())
}

#[derive(Debug, FromQueryResult)]
struct TotalRow {
    total: i64,
}

#[derive(Debug, FromQueryResult)]
struct RuleRow {
    id: String,
    column_name: String,
    operator: String,
    threshold: String,
    last_outcome: Option<String>,
    last_value: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct NotificationRow {
    id: String,
    column_name: String,
    operator: String,
    threshold: String,
    value: String,
}

async fn rewrite_rules(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let transaction = manager.get_connection().begin().await?;
    let rows = RuleRow::find_by_statement(Statement::from_string(DbBackend::MySql, SELECT_RULES))
        .all(&transaction)
        .await?;

    for row in rows {
        let condition = condition_of(&row.column_name, &row.operator, &row.threshold)
            .ok_or_else(|| unreadable("alert rule", &row.id))?;
        let witness = match (row.last_outcome.as_deref(), row.last_value.as_deref()) {
            (Some(outcome @ ("breach" | "no_breach")), Some(value)) => Some(
                witness_of(&row.column_name, value, outcome == "breach")
                    .ok_or_else(|| unreadable("alert rule", &row.id))?,
            ),
            _ => None,
        };

        transaction
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                UPDATE_RULE,
                [condition.into(), witness.into(), row.id.into()],
            ))
            .await?;
    }

    transaction.commit().await
}

async fn rewrite_notifications(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let transaction = manager.get_connection().begin().await?;
    let rows = NotificationRow::find_by_statement(Statement::from_string(
        DbBackend::MySql,
        SELECT_NOTIFICATIONS,
    ))
    .all(&transaction)
    .await?;

    for row in rows {
        let condition = condition_of(&row.column_name, &row.operator, &row.threshold)
            .ok_or_else(|| unreadable("alert notification", &row.id))?;
        let witness = witness_of(&row.column_name, &row.value, true)
            .ok_or_else(|| unreadable("alert notification", &row.id))?;

        transaction
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::MySql,
                UPDATE_NOTIFICATION,
                [condition.into(), witness.into(), row.id.into()],
            ))
            .await?;
    }

    transaction.commit().await
}

fn unreadable(what: &str, id: &str) -> DbErr {
    DbErr::Custom(format!(
        "{what} {id} holds a test this migration cannot read"
    ))
}

/// The stored condition one column, operator and threshold meant:
/// exactly one row, and its number compared.
fn condition_of(column: &str, operator: &str, threshold: &str) -> Option<String> {
    if !OPERATORS.contains(&operator) {
        return None;
    }
    let threshold = Number::from_stored(threshold)?;

    let stored = json!({
        "condition": {
            "rows": "one",
            "where": { "column": column, "op": operator, "value": threshold.to_json() },
        },
        "kinds": { column: "number" },
    });

    Some(stored.to_string())
}

/// The stored witness of the one value a check read.
fn witness_of(column: &str, value: &str, breached: bool) -> Option<String> {
    let value = Number::from_stored(value)?;

    let stored = json!({
        "matched": u64::from(breached),
        "answered": 1,
        "columns": [column],
        "rows": [[value.to_json()]],
    });

    Some(stored.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::domain::alerts::condition::{Condition, Witness};

    fn parsed(stored: &str) -> Value {
        serde_json::from_str(stored).unwrap_or_else(|error| panic!("{stored}: {error}"))
    }

    #[test]
    fn a_column_operator_and_threshold_become_the_one_row_condition_they_meant() {
        let cases = [
            (">", "50", json!(50)),
            ("<=", "0.5", json!(0.5)),
            (">=", "1e-7", json!(1e-7)),
            (
                "<",
                "-170141183460469231731687303715884105728",
                json!("-170141183460469231731687303715884105728"),
            ),
        ];

        for (operator, threshold, value) in cases {
            let stored = condition_of("total", operator, threshold)
                .unwrap_or_else(|| panic!("should read {operator} {threshold}"));
            let expected = json!({
                "rows": "one",
                "where": { "column": "total", "op": operator, "value": value },
            });

            assert_eq!(
                parsed(&stored)["condition"],
                expected,
                "for {operator} {threshold}"
            );
            let read = Condition::from_stored(&stored)
                .unwrap_or_else(|| panic!("the service reads what this wrote: {stored}"));
            assert_eq!(read.to_json(), expected);
        }
    }

    #[test]
    fn a_row_this_service_never_wrote_is_not_guessed_at() {
        assert!(condition_of("total", "~", "50").is_none());
        assert!(condition_of("total", "==", "50").is_none());
        assert!(condition_of("total", ">", "fifty").is_none());
        assert!(witness_of("total", "NaN", true).is_none());
    }

    #[test]
    fn the_value_a_check_read_becomes_a_one_row_witness_the_service_reads() {
        let stored = witness_of("total", "63", true).unwrap_or_else(|| panic!("reads 63"));

        assert_eq!(
            Witness::from_json(&parsed(&stored)),
            Some(Witness::one_value("total", json!(63), true))
        );
        let cleared = witness_of("total", "2.5", false).unwrap_or_else(|| panic!("reads 2.5"));
        assert_eq!(parsed(&cleared)["matched"], json!(0));
    }

    #[test]
    fn the_old_columns_go_only_once_every_row_is_converted() {
        for statement in DROP_COLUMNS {
            assert!(
                statement.contains("DROP COLUMN IF EXISTS threshold"),
                "{statement}"
            );
        }
        for statement in ADD_COLUMNS {
            assert!(
                statement.contains("ADD COLUMN IF NOT EXISTS"),
                "{statement}"
            );
        }
        assert!(!SELECT_RULES.contains("WHERE"), "every rule is rewritten");
        assert!(SELECT_RULES.ends_with("FOR UPDATE"));
        assert!(FORGET_CHECKS_WITHOUT_A_WITNESS.contains("last_witness IS NULL"));
        assert!(SELECT_NOTIFICATIONS.ends_with("WHERE rule_condition IS NULL"));
        assert!(UNCONVERTED[0].contains("last_witness IS NULL"));
        assert!(
            UNCONVERTED
                .iter()
                .all(|sql| sql.contains("rule_condition IS NULL"))
        );
    }
}

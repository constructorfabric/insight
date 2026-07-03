//! Startup probe for MariaDB CHECK-constraint enforcement.
//!
//! Refs #519 / `cpt-metric-cat-constraint-mariadb-check`.
//!
//! Our own bitnami-shipped MariaDB is currently 11.x and enforces CHECKs
//! correctly, so the probe is rarely load-bearing on a stock Insight deploy.
//! It still earns its keep because:
//!
//! - **Customer-managed databases.** On-prem and BYO-DB installs may point
//!   Insight at a managed MariaDB the customer operates (RDS, Cloud SQL,
//!   on-prem cluster). We can't audit their version, `sql_mode`, or whether
//!   `--skip-character-set-client-handshake` style flags are in play.
//!   MariaDB 10.2 parses CHECK constraints but enforcement is unreliable
//!   until 10.3, with IS-NULL predicate edge cases still being closed in
//!   later 10.x point releases — without this probe, a customer on an old
//!   managed instance would silently degrade our DB-layer validation to
//!   app-layer-only (and direct-SQL or future-migration writes bypass the
//!   app layer entirely).
//! - **Drift after the fact.** A DBA running `ALTER TABLE ... DROP CONSTRAINT`
//!   to unblock an incident and forgetting to re-add it; a future migration
//!   accidentally dropping a required CHECK; a rollback that demotes the
//!   schema. All silent failures without this probe.
//!
//! The probe runs once at service boot. It queries
//! `INFORMATION_SCHEMA.CHECK_CONSTRAINTS` in the current schema and refuses to
//! start if any required CHECK is missing. It reports every missing CHECK in a
//! single error so a single bad deploy doesn't require N restarts to surface
//! all gaps.

use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement, Value};

use crate::migration::REQUIRED_CHECKS_BY_TABLE;

#[derive(FromQueryResult)]
struct ConstraintRow {
    constraint_name: String,
}

/// Verify that every CHECK constraint required by the metric-catalog schema is
/// present in the connected database.
///
/// # Errors
///
/// Returns an error listing every missing CHECK (per table) if any are absent.
/// Returns an error if the `INFORMATION_SCHEMA` query itself fails.
pub async fn assert_required_checks(db: &DatabaseConnection) -> anyhow::Result<()> {
    let backend = db.get_database_backend();
    let mut missing: Vec<(String, String)> = Vec::new();

    for (table, required) in REQUIRED_CHECKS_BY_TABLE {
        if required.is_empty() {
            continue;
        }

        let rows = ConstraintRow::find_by_statement(Statement::from_sql_and_values(
            backend,
            "SELECT CONSTRAINT_NAME AS constraint_name \
             FROM INFORMATION_SCHEMA.CHECK_CONSTRAINTS \
             WHERE CONSTRAINT_SCHEMA = DATABASE() AND TABLE_NAME = ?",
            [Value::from(*table)],
        ))
        .all(db)
        .await?;

        // Case-fold both sides. MariaDB's default collation on
        // `INFORMATION_SCHEMA.CHECK_CONSTRAINTS.CONSTRAINT_NAME` is
        // case-insensitive (`utf8_general_ci`), but Rust's `HashSet<&str>` is
        // not — so a CHECK manually re-added as `CHK_FOO_BAR` would otherwise
        // read as missing even though MariaDB treats it as the same constraint.
        let present: std::collections::HashSet<String> = rows
            .iter()
            .map(|r| r.constraint_name.to_ascii_lowercase())
            .collect();

        for name in *required {
            if !present.contains(&name.to_ascii_lowercase()) {
                missing.push(((*table).to_owned(), (*name).to_owned()));
            }
        }
    }

    if missing.is_empty() {
        tracing::info!("CHECK-constraint probe: all required checks present");
        return Ok(());
    }

    let summary = missing
        .iter()
        .map(|(t, n)| format!("{t}.{n}"))
        .collect::<Vec<_>>()
        .join(", ");

    tracing::error!(
        missing = %summary,
        "required CHECK constraints missing — refusing to start \
         (MariaDB 10.3+ required; see DESIGN §2.2 cpt-metric-cat-constraint-mariadb-check)"
    );

    Err(anyhow::anyhow!(
        "required CHECK constraints missing in MariaDB: {summary}. \
         Service requires MariaDB 10.3+ with CHECK enforcement enabled."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_checks_table_is_non_empty() {
        // Sanity: at least one table contributes at least one CHECK. If this
        // ever returns empty, the probe degrades to a no-op without a compile
        // error — explicit guard.
        let total: usize = REQUIRED_CHECKS_BY_TABLE
            .iter()
            .map(|(_, checks)| checks.len())
            .sum();
        assert!(total > 0, "expected at least one required CHECK to probe");
    }

    #[test]
    fn no_duplicate_check_names_across_tables() {
        // CHECK names are global within a schema in MariaDB. Two tables can't
        // share a CHECK name without the second CREATE failing. Catch this at
        // unit-test time instead of at migration time.
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (table, checks) in REQUIRED_CHECKS_BY_TABLE {
            for name in *checks {
                assert!(
                    seen.insert(name),
                    "CHECK name {name} (table {table}) collides with another table — \
                     MariaDB requires CHECK names to be unique per schema"
                );
            }
        }
    }

    #[test]
    fn check_names_use_chk_prefix_convention() {
        // The probe relies on a stable naming scheme. If a future migration
        // forgets the `chk_` prefix the convention silently drifts; this test
        // surfaces that before review.
        for (_table, checks) in REQUIRED_CHECKS_BY_TABLE {
            for name in *checks {
                assert!(
                    name.starts_with("chk_"),
                    "CHECK name {name} must use the chk_ prefix"
                );
            }
        }
    }
}

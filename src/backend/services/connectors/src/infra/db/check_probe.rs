use sea_orm::{DatabaseConnection, FromQueryResult, Statement, Value};

use crate::migration::REQUIRED_CHECKS_BY_TABLE;

#[derive(FromQueryResult)]
struct ConstraintRow {
    constraint_name: String,
}

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
         (MariaDB 10.3+ required)"
    );

    Err(anyhow::anyhow!(
        "required CHECK constraints missing in MariaDB: {summary}. \
         Service requires MariaDB 10.3+ with CHECK enforcement enabled."
    ))
}

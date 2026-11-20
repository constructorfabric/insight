use std::time::Duration;

use async_trait::async_trait;
use clickhouse::Row;
use insight_clickhouse::{Client, Config};
use sea_orm::prelude::DateTime;
use serde::Deserialize;
use uuid::Uuid;

const READ_TIMEOUT: Duration = Duration::from_mins(5);

use crate::domain::seed::IdentityInputRow;
use crate::domain::seed_service::IdentityInputsReader;

/// `toString` of a `Nullable` column stays `Nullable`, which the strict decoder rejects
/// against a `String` field, hence `ifNull`. Aliases differ from the column names: a
/// same-name `ifNull(value, '') AS value` shadows the `value` in `WHERE` and can trip
/// ClickHouse's "Cyclic aliases" error.
const STREAM_SQL: &str = r"
    SELECT
        ifNull(insight_source_type, '')  AS source_type,
        ifNull(toString(insight_source_id), '') AS source_id,
        source_account_id                AS account_id,
        ifNull(value_type, '')           AS val_type,
        ifNull(value, '')                AS val,
        toString(_synced_at)             AS synced_at,
        ifNull(operation_type, '')       AS op_type
    FROM identity.identity_inputs
    WHERE (operation_type = 'UPSERT' AND value IS NOT NULL AND value != '')
       OR operation_type = 'DELETE'
    ORDER BY
        insight_source_type,
        insight_source_id,
        source_account_id,
        _synced_at DESC,
        value_type,
        value
";

#[derive(Debug, Row, Deserialize)]
struct InputRow {
    source_type: String,
    source_id: String,
    account_id: Option<String>,
    val_type: String,
    val: String,
    synced_at: String,
    op_type: String,
}

pub struct ClickHouseIdentityInputsReader {
    client: Client,
}

impl ClickHouseIdentityInputsReader {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    #[must_use]
    pub fn connect(url: &str, database: &str, user: &str, password: &str) -> Self {
        let mut config = Config::new(url, database).with_query_timeout(READ_TIMEOUT);
        if !user.is_empty() {
            config = config.with_auth(user, password);
        }
        Self::new(Client::new(config))
    }
}

#[async_trait]
impl IdentityInputsReader for ClickHouseIdentityInputsReader {
    async fn stream(&self, tenant_id: Uuid) -> anyhow::Result<Vec<IdentityInputRow>> {
        // SAFETY: tenancy shortcut (#1550): reads every tenant's rows, because the producer hashes tenant
        // ids and there is nothing to filter on yet. Single-tenant deployments only.
        let _ = tenant_id;
        let rows: Vec<InputRow> = self.client.query(STREAM_SQL).fetch_all().await?;
        rows.into_iter().map(map_row).collect()
    }
}

fn map_row(r: InputRow) -> anyhow::Result<IdentityInputRow> {
    let account_id = r.account_id.ok_or_else(|| {
        anyhow::anyhow!(
            "NULL source_account_id in identity_inputs (source_type={}, source_id={}, \
             value_type={}): refusing to fold it into a '' pseudo-account; fix the producer row",
            r.source_type,
            r.source_id,
            r.val_type,
        )
    })?;
    Ok(IdentityInputRow {
        source_type: r.source_type,
        source_id: Uuid::parse_str(&r.source_id)?,
        source_account_id: account_id,
        value_type: r.val_type,
        value: r.val,
        synced_at: parse_ch_datetime(&r.synced_at)?,
        is_delete: r.op_type == "DELETE",
    })
}

fn parse_ch_datetime(s: &str) -> anyhow::Result<DateTime> {
    DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S"))
        .map_err(|e| anyhow::anyhow!("unparseable _synced_at '{s}': {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_sql_keeps_empty_value_delete_rows() {
        assert!(
            STREAM_SQL.contains("OR operation_type = 'DELETE'"),
            "DELETE closure signals carry an empty value and must not be value-filtered"
        );
        assert!(
            STREAM_SQL.contains("operation_type = 'UPSERT' AND value IS NOT NULL"),
            "the non-empty filter applies to UPSERT rows only"
        );
    }

    #[test]
    fn parses_clickhouse_datetime_with_and_without_fraction() -> anyhow::Result<()> {
        let with_frac = parse_ch_datetime("2026-07-16 12:34:56.123456")?;
        let no_frac = parse_ch_datetime("2026-07-16 12:34:56")?;
        assert_eq!(
            with_frac.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-07-16 12:34:56"
        );
        assert_eq!(
            no_frac.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-07-16 12:34:56"
        );
        assert!(parse_ch_datetime("not-a-date").is_err());
        Ok(())
    }

    #[test]
    fn a_null_account_id_fails_the_read_rather_than_minting_a_pseudo_account() -> anyhow::Result<()>
    {
        let row = InputRow {
            source_type: "bamboohr".to_owned(),
            source_id: Uuid::now_v7().to_string(),
            account_id: None,
            val_type: "email".to_owned(),
            val: "person@inputs.test".to_owned(),
            synced_at: "2026-01-02 03:04:05.678".to_owned(),
            op_type: "UPSERT".to_owned(),
        };

        let refused = map_row(row)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();

        anyhow::ensure!(
            refused.contains("NULL source_account_id"),
            "an accountless row must name itself in the failure, not fold into '': {refused:?}"
        );
        Ok(())
    }
}

use super::*;

pub(crate) async fn reader_or_skip() -> anyhow::Result<Option<ClickHouseIdentityInputsReader>> {
    let Ok(url) = std::env::var("INTEGRATION_TESTS_IDENTITY_INPUTS_URL") else {
        return Ok(None);
    };
    let reader = ClickHouseIdentityInputsReader::connect(&url, "default", "", "");
    reader
        .client
        .query("CREATE DATABASE IF NOT EXISTS identity")
        .execute()
        .await?;
    reader.client.query("CREATE TABLE IF NOT EXISTS identity.identity_inputs (insight_source_type String, insight_source_id UUID, source_account_id Nullable(String), value_type String, value Nullable(String), _synced_at DateTime64(6), operation_type String, _version UInt64) ENGINE = MergeTree ORDER BY (insight_source_type, insight_source_id)").execute().await?;
    Ok(Some(reader))
}

pub(crate) async fn insert_profile(
    reader: &ClickHouseIdentityInputsReader,
    profile: &crate::domain::seed::SeedProfile,
) -> anyhow::Result<()> {
    let account = &profile.account;
    let mut rows: Vec<_> = profile
        .observations
        .iter()
        .map(|row| (row.value_type.as_str(), row.value.as_str(), row.is_delete))
        .collect();
    if let Some(email) = &profile.latest_email {
        rows.push(("email", email, false));
    }
    if let Some(membership) = profile.roster_membership {
        rows.push(("roster_membership", "active", !membership.active));
    }
    for (field, value, deleted) in rows {
        reader.client.query("INSERT INTO identity.identity_inputs VALUES (?, toUUID(?), ?, ?, ?, toDateTime64('2026-01-01 00:00:00', 6), ?, 1)")
            .bind(&account.source_type).bind(account.source_id.to_string()).bind(&account.account_id)
            .bind(field).bind(value).bind(if deleted { "DELETE" } else { "UPSERT" }).execute().await?;
    }
    Ok(())
}

#[tokio::test]
async fn email_dependencies_use_latest_values_and_preserve_source_and_instance_scope()
-> anyhow::Result<()> {
    let Some(reader) = reader_or_skip().await? else {
        return Ok(());
    };
    let source = Uuid::now_v7();
    let other = Uuid::now_v7();
    for (kind, instance, account, email, operation, version) in [
        (
            "directory",
            source,
            "match",
            " Manager@Example.Test ",
            "UPSERT",
            1_u64,
        ),
        (
            "directory",
            source,
            "changed",
            "manager@example.test",
            "UPSERT",
            1,
        ),
        (
            "directory",
            source,
            "changed",
            "different@example.test",
            "UPSERT",
            2,
        ),
        (
            "directory",
            source,
            "deleted",
            "manager@example.test",
            "UPSERT",
            1,
        ),
        ("directory", source, "deleted", "", "DELETE", 2),
        (
            "activity",
            source,
            "wrong-source",
            "manager@example.test",
            "UPSERT",
            1,
        ),
        (
            "directory",
            other,
            "wrong-instance",
            "manager@example.test",
            "UPSERT",
            1,
        ),
    ] {
        reader.client.query("INSERT INTO identity.identity_inputs VALUES (?, toUUID(?), ?, 'email', ?, toDateTime64('2026-01-01 00:00:00', 6), ?, ?)")
            .bind(kind).bind(instance.to_string()).bind(account).bind(email).bind(operation).bind(version).execute().await?;
    }
    let accounts = reader
        .email_accounts(&[SourceEmail {
            source_type: "directory".to_owned(),
            source_id: source,
            email: "manager@example.test".to_owned(),
        }])
        .await?;
    assert_eq!(
        accounts,
        vec![SourceAccountKey {
            source_type: "directory".to_owned(),
            source_id: source,
            account_id: "match".to_owned()
        }]
    );
    Ok(())
}

#[tokio::test]
async fn account_reads_scope_native_ids_and_keep_the_latest_explicit_clear() -> anyhow::Result<()> {
    let Ok(url) = std::env::var("INTEGRATION_TESTS_IDENTITY_INPUTS_URL") else {
        return Ok(());
    };
    let reader = ClickHouseIdentityInputsReader::connect(&url, "default", "", "");
    reader
        .client
        .query("CREATE DATABASE IF NOT EXISTS identity")
        .execute()
        .await?;
    reader.client.query("CREATE TABLE IF NOT EXISTS identity.identity_inputs (insight_source_type String, insight_source_id UUID, source_account_id Nullable(String), value_type String, value Nullable(String), _synced_at DateTime64(6), operation_type String, _version UInt64) ENGINE = MergeTree ORDER BY (insight_source_type, insight_source_id)").execute().await?;
    let source_id = Uuid::now_v7();
    for (kind, operation, value, version) in [
        ("directory", "UPSERT", "Old Name", 1_u64),
        ("directory", "DELETE", "", 2),
        ("activity", "UPSERT", "Other source", 3),
    ] {
        reader.client.query("INSERT INTO identity.identity_inputs VALUES (?, toUUID(?), 'shared-id', 'person_display_name', ?, toDateTime64('2026-01-01 00:00:00', 6), ?, ?)")
            .bind(kind).bind(source_id.to_string()).bind(value).bind(operation).bind(version).execute().await?;
    }
    let account = SourceAccountKey {
        source_type: "directory".to_owned(),
        source_id,
        account_id: "shared-id".to_owned(),
    };
    let rows = reader.accounts(&[account]).await?;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_delete);
    assert!(rows[0].value.is_empty());
    Ok(())
}

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
fn a_null_account_id_fails_the_read_rather_than_minting_a_pseudo_account() -> anyhow::Result<()> {
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

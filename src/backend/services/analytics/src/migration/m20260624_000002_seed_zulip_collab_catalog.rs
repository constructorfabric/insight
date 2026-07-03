//! Seed the `metric_catalog` + product-default `metric_threshold` row for the
//! Zulip chat counter `collab_bullet_rows.zulip_messages_sent` introduced in
//! `m20260624_000001_collab_zulip_chat` (paired gold branch in
//! `20260518000000_collab-bullet-rewrite.sql`, Branch 4b).
//!
//! One `metric_key`, routed through the existing `collab_bullet_rows` section,
//! `source_tags = ["zulip-proxy"]`. Mirrors `m20260620_000002_seed_wiki_catalog`
//! (additive catalog seed). The threshold is a product-default placeholder;
//! tune per tenant via the admin CRUD API.

use sea_orm::{ConnectionTrait, Statement, Value};
use sea_orm_migration::prelude::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub struct Migration;

struct SeedRow {
    metric_key: &'static str,
    label: &'static str,
    sublabel: Option<&'static str>,
    description: Option<&'static str>,
    unit: Option<&'static str>,
    format: Option<&'static str>,
    higher_is_better: bool,
    is_member_scale: bool,
    source_tags: &'static [&'static str],
    good: f64,
    warn: f64,
}

fn source_tags_json(tags: &[&'static str]) -> String {
    let mut out = String::from("[");
    for (i, tag) in tags.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        for c in tag.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                _ => out.push(c),
            }
        }
        out.push('"');
    }
    out.push(']');
    out
}

const SEEDS: &[SeedRow] = &[SeedRow {
    metric_key: "collab_bullet_rows.zulip_messages_sent",
    label: "Zulip Messages",
    sublabel: Some("Zulip \u{b7} chat messages sent \u{b7} period total"),
    description: Some("Chat messages sent by the person in the period (Zulip)."),
    unit: Some("messages"),
    format: None,
    higher_is_better: true,
    is_member_scale: false,
    source_tags: &["zulip-proxy"],
    good: 50.0,
    warn: 20.0,
}];

const INSERT_CATALOG_SQL: &str = "\
    INSERT INTO metric_catalog \
        (id, tenant_id, metric_key, label, sublabel, description, unit, format, \
         higher_is_better, is_member_scale, source_tags, is_enabled) \
    VALUES (?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, TRUE) \
    ON DUPLICATE KEY UPDATE \
        label = VALUES(label), \
        sublabel = VALUES(sublabel), \
        description = VALUES(description), \
        unit = VALUES(unit), \
        format = VALUES(format), \
        higher_is_better = VALUES(higher_is_better), \
        is_member_scale = VALUES(is_member_scale), \
        source_tags = VALUES(source_tags), \
        is_enabled = VALUES(is_enabled)";

const INSERT_THRESHOLD_SQL: &str = "\
    INSERT INTO metric_threshold \
        (id, tenant_id, metric_key, scope, role_slug, team_id, good, warn, is_locked) \
    VALUES (?, NULL, ?, 'product-default', '', '', ?, ?, FALSE) \
    ON DUPLICATE KEY UPDATE \
        good = VALUES(good), \
        warn = VALUES(warn)";

fn nullable_str_value(v: Option<&str>) -> Value {
    match v {
        Some(s) => Value::from(s),
        None => Value::String(None),
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let backend = manager.get_database_backend();

        for row in SEEDS {
            let catalog_id = Uuid::now_v7();
            let threshold_id = Uuid::now_v7();
            let source_tags_json_str = source_tags_json(row.source_tags);

            conn.execute(Statement::from_sql_and_values(
                backend,
                INSERT_CATALOG_SQL,
                [
                    Value::Bytes(Some(Box::new(catalog_id.as_bytes().to_vec()))),
                    Value::from(row.metric_key),
                    Value::from(row.label),
                    nullable_str_value(row.sublabel),
                    nullable_str_value(row.description),
                    nullable_str_value(row.unit),
                    nullable_str_value(row.format),
                    Value::from(row.higher_is_better),
                    Value::from(row.is_member_scale),
                    Value::from(source_tags_json_str.as_str()),
                ],
            ))
            .await?;

            conn.execute(Statement::from_sql_and_values(
                backend,
                INSERT_THRESHOLD_SQL,
                [
                    Value::Bytes(Some(Box::new(threshold_id.as_bytes().to_vec()))),
                    Value::from(row.metric_key),
                    Value::from(row.good),
                    Value::from(row.warn),
                ],
            ))
            .await?;
        }

        tracing::info!(
            seeded = SEEDS.len(),
            "zulip collab metric_catalog seed applied"
        );
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260624_000002_seed_zulip_collab_catalog is irreversible: \
             delete the collab_bullet_rows.zulip_messages_sent catalog row manually if needed."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_seed_routed_to_collab_bullet_rows() {
        assert_eq!(SEEDS.len(), 1);
        assert!(SEEDS[0].metric_key.starts_with("collab_bullet_rows."));
        assert_eq!(SEEDS[0].source_tags, &["zulip-proxy"]);
    }
}

//! Catalog seed for the collaboration Messaging modality (issue #1527,
//! epic #1516 · release #1526).
//!
//! Clone of #1514's AI-personal seed (`m20260623_000002`): for each metric it
//! writes a product-default `metric_catalog` row, an initial product-default
//! `metric_threshold` (the catalog probe requires one per row), and a
//! `metric_query_catalog` junction row linking the peer-counter query
//! (`m20260702_000001`) to the catalog metric by key.
//!
//! Thresholds here are **initial defaults, not calibrated** — per-org_unit
//! calibration is a post-release follow-up (#1527 scope note). Honest-NULL "No
//! data"/source-tooltip rendering is #1517. `source_tags` carry the vendors so
//! the FE can render source attribution (a connector is a tag, not a new row).

use sea_orm::{ConnectionTrait, Statement, Value};
use sea_orm_migration::prelude::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Must match `m20260702_000001_collab_messaging_queries::COLLAB_MESSAGING_COUNTERS_HEX`.
const COLLAB_MESSAGING_COUNTERS_HEX: &str = "00000000000000000001000000000053";

struct SeedRow {
    metric_key: &'static str,
    label: &'static str,
    sublabel: Option<&'static str>,
    description: Option<&'static str>,
    unit: Option<&'static str>,
    format: Option<&'static str>,
    higher_is_better: bool,
    /// JSON array of vendor source tags (a connector is a tag, not a new row).
    source_tags: &'static str,
    good: f64,
    warn: f64,
}

const SEEDS: &[SeedRow] = &[
    SeedRow {
        metric_key: "collab_person_counter_daily.messages_sent",
        label: "Messages sent",
        sublabel: Some("Chat messages across sources · period total"),
        description: Some(
            "Total chat messages sent across M365 Teams, Slack and Zulip. Vendor \
             semantics differ (Slack is a superset incl. replies; M365 excludes \
             group chats and replies) — a chat-engagement signal, not a comparable \
             absolute.",
        ),
        unit: Some("messages"),
        format: Some("integer"),
        higher_is_better: true,
        source_tags: r#"["m365","slack","zulip"]"#,
        good: 200.0,
        warn: 50.0,
    },
    SeedRow {
        metric_key: "collab_person_counter_daily.channel_posts",
        label: "Channel posts",
        sublabel: Some("Channel posts + replies · period total"),
        description: Some(
            "Channel posts across M365 and Slack, folding posts and replies for \
             vendor comparability (Slack cannot separate them; M365 posts + replies \
             are summed). Zulip does not surface channel posts.",
        ),
        unit: Some("messages"),
        format: Some("integer"),
        higher_is_better: true,
        source_tags: r#"["m365","slack"]"#,
        good: 20.0,
        warn: 5.0,
    },
];

const INSERT_CATALOG_SQL: &str = "\
    INSERT INTO metric_catalog \
        (id, tenant_id, metric_key, label, sublabel, description, unit, format, \
         higher_is_better, is_member_scale, source_tags, is_enabled) \
    VALUES (?, NULL, ?, ?, ?, ?, ?, ?, ?, FALSE, ?, TRUE) \
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

const INSERT_LINK_SQL: &str = "\
    INSERT IGNORE INTO metric_query_catalog \
        (id, metrics_id, metric_catalog_id) \
    SELECT UNHEX(REPLACE(UUID(),'-','')), UNHEX(?), c.id \
    FROM metric_catalog c \
    WHERE c.metric_key = ? AND c.tenant_id IS NULL";

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
                    Value::from(row.source_tags),
                ],
            ))
            .await?;

            let threshold_id = Uuid::now_v7();
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

            conn.execute(Statement::from_sql_and_values(
                backend,
                INSERT_LINK_SQL,
                [
                    Value::from(COLLAB_MESSAGING_COUNTERS_HEX),
                    Value::from(row.metric_key),
                ],
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

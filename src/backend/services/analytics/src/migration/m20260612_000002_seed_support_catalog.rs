//! Seed `metric_catalog` + product-default `metric_threshold` for the Support
//! (Zendesk) bullets (INSIGHT-459). Counterpart of
//! `m20260609_000002_seed_chatgpt_team_metrics_catalog`.
//!
//! `metric_key` namespace `support_bullet_rows.*` (matches the gold view and the
//! query↔catalog link prefix). Thresholds are sensible product-defaults
//! (good ≈ above team median); tune per-tenant via the admin CRUD (#525).
//!
//! `support_csat` is a QUALITY signal (% good), attributed to the ticket
//! *assignee* (the agreed CSAT exception to actor-attribution, PRD §4) — noted
//! in its description. `support_kb` ships enabled but has no data until a
//! Guide/Help-Center stream lands → renders `ComingSoon` (honest NULL from the
//! `query_ref` stub).

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
    higher_is_better: bool,
    is_member_scale: bool,
    good: f64,
    warn: f64,
}

const SEEDS: &[SeedRow] = &[
    SeedRow {
        metric_key: "support_bullet_rows.support_active",
        label: "Active support members",
        sublabel: Some("Zendesk \u{b7} any support activity this period"),
        description: Some(
            "Members with any support activity (comment / update / solve) in the period. DAU marker — counted once per (person, day).",
        ),
        unit: None,
        higher_is_better: true,
        is_member_scale: true,
        good: 2.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "support_bullet_rows.support_updates",
        label: "Ticket updates",
        sublabel: Some("Zendesk \u{b7} ticket field updates \u{b7} period total"),
        description: Some(
            "Ticket field changes made by the person (actor-attributed; excludes the status\u{2192}solved change, which is counted as Solved tickets).",
        ),
        unit: None,
        higher_is_better: true,
        is_member_scale: false,
        good: 10.0,
        warn: 3.0,
    },
    SeedRow {
        metric_key: "support_bullet_rows.support_public_comments",
        label: "Public comments",
        sublabel: Some("Zendesk \u{b7} public comments \u{b7} period total"),
        description: Some("Public (customer-facing) comments authored by the person."),
        unit: None,
        higher_is_better: true,
        is_member_scale: false,
        good: 5.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "support_bullet_rows.support_private_comments",
        label: "Private comments",
        sublabel: Some("Zendesk \u{b7} internal comments \u{b7} period total"),
        description: Some("Private (internal) comments authored by the person."),
        unit: None,
        higher_is_better: true,
        is_member_scale: false,
        good: 5.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "support_bullet_rows.support_solved",
        label: "Solved tickets",
        sublabel: Some("Zendesk \u{b7} tickets solved \u{b7} period total"),
        description: Some("Tickets the person moved to status=solved (actor-attributed)."),
        unit: None,
        higher_is_better: true,
        is_member_scale: false,
        good: 3.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "support_bullet_rows.support_csat",
        label: "Customer satisfaction",
        sublabel: Some("Zendesk \u{b7} % good (assignee) \u{b7} period"),
        description: Some(
            "Customer satisfaction: % of good ratings (\u{3a3}good / \u{3a3}rated). QUALITY signal, attributed to the ticket assignee at rating time (NOT the actor) — the rating is set by the customer and bound to assignee by Zendesk (PRD \u{a7}4).",
        ),
        unit: Some("%"),
        higher_is_better: true,
        is_member_scale: false,
        good: 80.0,
        warn: 60.0,
    },
    SeedRow {
        metric_key: "support_bullet_rows.support_kb",
        label: "Knowledge base articles",
        sublabel: Some("Zendesk \u{b7} articles authored \u{b7} period total"),
        description: Some(
            "Help-Center / Guide articles authored by the person. No data until the Guide stream is enabled — renders ComingSoon until then.",
        ),
        unit: None,
        higher_is_better: true,
        is_member_scale: false,
        good: 1.0,
        warn: 0.0,
    },
];

const SOURCE_TAGS_JSON: &str = "[\"zendesk\"]";

const INSERT_CATALOG_SQL: &str = "\
    INSERT INTO metric_catalog \
        (id, tenant_id, metric_key, label, sublabel, description, unit, format, \
         higher_is_better, is_member_scale, source_tags, is_enabled) \
    VALUES (?, NULL, ?, ?, ?, ?, ?, NULL, ?, ?, ?, TRUE) \
    ON DUPLICATE KEY UPDATE \
        label = VALUES(label), sublabel = VALUES(sublabel), \
        description = VALUES(description), unit = VALUES(unit), \
        higher_is_better = VALUES(higher_is_better), \
        is_member_scale = VALUES(is_member_scale), \
        source_tags = VALUES(source_tags), is_enabled = VALUES(is_enabled)";

const INSERT_THRESHOLD_SQL: &str = "\
    INSERT INTO metric_threshold \
        (id, tenant_id, metric_key, scope, role_slug, team_id, good, warn, is_locked) \
    VALUES (?, NULL, ?, 'product-default', '', '', ?, ?, FALSE) \
    ON DUPLICATE KEY UPDATE good = VALUES(good), warn = VALUES(warn)";

fn nullable_str(v: Option<&str>) -> Value {
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
            conn.execute(Statement::from_sql_and_values(
                backend,
                INSERT_CATALOG_SQL,
                [
                    Value::Bytes(Some(Box::new(Uuid::now_v7().as_bytes().to_vec()))),
                    Value::from(row.metric_key),
                    Value::from(row.label),
                    nullable_str(row.sublabel),
                    nullable_str(row.description),
                    nullable_str(row.unit),
                    Value::from(row.higher_is_better),
                    Value::from(row.is_member_scale),
                    Value::from(SOURCE_TAGS_JSON),
                ],
            ))
            .await?;
            conn.execute(Statement::from_sql_and_values(
                backend,
                INSERT_THRESHOLD_SQL,
                [
                    Value::Bytes(Some(Box::new(Uuid::now_v7().as_bytes().to_vec()))),
                    Value::from(row.metric_key),
                    Value::from(row.good),
                    Value::from(row.warn),
                ],
            ))
            .await?;
        }
        tracing::info!(seeded = SEEDS.len(), "support metric_catalog seed applied");
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260612_000002_seed_support_catalog is irreversible: delete/restore \
             the support_bullet_rows.* catalog rows manually if needed."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_count_is_seven() {
        assert_eq!(SEEDS.len(), 7);
    }

    #[test]
    fn all_keys_namespaced_support_bullet_rows() {
        for r in SEEDS {
            assert!(
                r.metric_key.starts_with("support_bullet_rows."),
                "{}",
                r.metric_key
            );
        }
    }

    #[test]
    fn only_active_is_member_scale_and_all_higher_is_better() {
        for r in SEEDS {
            assert!(r.higher_is_better, "{}: higher_is_better", r.metric_key);
            let is_active = r.metric_key.ends_with("support_active");
            assert_eq!(
                r.is_member_scale, is_active,
                "{}: member-scale only for active marker",
                r.metric_key
            );
        }
    }

    #[test]
    fn csat_is_percent_unit() {
        let csat = SEEDS
            .iter()
            .find(|r| r.metric_key.ends_with("support_csat"));
        assert!(matches!(csat, Some(r) if r.unit == Some("%")));
    }

    #[test]
    fn no_duplicate_keys() {
        use std::collections::HashSet;
        let mut seen: HashSet<&str> = HashSet::new();
        for r in SEEDS {
            assert!(seen.insert(r.metric_key), "dup {}", r.metric_key);
        }
    }
}

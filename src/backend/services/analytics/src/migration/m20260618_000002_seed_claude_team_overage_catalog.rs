//! Seed `metric_catalog` + `product-default` `metric_threshold` for the
//! Claude Team `cc_overage` `metric_key` introduced in
//! `m20260618_000001_ai_claude_team_overage_metric`.
//!
//!   `cc_overage` — per-seat spend in cents ABOVE the monthly credit limit
//!                  (max(0, used − limit)) from the Claude Team plan.
//!                  `higher_is_better = false` (overage is a cost/risk signal:
//!                  any spend over the limit is undesirable).
//!
//! Threshold placeholders (cents, "lower is better"):
//!   `good ≤ 0¢`     — the seat is within its monthly limit (no overage).
//!   `warn ≤ 5000¢`  — up to $50 over the limit. Above that → bad.
//! Tune per-tenant via the admin CRUD API (#525).
//!
//! ⚠️  FE visibility: this catalog row registers label / unit / threshold
//! metadata. The metric renders once a `bullet-layout-groups` entry for
//! `cc_overage` is added in `cyber-insight-front` (paired FE change).

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
    metric_key: "ai_bullet_rows.cc_overage",
    label: "Claude Overage",
    sublabel: Some("Claude Team \u{b7} spend over monthly limit \u{b7} cents \u{b7} period total"),
    description: Some(
        "Per-seat spend in cents ABOVE the monthly Claude Team credit limit \
         (max(0, used_credits \u{2212} monthly_credit_limit)). Sourced from \
         class_ai_overage (/overage_spend_limits). 0 means the seat is within \
         its limit; ComingSoon means no computable overage (unknown limit).",
    ),
    unit: Some("\u{a2}"),
    format: None,
    higher_is_better: false,
    is_member_scale: false,
    source_tags: &["claude_team"],
    // good ≤ 0¢ (within limit), warn ≤ 5000¢ ($50 over). Lower-is-better.
    good: 0.0,
    warn: 5000.0,
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
            "claude_team overage metric_catalog seed migration applied"
        );

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260618_000002_seed_claude_team_overage_catalog is irreversible: \
             delete the cc_overage catalog row manually if needed."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_count_is_one() {
        assert_eq!(
            SEEDS.len(),
            1,
            "expected exactly 1 new catalog row (cc_overage)"
        );
    }

    #[test]
    fn key_routes_to_ai_bullet_rows() {
        assert!(
            SEEDS[0].metric_key.starts_with("ai_bullet_rows."),
            "metric_key {:?} does not route to ai_bullet_rows",
            SEEDS[0].metric_key
        );
        assert_eq!(SEEDS[0].metric_key, "ai_bullet_rows.cc_overage");
    }

    /// Overage is a cost/risk signal — `higher_is_better` must be false.
    #[test]
    fn overage_is_lower_is_better() {
        assert!(
            !SEEDS[0].higher_is_better,
            "cc_overage is overage spend — higher_is_better must be false"
        );
    }

    /// Unit must be cents (matches `cc_cost` and the gold view's cents values).
    #[test]
    fn unit_is_cents() {
        assert_eq!(SEEDS[0].unit, Some("\u{a2}"), "cc_overage unit must be ¢");
    }

    #[test]
    fn tagged_claude_team() {
        assert_eq!(SEEDS[0].source_tags, &["claude_team"]);
    }

    /// good (within limit) must be ≤ warn (some overage tolerated) for a
    /// lower-is-better metric.
    #[test]
    fn thresholds_ordered_for_lower_is_better() {
        assert!(
            SEEDS[0].good <= SEEDS[0].warn,
            "lower-is-better: good ({}) must be ≤ warn ({})",
            SEEDS[0].good,
            SEEDS[0].warn
        );
    }

    #[test]
    fn source_tags_json_is_well_formed() {
        let json = source_tags_json(SEEDS[0].source_tags);
        assert!(json.starts_with('[') && json.ends_with(']'));
        assert!(json.len() > 2);
    }
}

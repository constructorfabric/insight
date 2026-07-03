//! Seed `metric_catalog` + `product-default` `metric_threshold` rows for
//! the three new Claude Team `metric_keys` introduced in
//! `m20260601_000001_ai_claude_team_metrics` (INSIGHT-458).
//!
//! New keys (all routed through `insight.ai_bullet_rows`):
//!
//!   `cc_cost`      — per-user-per-day cost in cents sourced from Claude Team.
//!                    `higher_is_better = false` (cost is a spending signal).
//!   `prs_with_cc`  — PRs where Claude Code was active at least once.
//!                    Requires Anthropic GitHub-app; structural 0 on orgs
//!                    without it (including the dev org).
//!   `prs_total`    — total PRs in the measurement window. Denominator for a
//!                    future `prs_with_cc_pct` ratio metric.
//!
//! ⚠️  FE visibility note: these three keys are backend-computed and stored
//! in the catalog to register their label / unit / threshold metadata.
//! They will not appear in the UI until the corresponding `BULLET_DEFS`
//! entries are added to `insight-front` (tracked as a follow-up to
//! INSIGHT-458). The catalog rows are a prerequisite, not a substitute, for
//! that work.
//!
//! Threshold placeholders: initial `good` / `warn` values are estimates for
//! an active developer. Adjust per-tenant via the admin CRUD API (#525).
//!
//!   `cc_cost`      — good ≤ 5000 ¢ ($50) / warn ≤ 10000 ¢ ($100) per period.
//!   `prs_with_cc`  — good ≥ 3 / warn ≥ 1 PRs with CC attribution per period.
//!   `prs_total`    — good ≥ 5 / warn ≥ 2 PRs per period (context denominator).
//!
//! Three FE-visible `metric_keys` from m20260527 remain at 69 rows. This
//! migration adds 3 more for a running catalog total of 72.

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

const SEEDS: &[SeedRow] = &[
    SeedRow {
        metric_key: "ai_bullet_rows.cc_cost",
        label: "Claude Code Cost",
        sublabel: Some("Claude Team \u{b7} usage cost \u{b7} cents \u{b7} period total"),
        description: Some(
            "Per-user-per-day spend in cents from the Claude Team plan. \
             Claude Team is the only per-user cost source in Silver; \
             Enterprise and Admin costs are org-level only.",
        ),
        unit: Some("\u{a2}"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["claude_team"],
        // good ≤ 5000¢ ($50/period), warn ≤ 10000¢ ($100/period).
        // For a cost metric the FE compares in "lower-is-better" mode.
        good: 5000.0,
        warn: 10000.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.prs_with_cc",
        label: "PRs with Claude Code",
        sublabel: Some("Claude Team GitHub-app \u{b7} PRs with CC attribution \u{b7} period total"),
        description: Some(
            "Number of pull requests where Claude Code was active at least once \
             in the measurement window. Populated only on tenants with the \
             Anthropic GitHub-app connected; 0 on orgs without it.",
        ),
        unit: Some("PRs"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["claude_team"],
        good: 3.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.prs_total",
        label: "Total PRs (CC window)",
        sublabel: Some("Claude Team GitHub-app \u{b7} total PRs in window \u{b7} period total"),
        description: Some(
            "Total pull requests opened in the measurement window — denominator \
             for the prs_with_cc_pct ratio metric. Same availability caveat as \
             prs_with_cc: requires the Anthropic GitHub-app connection.",
        ),
        unit: Some("PRs"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["claude_team"],
        good: 5.0,
        warn: 2.0,
    },
];

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
            "claude_team metric_catalog seed migration applied"
        );

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260601_000002_seed_claude_team_metrics_catalog is irreversible: \
             delete the three catalog rows manually if needed."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the number of new catalog rows shipped by this migration.
    /// Update if Claude Team gains new `metric_keys` in a future migration.
    #[test]
    fn seed_count_is_three() {
        assert_eq!(
            SEEDS.len(),
            3,
            "expected exactly 3 new Claude Team catalog rows (cc_cost, prs_with_cc, prs_total)"
        );
    }

    /// All three `metric_keys` must route to `ai_bullet_rows` (not a typo
    /// like `ai_bullet_row` or a different table segment).
    #[test]
    fn all_keys_route_to_ai_bullet_rows() {
        for row in SEEDS {
            assert!(
                row.metric_key.starts_with("ai_bullet_rows."),
                "metric_key {:?} does not route to ai_bullet_rows",
                row.metric_key
            );
        }
    }

    /// `cc_cost` is a spending signal — higher values mean more cost.
    /// `higher_is_better` must be `false`.
    #[test]
    fn cc_cost_is_lower_is_better() {
        let row = SEEDS
            .iter()
            .find(|r| r.metric_key == "ai_bullet_rows.cc_cost")
            .unwrap_or_else(|| panic!("cc_cost row must be in SEEDS"));
        assert!(
            !row.higher_is_better,
            "cc_cost is a cost metric — higher_is_better must be false"
        );
    }

    /// `prs_with_cc` and `prs_total` are activity counters — more is better.
    #[test]
    fn prs_keys_are_higher_is_better() {
        for key in ["ai_bullet_rows.prs_with_cc", "ai_bullet_rows.prs_total"] {
            let row = SEEDS
                .iter()
                .find(|r| r.metric_key == key)
                .unwrap_or_else(|| panic!("{key} row must be in SEEDS"));
            assert!(
                row.higher_is_better,
                "{key}: prs counters are activity signals — higher_is_better must be true"
            );
        }
    }

    /// All three rows carry `source_tags = ["claude_team"]` — they are
    /// exclusively sourced from the Claude Team connector.
    #[test]
    fn all_keys_tagged_claude_team() {
        for row in SEEDS {
            assert_eq!(
                row.source_tags,
                &["claude_team"],
                "{}: expected source_tags = [\"claude_team\"]",
                row.metric_key
            );
        }
    }

    /// No duplicate `metric_keys` within this migration's SEEDS slice.
    #[test]
    fn no_duplicate_metric_keys() {
        use std::collections::HashSet;
        let mut seen: HashSet<&str> = HashSet::new();
        for row in SEEDS {
            assert!(
                seen.insert(row.metric_key),
                "duplicate seed metric_key {:?}",
                row.metric_key
            );
        }
    }

    /// `source_tags` JSON must be a well-formed JSON array string.
    #[test]
    fn source_tags_json_is_well_formed() {
        for row in SEEDS {
            let json = source_tags_json(row.source_tags);
            assert!(
                json.starts_with('[') && json.ends_with(']'),
                "{}: source_tags_json must be a JSON array, got {:?}",
                row.metric_key,
                json
            );
            // Must not be an empty array — every row needs at least one tag.
            assert!(
                json.len() > 2,
                "{}: source_tags must have at least one entry",
                row.metric_key
            );
        }
    }
}

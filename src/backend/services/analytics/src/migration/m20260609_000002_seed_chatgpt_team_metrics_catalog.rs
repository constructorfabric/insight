//! Seed `metric_catalog` + `product-default` `metric_threshold` rows for the
//! `ChatGPT` Team `metric_keys` surfaced in `m20260609_000001_ai_chatgpt_team_metrics`
//! (INSIGHT-459). Counterpart of `m20260601_000002_seed_claude_team_metrics_catalog`.
//!
//! Two existing rows are refreshed (they shipped as `ComingSoon` placeholders
//! in m20260527 with stale metadata):
//!   `ai_bullet_rows.codex_active` — sublabel fixed `OpenAI API` →
//!                    `ChatGPT Team · Codex`; `source_tags` → `[chatgpt_team]`.
//!   `ai_bullet_rows.chatgpt`      — `source_tags` `[chatgpt]` → `[chatgpt_team]`.
//!
//! Three new rows (newly emitted by the Gold view + `query_ref)`:
//!   `ai_bullet_rows.codex_lines`    — Codex AI-accepted lines (`lines_added`).
//!   `ai_bullet_rows.codex_sessions` — Codex sessions (threads / `n_threads`).
//!   `ai_bullet_rows.chatgpt_active` — `ChatGPT` chat DAU marker.
//!
//! Thresholds mirror the cyber-insight-front `BULLET_DEFS` for the same keys
//! (good/warn). Adjust per-tenant via the admin CRUD API (#525).
//!
//! Idempotent via ON DUPLICATE KEY UPDATE — safe to re-run.

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
    // ── refreshed (existing ComingSoon rows) ──────────────────────────
    SeedRow {
        metric_key: "ai_bullet_rows.codex_active",
        label: "Codex — active members",
        sublabel: Some("ChatGPT Team \u{b7} Codex \u{b7} any activity this period"),
        description: Some(
            "Members with any Codex (ChatGPT Team) activity in the period. \
             DAU marker — counted once per (person, day).",
        ),
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: true,
        source_tags: &["chatgpt_team"],
        good: 2.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.chatgpt",
        label: "ChatGPT Activity",
        sublabel: Some("ChatGPT Team \u{b7} interactions \u{b7} period total"),
        description: Some(
            "ChatGPT chat interactions (messages) per user, sourced from \
             class_ai_assistant_usage (surface='chat').",
        ),
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["chatgpt_team"],
        good: 10.0,
        warn: 0.0,
    },
    // ── new keys ──────────────────────────────────────────────────────
    SeedRow {
        metric_key: "ai_bullet_rows.chatgpt_active",
        label: "ChatGPT — active members",
        sublabel: Some("ChatGPT Team \u{b7} chat \u{b7} any activity this period"),
        description: Some(
            "Members with any ChatGPT chat activity in the period. DAU marker \
             — counted once per (person, day).",
        ),
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: true,
        source_tags: &["chatgpt_team"],
        good: 2.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.codex_lines",
        label: "Codex Accepted Lines",
        sublabel: Some("ChatGPT Team \u{b7} Codex \u{b7} accepted lines \u{b7} period total"),
        description: Some(
            "AI-accepted lines from Codex (code_attribution.lines_of_code.added). \
             Same semantics as cc_lines / cursor_lines.",
        ),
        unit: Some("lines"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["chatgpt_team"],
        good: 50.0,
        warn: 10.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.codex_sessions",
        label: "Codex Sessions",
        sublabel: Some("ChatGPT Team \u{b7} Codex \u{b7} threads \u{b7} period total"),
        description: Some(
            "Codex sessions (threads / n_threads) per user. Analogous to \
             cc_sessions for Claude Code.",
        ),
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["chatgpt_team"],
        good: 4.0,
        warn: 1.0,
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
            "chatgpt_team metric_catalog seed migration applied"
        );

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260609_000002_seed_chatgpt_team_metrics_catalog is irreversible: \
             delete/restore the catalog rows manually if needed."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the seed count: 2 refreshed (`codex_active`, chatgpt) + 3 new.
    #[test]
    fn seed_count_is_five() {
        assert_eq!(SEEDS.len(), 5, "expected 5 ChatGPT Team catalog rows");
    }

    /// All `metric_keys` route to `ai_bullet_rows.` (catalog namespace).
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

    /// Every row is tagged exclusively to the `chatgpt_team` connector.
    #[test]
    fn all_keys_tagged_chatgpt_team() {
        for row in SEEDS {
            assert_eq!(
                row.source_tags,
                &["chatgpt_team"],
                "{}: expected source_tags = [\"chatgpt_team\"]",
                row.metric_key
            );
        }
    }

    /// The two *_active markers are member-scale; the rest are not.
    #[test]
    fn active_markers_are_member_scale() {
        for row in SEEDS {
            let is_active = row.metric_key.ends_with("_active");
            assert_eq!(
                row.is_member_scale, is_active,
                "{}: is_member_scale must match *_active marker status",
                row.metric_key
            );
        }
    }

    /// All `ChatGPT` Team metrics are activity/adoption signals — higher is better.
    #[test]
    fn all_higher_is_better() {
        for row in SEEDS {
            assert!(
                row.higher_is_better,
                "{}: higher_is_better must be true",
                row.metric_key
            );
        }
    }

    /// No duplicate `metric_keys`.
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

    /// `source_tags` JSON must be a well-formed, non-empty JSON array.
    #[test]
    fn source_tags_json_is_well_formed() {
        for row in SEEDS {
            let json = source_tags_json(row.source_tags);
            assert!(json.starts_with('[') && json.ends_with(']') && json.len() > 2);
        }
    }
}

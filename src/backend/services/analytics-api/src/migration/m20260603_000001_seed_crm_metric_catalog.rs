//! Seed `metric_catalog` + `product-default` `metric_threshold` rows for
//! the CRM (sales-rep) bullet metrics — the next batch of FE constants to
//! move under the "one source of truth = catalog" rule (follow-on to #82).
//!
//! Source: `cyberfabric/cyber-insight-front` `src/api/transforms.ts`
//! `CRM_QUALITY_BULLETS` + `CRM_ACTIVITY_BULLETS` (the two compile-in
//! arrays still feeding the sales-rep dashboard at the time this
//! migration ships). The FE side hydrates from the wire once these rows
//! land on production — same flow as #66 waves 1-3 + #82 for the
//! engineering metrics.
//!
//! Eight rows, all routed through `crm_bullet_rows` (the CH gold view
//! the CRM bullet queries SELECT FROM — see
//! `m20260507_000001_seed_crm_metrics::CRM_BULLET_QUALITY_QUERY` /
//! `CRM_BULLET_ACTIVITY_QUERY`):
//!
//!   Quality bullets (`CRM_QUALITY_BULLETS`):
//!     `win_rate`       — won / closed, period total, higher is better
//!     `avg_deal_size`  — mean of `properties_amount` on won deals
//!     `cycle_days`     — created → won mean days, lower is better
//!     `deals_opened`   — volume of deals created in period
//!
//!   Activity bullets (`CRM_ACTIVITY_BULLETS`):
//!     `calls`          — HubSpot `engagements_calls`
//!     `emails`         — HubSpot `engagements_emails`
//!     `meetings`       — HubSpot `engagements_meetings`
//!     `comms_per_won`  — (calls + emails + meetings + tasks) / `deals_won`
//!                      ·  efficiency · lower is better
//!
//! ### Threshold policy
//!
//! The FE CRM bullet defs carry no `good` / `warn` values — the renderer
//! in `transformCrmBullets` (`transforms.ts`) computes status by
//! comparing the rep's value against the team median ± 10% tolerance,
//! not against a numeric policy threshold. We seed `good = 0.0` /
//! `warn = 0.0` for every row to satisfy the `metric_threshold` NOT NULL
//! constraint; admin CRUD (#525) is the path for landing real per-tenant
//! policy values when they exist. The catalog consumer for CRM bullets
//! does not read these values today (see `transforms.ts`).
//!
//! ### Schema-validator note
//!
//! `crm_bullet_rows` exists in CH (gold view shipped under
//! `migrations/20260512000000_crm-gold-views.sql`) but the `metric_key`s
//! sit in the row-form `metric_value` column rather than as named
//! columns — same shape as `task_delivery_bullet_rows` et al. The
//! schema-validator (Refs #521) will report `column_not_found` for
//! every row in this seed, which is the truthful informational signal;
//! it never blocks reads or writes.
//!
//! ### Follow-on
//!
//! - The companion link migration (`m20260603_000002_link_crm_query_catalog`)
//!   wires the CRM Bullet Quality (`…0022`) and CRM Bullet Activity
//!   (`…0023`) `metrics` rows into `metric_query_catalog`.
//! - FE refactor (`transforms.ts::transformCrmBullets`) consumes the
//!   catalog and deletes `CRM_QUALITY_BULLETS` / `CRM_ACTIVITY_BULLETS`
//!   after a parity capture against staging.

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
    // ─────────────────── CRM_QUALITY_BULLETS ───────────────────
    SeedRow {
        metric_key: "crm_bullet_rows.win_rate",
        label: "Win Rate",
        sublabel: Some("Won \u{f7} closed in period"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["hubspot"],
        good: 0.0,
        warn: 0.0,
    },
    SeedRow {
        metric_key: "crm_bullet_rows.avg_deal_size",
        label: "Avg Deal Size",
        sublabel: Some("Won deals \u{b7} mean of properties_amount"),
        description: None,
        unit: Some("$"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["hubspot"],
        good: 0.0,
        warn: 0.0,
    },
    SeedRow {
        metric_key: "crm_bullet_rows.cycle_days",
        label: "Avg Cycle Time",
        sublabel: Some("Created \u{2192} won \u{b7} mean days \u{b7} lower = better"),
        description: None,
        unit: Some("d"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["hubspot"],
        good: 0.0,
        warn: 0.0,
    },
    SeedRow {
        metric_key: "crm_bullet_rows.deals_opened",
        label: "Deals Opened",
        sublabel: Some("Volume \u{b7} deals created in period"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["hubspot"],
        good: 0.0,
        warn: 0.0,
    },
    // ─────────────────── CRM_ACTIVITY_BULLETS ───────────────────
    SeedRow {
        metric_key: "crm_bullet_rows.calls",
        label: "Calls",
        sublabel: Some("HubSpot \u{b7} engagements_calls in period"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["hubspot"],
        good: 0.0,
        warn: 0.0,
    },
    SeedRow {
        metric_key: "crm_bullet_rows.emails",
        label: "Emails",
        sublabel: Some("HubSpot \u{b7} engagements_emails in period"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["hubspot"],
        good: 0.0,
        warn: 0.0,
    },
    SeedRow {
        metric_key: "crm_bullet_rows.meetings",
        label: "Meetings",
        sublabel: Some("HubSpot \u{b7} engagements_meetings in period"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["hubspot"],
        good: 0.0,
        warn: 0.0,
    },
    SeedRow {
        metric_key: "crm_bullet_rows.comms_per_won",
        label: "Comms / Won Deal",
        sublabel: Some("Total comms \u{f7} deals won \u{b7} efficiency \u{b7} lower = better"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["hubspot"],
        good: 0.0,
        warn: 0.0,
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
            "crm metric_catalog seed migration applied"
        );

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260603_000001_seed_crm_metric_catalog is irreversible: \
             delete the eight catalog rows manually if needed."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Pins the row count. Bumping this requires updating the FE source
    /// arrays (`CRM_QUALITY_BULLETS` / `CRM_ACTIVITY_BULLETS`) and the
    /// row comments — silent drift would mean a stale catalog vs the
    /// rendered FE.
    #[test]
    fn seed_count_is_eight() {
        assert_eq!(
            SEEDS.len(),
            8,
            "expected 8 CRM catalog rows (4 quality + 4 activity bullets)"
        );
    }

    /// Every CRM `metric_key` routes through `crm_bullet_rows` — the
    /// gold view the CRM bullet queries SELECT FROM. Catches typos like
    /// `crm_bullet_row.foo` slipping into the seed.
    #[test]
    fn all_keys_route_to_crm_bullet_rows() {
        for row in SEEDS {
            assert!(
                row.metric_key.starts_with("crm_bullet_rows."),
                "metric_key {:?} does not route to crm_bullet_rows",
                row.metric_key
            );
        }
    }

    /// `cycle_days` and `comms_per_won` are inverse metrics — shorter
    /// cycle / fewer comms per won deal are signals of efficiency.
    /// `higher_is_better` must be `false` on both.
    #[test]
    fn lower_is_better_keys_are_marked() {
        let lower_better = [
            "crm_bullet_rows.cycle_days",
            "crm_bullet_rows.comms_per_won",
        ];
        for key in lower_better {
            let row = SEEDS
                .iter()
                .find(|r| r.metric_key == key)
                .unwrap_or_else(|| panic!("{key} row must be in SEEDS"));
            assert!(
                !row.higher_is_better,
                "{key}: lower-is-better metric must set higher_is_better = false"
            );
        }
    }

    /// Every other CRM bullet is a volume / rate counter where more is
    /// better.
    #[test]
    fn higher_is_better_keys_are_marked() {
        let higher_better = [
            "crm_bullet_rows.win_rate",
            "crm_bullet_rows.avg_deal_size",
            "crm_bullet_rows.deals_opened",
            "crm_bullet_rows.calls",
            "crm_bullet_rows.emails",
            "crm_bullet_rows.meetings",
        ];
        for key in higher_better {
            let row = SEEDS
                .iter()
                .find(|r| r.metric_key == key)
                .unwrap_or_else(|| panic!("{key} row must be in SEEDS"));
            assert!(
                row.higher_is_better,
                "{key}: volume / rate counter must set higher_is_better = true"
            );
        }
    }

    /// All CRM rows source from HubSpot — the only CRM connector today.
    #[test]
    fn all_keys_tagged_hubspot() {
        for row in SEEDS {
            assert_eq!(
                row.source_tags,
                &["hubspot"],
                "{}: expected source_tags = [\"hubspot\"]",
                row.metric_key
            );
        }
    }

    /// FE CRM defs carry no `good` / `warn` — the renderer uses team
    /// median ±10% comparison. Pin the zeroed thresholds so a future
    /// reviewer doesn't mistake them for product policy. Equality
    /// against `0.0` is intentional: the values are compile-time
    /// literals from `SEEDS`, not computed, so the `float_cmp` lint
    /// doesn't apply.
    #[test]
    #[allow(clippy::float_cmp)]
    fn thresholds_are_zeroed_for_v1() {
        for row in SEEDS {
            let key = row.metric_key;
            assert_eq!(
                row.good, 0.0,
                "{key}: CRM v1 seed must zero `good` (no FE policy)"
            );
            assert_eq!(
                row.warn, 0.0,
                "{key}: CRM v1 seed must zero `warn` (no FE policy)"
            );
        }
    }

    #[test]
    fn no_duplicate_metric_keys() {
        let mut seen: HashSet<&str> = HashSet::new();
        for row in SEEDS {
            let key = row.metric_key;
            assert!(seen.insert(key), "duplicate seed metric_key {key:?}");
        }
    }

    #[test]
    fn source_tags_json_is_well_formed() {
        for row in SEEDS {
            let key = row.metric_key;
            let json = source_tags_json(row.source_tags);
            assert!(
                json.starts_with('[') && json.ends_with(']'),
                "{key}: source_tags_json must be a JSON array, got {json:?}"
            );
            assert!(
                json.len() > 2,
                "{key}: source_tags must have at least one entry"
            );
        }
    }
}

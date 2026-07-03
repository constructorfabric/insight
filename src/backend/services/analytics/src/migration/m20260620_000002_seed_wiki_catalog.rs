//! Seed `metric_catalog` + product-default `metric_threshold` rows for the
//! wiki bullet `metric_keys` introduced in `m20260620_000001_seed_wiki_metrics`
//! (paired gold view `insight.wiki_bullet_rows`).
//!
//! 4 `metric_keys`, all routed through `wiki_bullet_rows`, `source_tags`
//! `["confluence", "outline"]` (both wiki sources feed the same keys):
//!   `wiki_pages_created` / `wiki_edits` / `wiki_comments` — counters (higher better);
//!   `wiki_active_authors` — 0/1 member-scale marker.
//!
//! Mirrors `m20260601_000002_seed_claude_team_metrics_catalog` (additive
//! catalog seed). Thresholds are product-default placeholders; tune per
//! tenant via the admin CRUD API.

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
        metric_key: "wiki_bullet_rows.wiki_pages_created",
        label: "Wiki Pages Created",
        sublabel: Some("Confluence/Outline \u{b7} pages authored \u{b7} period total"),
        description: Some("Wiki pages created by the person in the period (Confluence/Outline)."),
        unit: Some("pages"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["confluence", "outline"],
        good: 5.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "wiki_bullet_rows.wiki_edits",
        label: "Wiki Edits",
        sublabel: Some("Confluence/Outline \u{b7} page revisions \u{b7} period total"),
        description: Some("Page revisions (version_count \u{2212} 1) attributed to the person."),
        unit: Some("edits"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["confluence", "outline"],
        good: 30.0,
        warn: 10.0,
    },
    SeedRow {
        metric_key: "wiki_bullet_rows.wiki_comments",
        label: "Wiki Comments",
        sublabel: Some(
            "Confluence/Outline \u{b7} comments on the person's pages \u{b7} period total",
        ),
        description: Some("Comments (footer + inline + replies) received on the person's pages."),
        unit: Some("comments"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["confluence", "outline"],
        good: 10.0,
        warn: 2.0,
    },
    SeedRow {
        metric_key: "wiki_bullet_rows.wiki_active_authors",
        label: "Active Wiki Authors",
        sublabel: Some("Confluence/Outline \u{b7} members who authored/edited this period"),
        description: Some("Count of members active in the wiki this period (member-scale marker)."),
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: true,
        source_tags: &["confluence", "outline"],
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

        tracing::info!(seeded = SEEDS.len(), "wiki metric_catalog seed applied");
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260620_000002_seed_wiki_catalog is irreversible: \
             delete the 4 wiki_bullet_rows catalog rows manually if needed."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_count_is_four() {
        assert_eq!(SEEDS.len(), 4);
    }

    #[test]
    fn all_keys_route_to_wiki_bullet_rows() {
        for r in SEEDS {
            assert!(
                r.metric_key.starts_with("wiki_bullet_rows."),
                "metric_key {:?} must route to wiki_bullet_rows",
                r.metric_key
            );
        }
    }

    #[test]
    fn active_authors_is_member_scale_counters_are_not() {
        for r in SEEDS {
            if r.metric_key.ends_with("wiki_active_authors") {
                assert!(
                    r.is_member_scale,
                    "wiki_active_authors must be member-scale"
                );
            } else {
                assert!(
                    !r.is_member_scale,
                    "{} must not be member-scale",
                    r.metric_key
                );
            }
            assert!(
                r.higher_is_better,
                "{} should be higher-is-better",
                r.metric_key
            );
        }
    }

    #[test]
    fn tagged_confluence_and_outline() {
        for r in SEEDS {
            assert_eq!(r.source_tags, &["confluence", "outline"]);
        }
    }

    #[test]
    fn source_tags_json_well_formed() {
        let j = source_tags_json(SEEDS[0].source_tags);
        assert!(j.starts_with('[') && j.ends_with(']') && j.len() > 2);
    }
}

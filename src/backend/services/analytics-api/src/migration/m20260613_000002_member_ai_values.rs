//! Per-person AI metric values for a roster (`…0043`), completing the
//! `…0040-42` member-values set (Task Delivery / Collaboration / Git) so the
//! team view can compare each member's AI metrics against their own department
//! (the AI department distribution lives in `…0048`).
//!
//! Long rows `(person_id, metric_key, value)`, no cohort — the team view colors
//! client-side against the department distribution. The per-person wide
//! aggregate + ARRAY JOIN cover the same distributable AI keys as the AI
//! department distribution (`m20260613_000001`): the active-counter flags and
//! the all-NULL placeholders are excluded (member-scale / no source data, so
//! they have no per-person distribution to compare against).
//!
//! `person_id IN (roster)` is applied by the handler at the outer level; the
//! per-person leaf keeps `GROUP BY person_id` for the date-walker. `down()`
//! deletes the metric (append-only seed).

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const ZERO_TENANT: &str = "00000000000000000000000000000000";
const MEMBER_VALUES_AI_HEX: &str = "00000000000000000001000000000043";

/// Per-person AI wide aggregate, copied verbatim from
/// `m20260613_000001_dept_ai_distribution::ai_wide_aggregate_pp`.
fn ai_wide_aggregate_pp() -> &'static str {
    "SELECT person_id, any(org_unit_id) AS org_unit_id, \
         sumIf(metric_value, metric_key = 'cursor_completions') AS cursor_completions_v, \
         sumIf(metric_value, metric_key = 'cursor_agents') AS cursor_agents_v, \
         sumIf(metric_value, metric_key = 'cursor_lines') AS cursor_lines_v, \
         sumIf(metric_value, metric_key = 'cc_sessions') AS cc_sessions_v, \
         sumIf(metric_value, metric_key = 'cc_lines') AS cc_lines_v, \
         sumIf(metric_value, metric_key = 'cc_tool_accept') AS cc_tool_accept_v, \
         sumIf(metric_value, metric_key = 'team_ai_loc') AS team_ai_loc_v, \
         if(sumIf(metric_value, metric_key = 'cursor_offered') > 0, \
            round(toFloat64(100) \
                  * sumIf(metric_value, metric_key = 'cursor_completions') \
                  / sumIf(metric_value, metric_key = 'cursor_offered'), 1), \
            CAST(NULL AS Nullable(Float64))) AS cursor_acceptance_v, \
         if(sumIf(metric_value, metric_key = 'cc_offered') > 0, \
            round(toFloat64(100) \
                  * sumIf(metric_value, metric_key = 'cc_tool_accept') \
                  / sumIf(metric_value, metric_key = 'cc_offered'), 1), \
            CAST(NULL AS Nullable(Float64))) AS cc_tool_acceptance_v, \
         if(sumIf(metric_value, metric_key = 'cursor_total_lines') > 0, \
            round(toFloat64(100) \
                  * sumIf(metric_value, metric_key = 'cursor_lines') \
                  / sumIf(metric_value, metric_key = 'cursor_total_lines'), 1), \
            CAST(NULL AS Nullable(Float64))) AS ai_loc_share2_v \
     FROM insight.ai_bullet_rows \
     GROUP BY person_id"
}

/// ARRAY JOIN unpivot for the distributable AI keys, copied verbatim from
/// `m20260613_000001_dept_ai_distribution::ai_array_join_kv`.
fn ai_array_join_kv() -> &'static str {
    "ARRAY JOIN [ \
         ('cursor_completions', cursor_completions_v), \
         ('cursor_agents',      cursor_agents_v), \
         ('cursor_lines',       cursor_lines_v), \
         ('cc_sessions',        cc_sessions_v), \
         ('cc_lines',           cc_lines_v), \
         ('cc_tool_accept',     cc_tool_accept_v), \
         ('team_ai_loc',        team_ai_loc_v), \
         ('cursor_acceptance',  cursor_acceptance_v), \
         ('cc_tool_acceptance', cc_tool_acceptance_v), \
         ('ai_loc_share2',      ai_loc_share2_v) \
     ] AS kv"
}

fn ai_member_values_query() -> String {
    format!(
        "SELECT person_id, kv.1 AS metric_key, kv.2 AS value \
         FROM ({pp}) pp \
         {kv}",
        pp = ai_wide_aggregate_pp(),
        kv = ai_array_join_kv(),
    )
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "INSERT INTO metrics (id, insight_tenant_id, name, description, query_ref, is_enabled) \
             VALUES (UNHEX('{MEMBER_VALUES_AI_HEX}'), UNHEX('{ZERO_TENANT}'), 'Team Member Values — AI', \
             'Per-person AI metric values for a roster (person_id IN). Long rows (person_id, metric_key, value); no cohort. Distributable AI keys only (active-counter flags and NULL placeholders excluded).', \
             '{qr}', 1) \
             ON DUPLICATE KEY UPDATE name=VALUES(name), description=VALUES(description), query_ref=VALUES(query_ref), is_enabled=1",
            qr = ai_member_values_query().replace('\'', "''"),
        ))
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(&format!(
            "DELETE FROM metrics WHERE id = UNHEX('{MEMBER_VALUES_AI_HEX}')"
        ))
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_rows_per_person_no_cohort() {
        let qr = ai_member_values_query();
        assert!(
            qr.starts_with("SELECT person_id, kv.1 AS metric_key, kv.2 AS value FROM ("),
            "must emit per-person long rows"
        );
        assert!(qr.contains("ARRAY JOIN ["), "must unpivot to long rows");
        assert!(qr.contains("GROUP BY person_id"), "per-person rollup");
        assert!(qr.contains("insight.ai_bullet_rows"), "reads the AI source");
        // Value-only: no cohort join / distribution columns.
        for forbidden in ["LEFT JOIN", "team_median", "_p25", "_p75", " AS median"] {
            assert!(!qr.contains(forbidden), "must NOT contain {forbidden:?}");
        }
    }

    #[test]
    fn excludes_active_counters_and_placeholders() {
        let qr = ai_member_values_query();
        for key in ["cursor_acceptance", "cc_tool_acceptance", "ai_loc_share2"] {
            assert!(
                qr.contains(&format!("'{key}'")),
                "missing distributable key {key}"
            );
        }
        for key in [
            "active_ai_members",
            "cursor_active",
            "cc_active",
            "chatgpt",
            "claude_web",
        ] {
            assert!(
                !qr.contains(&format!("('{key}'")),
                "non-distributable key {key} must be excluded"
            );
        }
    }
}

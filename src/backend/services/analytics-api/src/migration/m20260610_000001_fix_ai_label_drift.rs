//! Fix AI bullet label/source drift in `metric_catalog` (issue #1286, secondary
//! findings). `threshold-config.ts` was deleted on the FE (#66): metric labels
//! now come from the wire catalog, so these corrections live here.
//!
//! Surgical `UPDATE … SET sublabel` on the product-default rows (tenant_id IS
//! NULL) — we touch ONLY the sublabel, never the label/description/thresholds.
//! Idempotent (re-running sets the same text).
//!
//!   • cc_active / cc_lines / cc_sessions / cc_tool_acceptance — sourced from
//!     the **Claude Team** connector (`claude_team_code_metrics`), not the
//!     "Anthropic Enterprise API". (codex_* was already corrected to
//!     "ChatGPT Team · Codex" in m20260609_000002.)
//!   • team_ai_loc — the daily LOC sum correctly includes Codex
//!     (cc + codex + cursor), so the sublabel must say so.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// (metric_key, corrected sublabel). \u{b7} = "·", \u{f7} = "÷".
const SUBLABEL_FIXES: &[(&str, &str)] = &[
    ("ai_bullet_rows.cc_active",         "Claude Team \u{b7} any activity this period"),
    ("ai_bullet_rows.cc_lines",          "Claude Team \u{b7} accepted lines \u{b7} period total"),
    ("ai_bullet_rows.cc_sessions",       "Claude Team \u{b7} sessions \u{b7} period total"),
    ("ai_bullet_rows.cc_tool_acceptance","Claude Team \u{b7} accepted \u{f7} offered \u{b7} daily avg"),
    ("ai_bullet_rows.team_ai_loc",       "Cursor + Claude Code + Codex \u{b7} accepted lines \u{b7} period total"),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (metric_key, sublabel) in SUBLABEL_FIXES {
            db.execute_unprepared(&format!(
                "UPDATE metric_catalog SET sublabel = '{sub}' \
                 WHERE tenant_id IS NULL AND metric_key = '{key}'",
                sub = sublabel.replace('\'', "''"),
                key = metric_key.replace('\'', "''"),
            ))
            .await?;
        }
        tracing::info!(fixed = SUBLABEL_FIXES.len(), "ai bullet sublabel drift corrected");
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260610_000001_fix_ai_label_drift is irreversible: \
             restore the prior sublabels from m20260527_000001 manually if needed."
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No leftover "Anthropic Enterprise API" / "OpenAI API" attribution in the
    /// corrected sublabels, and team_ai_loc names Codex.
    #[test]
    fn sublabels_have_correct_source_attribution() {
        for (key, sub) in SUBLABEL_FIXES {
            assert!(!sub.contains("Anthropic Enterprise API"), "{key}: stale Anthropic label");
            assert!(!sub.contains("OpenAI API"), "{key}: stale OpenAI label");
        }
        let team = SUBLABEL_FIXES.iter().find(|(k, _)| *k == "ai_bullet_rows.team_ai_loc").unwrap().1;
        assert!(team.contains("Codex"), "team_ai_loc sublabel must include Codex");
    }

    // =====================================================================
    // Guard against the issue #1286 defect CLASS: a new bullet metric_key
    // silently defaulting to avg() in the ai_person_period rollup.
    //
    // These three sets MIRROR the multiIf in the CH migration
    // 20260610000000_ai-person-period-rollup-fix.sql (counters→sum,
    // active→max, ratios→avg). Keep them in sync with that file. The test
    // asserts every metric_key the gold view emits is classified into exactly
    // one bucket — so adding a connector key without classifying it fails CI.
    // =====================================================================
    const SUM_KEYS: &[&str] = &[
        "chatgpt", "cc_lines", "cc_sessions", "cursor_agents", "cursor_lines",
        "claude_web", "cursor_completions", "team_ai_loc", "codex_lines",
        "codex_sessions", "cc_offered", "cc_tool_accept", "cc_cost",
        "prs_total", "prs_with_cc",
    ];
    const MAX_KEYS: &[&str] = &[
        "active_ai_members", "cursor_active", "cc_active", "codex_active",
        "chatgpt_active",
    ];
    /// Ratio / share metrics that legitimately use avg().
    const AVG_KEYS: &[&str] = &[
        "cursor_acceptance", "cc_tool_acceptance", "ai_loc_share2",
        "cursor_offered", "cursor_total_lines",
    ];

    /// Every metric_key emitted into ai_bullet_rows (the FE-visible set from
    /// m20260609_000001's ARRAY JOIN) must be explicitly classified.
    const ALL_BULLET_KEYS: &[&str] = &[
        "active_ai_members", "cursor_active", "cc_active", "cursor_completions",
        "cursor_agents", "cursor_lines", "cc_sessions", "cc_lines", "cc_tool_accept",
        "team_ai_loc", "cc_cost", "prs_with_cc", "prs_total", "cursor_acceptance",
        "cc_tool_acceptance", "ai_loc_share2", "codex_active", "chatgpt", "claude_web",
        "codex_lines", "codex_sessions", "chatgpt_active",
    ];

    #[test]
    fn every_bullet_key_is_classified_not_defaulting_to_avg() {
        for key in ALL_BULLET_KEYS {
            let in_sum = SUM_KEYS.contains(key);
            let in_max = MAX_KEYS.contains(key);
            let in_avg = AVG_KEYS.contains(key);
            let n = [in_sum, in_max, in_avg].iter().filter(|b| **b).count();
            assert_eq!(
                n, 1,
                "metric_key '{key}' must be classified in EXACTLY one of \
                 sum/max/avg for ai_person_period (found in {n}). A new key \
                 left unclassified silently defaults to avg() — issue #1286."
            );
        }
    }

    /// Counters must never be in the active(max) bucket and vice-versa.
    #[test]
    fn codex_counters_sum_chatgpt_active_max() {
        assert!(SUM_KEYS.contains(&"codex_lines") && SUM_KEYS.contains(&"codex_sessions"));
        assert!(MAX_KEYS.contains(&"chatgpt_active"));
        assert!(!SUM_KEYS.contains(&"chatgpt_active"));
        assert!(!MAX_KEYS.contains(&"codex_lines"));
    }
}

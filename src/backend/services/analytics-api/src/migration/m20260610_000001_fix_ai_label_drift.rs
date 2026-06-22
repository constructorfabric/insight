//! Fix AI bullet label/source drift in `metric_catalog` (issue #1286, secondary
//! findings). `threshold-config.ts` was deleted on the FE (#66): metric labels
//! now come from the wire catalog, so these corrections live here.
//!
//! Surgical `UPDATE … SET sublabel` on the product-default rows (`tenant_id` IS
//! NULL) — we touch ONLY the sublabel, never the label/description/thresholds.
//! Idempotent (re-running sets the same text).
//!
//!   • `cc_active` / `cc_lines` / `cc_sessions` / `cc_tool_acceptance` — sourced from
//!     the **Claude Team** connector (`claude_team_code_metrics`), not the
//!     "Anthropic Enterprise API". (codex_* was already corrected to
//!     "`ChatGPT` Team · Codex" in `m20260609_000002`.)
//!   • `team_ai_loc` — the daily LOC sum correctly includes Codex
//!     (cc + codex + cursor), so the sublabel must say so.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// (`metric_key`, corrected sublabel). \u{b7} = "·", \u{f7} = "÷".
const SUBLABEL_FIXES: &[(&str, &str)] = &[
    (
        "ai_bullet_rows.cc_active",
        "Claude Team \u{b7} any activity this period",
    ),
    (
        "ai_bullet_rows.cc_lines",
        "Claude Team \u{b7} accepted lines \u{b7} period total",
    ),
    (
        "ai_bullet_rows.cc_sessions",
        "Claude Team \u{b7} sessions \u{b7} period total",
    ),
    (
        "ai_bullet_rows.cc_tool_acceptance",
        "Claude Team \u{b7} accepted \u{f7} offered \u{b7} daily avg",
    ),
    (
        "ai_bullet_rows.team_ai_loc",
        "Cursor + Claude Code + Codex \u{b7} accepted lines \u{b7} period total",
    ),
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
        tracing::info!(
            fixed = SUBLABEL_FIXES.len(),
            "ai bullet sublabel drift corrected"
        );
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

    /// No leftover "Anthropic Enterprise API" / "`OpenAI` API" attribution in the
    /// corrected sublabels, and `team_ai_loc` names Codex.
    #[test]
    fn sublabels_have_correct_source_attribution() {
        for (key, sub) in SUBLABEL_FIXES {
            assert!(
                !sub.contains("Anthropic Enterprise API"),
                "{key}: stale Anthropic label"
            );
            assert!(!sub.contains("OpenAI API"), "{key}: stale OpenAI label");
        }
        // `find` + `matches!` (not unwrap/expect — the crate denies both):
        // team_ai_loc must be present AND its sublabel must name Codex.
        let team = SUBLABEL_FIXES
            .iter()
            .find(|(k, _)| *k == "ai_bullet_rows.team_ai_loc");
        assert!(
            matches!(team, Some((_, sub)) if sub.contains("Codex")),
            "team_ai_loc sublabel must be present and include Codex"
        );
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
    /// `ai_person_period` sum-branch (counters). Mirrors the multiIf in the
    /// latest period-rollup migration (20260618000000_ai-claude-team-overage-gold.sql,
    /// which re-set the view to add `cc_overage`; previously 20260610000000) —
    /// keep in sync.
    const SUM_KEYS: &[&str] = &[
        "chatgpt",
        "cc_lines",
        "cc_sessions",
        "cursor_agents",
        "cursor_lines",
        "claude_web",
        "cursor_completions",
        "team_ai_loc",
        "codex_lines",
        "codex_sessions",
        "cc_offered",
        "cc_tool_accept",
        "cc_cost",
        "cc_overage",
        "prs_total",
        "prs_with_cc",
        "cursor_offered",
        "cursor_total_lines",
    ];
    /// `ai_person_period` max-branch (0/1 active markers).
    const MAX_KEYS: &[&str] = &[
        "active_ai_members",
        "cursor_active",
        "cc_active",
        "codex_active",
        "chatgpt_active",
    ];

    /// The `metric_keys` actually EMITTED into `insight.ai_bullet_rows` by the gold
    /// view (its ARRAY JOIN branches; latest = 20260618000000, branches 1–6) —
    /// these are the only keys that reach `ai_person_period` and must therefore be
    /// classified. NB this is the GOLD key set, NOT the `query_ref` ARRAY JOIN: the
    /// latter also lists query_ref-computed ratios (`cursor_acceptance`,
    /// `cc_tool_acceptance`, `ai_loc_share2`) and the `claude_web` stub, which are
    /// never emitted to `ai_bullet_rows` and so never hit the period rollup. prs_*
    /// were removed (honest-NULL) so they are absent here too.
    ///
    /// ⚠️ This list is hand-maintained (mirrors the gold view's branches). A new
    /// gold branch key MUST be added here AND classified in `SUM_KEYS/MAX_KEYS`, or
    /// the guard gives false-green (it only checks keys present in this list).
    const BULLET_ROWS_KEYS: &[&str] = &[
        // branch 1 (all dev tools)
        "active_ai_members",
        "team_ai_loc",
        // branch 2 (cursor)
        "cursor_active",
        "cursor_completions",
        "cursor_agents",
        "cursor_lines",
        "cursor_offered",
        "cursor_total_lines",
        // branch 3 (claude code)
        "cc_active",
        "cc_sessions",
        "cc_lines",
        "cc_tool_accept",
        "cc_offered",
        "cc_cost",
        // branch 4 (codex)
        "codex_active",
        "codex_lines",
        "codex_sessions",
        // branch 5 (chatgpt chat)
        "chatgpt_active",
        "chatgpt",
        // branch 6 (claude overage)
        "cc_overage",
    ];

    /// Every key the gold view emits must be classified into EXACTLY one of
    /// sum/max — none may fall to the `avg()` default (the #1286 defect class).
    /// Adding a gold branch key without classifying it fails here.
    #[test]
    fn every_bullet_key_is_classified_not_defaulting_to_avg() {
        for key in BULLET_ROWS_KEYS {
            let n = [SUM_KEYS.contains(key), MAX_KEYS.contains(key)]
                .iter()
                .filter(|b| **b)
                .count();
            assert_eq!(
                n, 1,
                "metric_key '{key}' is emitted to ai_bullet_rows but is in {n} \
                 of sum/max (must be exactly 1). Unclassified → silently \
                 defaults to avg() in ai_person_period — issue #1286."
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

    /// `cc_overage` is a per-period spend counter (twin of `cc_cost`) → sum, never
    /// max/avg. Avg would divide a monthly snapshot by active-day count (#1286).
    #[test]
    fn cc_overage_sums_like_cc_cost() {
        assert!(SUM_KEYS.contains(&"cc_overage"), "cc_overage must sum");
        assert!(
            !MAX_KEYS.contains(&"cc_overage"),
            "cc_overage is not an active flag"
        );
        assert!(
            BULLET_ROWS_KEYS.contains(&"cc_overage"),
            "cc_overage must be listed as an emitted gold key"
        );
    }
}

//! Seed `metric_catalog` + `product-default` `metric_threshold` rows from
//! the frontend's hardcoded metric metadata (Refs #523).
//!
//! Source files (read-only — see `constructorfabric/insight-front` `main`):
//! - `src/api/threshold-config.ts` — `BULLET_DEFS`, `IC_KPI_DEFS`.
//!
//! Maps each FE `metric_key` to a real ClickHouse `<table>.<column>`:
//! - **7 rows** point at real columns on `insight.ic_kpis`
//!   (`tasks_closed`, `bugs_fixed`, `prs_merged`, `pr_cycle_time_h`,
//!   `focus_time_pct`, `ai_loc_share_pct`, `ai_sessions`) — the
//!   `schema-validator` (Refs #521) returns `Ok` on first probe.
//! - **62 rows** point at row-form bullet storage views
//!   (`task_delivery_bullet_rows`, `git_bullet_rows`, etc.) using the FE
//!   `metric_key` as the column-segment. The table exists but the column
//!   does not — those metrics are stored as `metric_key` *values* in the
//!   shared `metric_value` column. The validator marks them
//!   `error/column_not_found`, which is the truthful signal that a
//!   column-form gold view hasn't shipped yet for that metric. The
//!   validator outcome is informational (DESIGN §3.7) — never blocks
//!   writes or reads, and the catalog is fully usable today.
//!
//! Three FE `metric_key`s appear in both `BULLET_DEFS` and `IC_KPI_DEFS`
//! (`bugs_fixed`, `prs_merged`, `pr_cycle_time_h`) — they merge into one
//! catalog row keyed by `ic_kpis.<col>`. The row carries `BULLET_DEFS`'s
//! `label` / `sublabel` / `good` / `warn` (the bullet rendering is the
//! byte-for-byte gate from PRD §12) and `IC_KPI_DEFS`'s `description` /
//! `format`.
//!
//! Out of scope for this migration (deliberately):
//! - **Legacy `analytics.thresholds` DROP.** DESIGN §3.6 splits the seed
//!   from the DROP so a failed seed migration can roll back without
//!   taking the legacy table down with it. The DROP ships in a separate
//!   follow-on migration **≥ 1 release after this one lands**. Do NOT
//!   add a `DROP TABLE analytics.thresholds` here.
//! - **Per-tenant rows.** v1 seeds product-owned metrics only
//!   (`tenant_id IS NULL`); tenant overlay is admin-CRUD-driven (#525).
//! - **Cache flush.** The DESIGN §3.6 seed-migration sequence ends with
//!   `cache_layer.flush_all() → ack`. Migrations don't carry a cache
//!   handle, so the flush fires from `main.rs` immediately after migration
//!   apply via [`crate::infra::cache::catalog_cache`]. No-op today;
//!   #524 swaps in the real Redis prefix purge.

use sea_orm::{ConnectionTrait, Statement, Value};
use sea_orm_migration::prelude::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// One catalog metric + its product-default threshold.
///
/// Decimal columns (`good` / `warn`) are carried as `f64` here — the FE
/// constants are all integers or single-decimal floats (`avg_slip = 3.1`,
/// `overrun_ratio = 1.5`), which round-trip exactly through MariaDB
/// `DECIMAL(20,6)`. If a future seed entry needs higher precision, switch
/// this to a string and bind with `Value::from(&str)`.
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

/// Render `source_tags` as a JSON-array literal for binding into the
/// `metric_catalog.source_tags` JSON column. MariaDB accepts a string
/// argument and validates it as JSON server-side; we escape `"` and `\`
/// defensively, but every value in the seed is a controlled identifier
/// (no embedded quotes today).
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

/// 69 unique catalog rows. Ordering tracks `BULLET_DEFS` first (with the
/// three IC-merged rows in their BULLET positions but keyed to the real
/// `ic_kpis.<col>`), then four IC-only KPIs. Each row's comment trails
/// the FE constant that backs it — the byte-for-byte FE comparison gate
/// from PRD §12 + DESIGN §3.2 needs every value here to match the
/// frontend's view at the time of seeding.
const SEEDS: &[SeedRow] = &[
    // ─────────────────── BULLET_DEFS / task_delivery ───────────────────
    SeedRow {
        metric_key: "task_delivery_bullet_rows.tasks_completed",
        label: "Tasks Closed / Developer",
        sublabel: Some("Jira \u{b7} closed issues \u{b7} per developer \u{b7} period total"),
        description: None,
        unit: Some("tasks"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 5.0,
        warn: 3.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.task_dev_time",
        label: "Task Development Time",
        sublabel: Some(
            "Jira \u{b7} time in dev statuses \u{b7} per-task median \u{b7} lower = better",
        ),
        description: None,
        unit: Some("h"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 15.0,
        warn: 22.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.task_reopen_rate",
        label: "Task Reopen Rate",
        sublabel: Some(
            "Jira \u{b7} reopen events \u{f7} closures (this period) \u{b7} lower = better",
        ),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 5.0,
        warn: 10.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.due_date_compliance",
        label: "Due Date Compliance",
        sublabel: Some("Jira \u{b7} tasks closed by due date"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 72.0,
        warn: 55.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.estimation_accuracy",
        label: "Estimation Accuracy",
        sublabel: Some("Jira \u{b7} how close estimate matches actual time"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 80.0,
        warn: 50.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.worklog_logging_accuracy",
        label: "Worklog Logging Accuracy",
        sublabel: Some(
            "Jira \u{b7} worklog logged \u{f7} time in dev statuses \u{b7} 100 = on target \u{b7} requires both",
        ),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 80.0,
        warn: 50.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.bugs_to_task_ratio",
        label: "Bugs / Tasks Closed",
        sublabel: Some("Jira \u{b7} bug-type issues \u{f7} total closed \u{b7} lower = better"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 20.0,
        warn: 35.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.mean_time_to_resolution",
        label: "Mean Time to Resolution",
        sublabel: Some("Jira \u{b7} close \u{2212} create \u{b7} lower = better"),
        description: None,
        unit: Some("d"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 7.0,
        warn: 14.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.stale_in_progress",
        label: "Stale In-Progress",
        sublabel: Some("Jira \u{b7} open issues untouched > 14 days \u{b7} snapshot, as of today"),
        description: None,
        unit: Some("tasks"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 0.0,
        warn: 2.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.flow_efficiency",
        label: "Flow Efficiency",
        sublabel: Some(
            "Jira \u{b7} time in dev statuses \u{f7} lifetime \u{b7} per-task median \u{b7} higher = better",
        ),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 40.0,
        warn: 20.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.pickup_time",
        label: "Pickup Time",
        sublabel: Some(
            "Jira \u{b7} created \u{2192} first dev status \u{b7} per-task median \u{b7} lower = better",
        ),
        description: None,
        unit: Some("d"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 3.0,
        warn: 7.0,
    },
    // ─────────────────── BULLET_DEFS / git_output ───────────────────
    SeedRow {
        metric_key: "git_bullet_rows.commits",
        label: "Commits Authored",
        sublabel: Some(
            "Bitbucket \u{b7} commits authored \u{b7} period total \u{b7} excl. merge commits",
        ),
        description: None,
        unit: Some("count"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["bitbucket"],
        good: 30.0,
        warn: 10.0,
    },
    SeedRow {
        metric_key: "git_bullet_rows.prs_created",
        label: "Pull Requests Created",
        sublabel: Some("Bitbucket \u{b7} PRs authored \u{b7} period total"),
        description: None,
        unit: Some("count"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["bitbucket"],
        good: 6.0,
        warn: 2.0,
    },
    // BULLET `prs_merged` + IC_KPI `prs_merged` merged onto the real
    // `ic_kpis.prs_merged` column. Label / sublabel / good / warn from
    // BULLET (byte-for-byte gate); description / format from IC.
    SeedRow {
        metric_key: "ic_kpis.prs_merged",
        label: "Pull Requests Merged",
        sublabel: Some("Bitbucket \u{b7} authored and merged \u{b7} period total"),
        description: Some(
            "Pull requests authored and merged. Source not ingested yet — \
             cell shows ComingSoon until Bitbucket PR ingestion lands.",
        ),
        unit: Some("count"),
        format: Some("integer"),
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["bitbucket"],
        good: 5.0,
        warn: 2.0,
    },
    SeedRow {
        metric_key: "git_bullet_rows.clean_loc",
        label: "Clean LOC",
        sublabel: Some("Bitbucket \u{b7} lines added \u{b7} excl. spec/config \u{b7} period total"),
        description: None,
        unit: Some("count"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["bitbucket"],
        good: 5000.0,
        warn: 1000.0,
    },
    // BULLET `pr_cycle_time_h` + IC_KPI `pr_cycle_time_h` merged onto
    // the real `ic_kpis.pr_cycle_time_h` column.
    SeedRow {
        metric_key: "ic_kpis.pr_cycle_time_h",
        label: "Pull Request Cycle Time",
        sublabel: Some(
            "Bitbucket \u{b7} PR opened \u{2192} merged \u{b7} per-PR median \u{b7} lower = better",
        ),
        description: Some(
            "Average hours from PR opened to merged. Source not ingested yet — \
             cell shows ComingSoon until Bitbucket PR ingestion lands.",
        ),
        unit: Some("h"),
        format: Some("hours"),
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["bitbucket"],
        good: 24.0,
        warn: 48.0,
    },
    SeedRow {
        metric_key: "git_bullet_rows.pr_size",
        label: "PR Size",
        sublabel: Some(
            "Bitbucket \u{b7} lines changed per PR \u{b7} per-PR median \u{b7} smaller = reviewable",
        ),
        description: None,
        unit: Some("count"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["bitbucket"],
        good: 200.0,
        warn: 500.0,
    },
    SeedRow {
        metric_key: "git_bullet_rows.merge_rate",
        label: "PR Merge Rate",
        sublabel: Some(
            "Bitbucket \u{b7} \u{3a3} merged \u{f7} \u{3a3} created over period \u{b7} higher = better",
        ),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["bitbucket"],
        good: 80.0,
        warn: 50.0,
    },
    SeedRow {
        metric_key: "git_bullet_rows.lines_per_commit",
        label: "Lines / Commit",
        sublabel: Some(
            "Bitbucket \u{b7} \u{3a3} LOC \u{f7} \u{3a3} commits \u{b7} lower = reviewable",
        ),
        description: None,
        unit: Some("count"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["bitbucket"],
        good: 100.0,
        warn: 200.0,
    },
    SeedRow {
        metric_key: "git_bullet_rows.commits_per_active_day",
        label: "Commits / Active Day",
        sublabel: Some(
            "Bitbucket \u{b7} \u{3a3} commits \u{f7} days with any commit \u{b7} cadence",
        ),
        description: None,
        unit: Some("count"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["bitbucket"],
        good: 3.0,
        warn: 1.0,
    },
    // ─────────────────── BULLET_DEFS / code_quality ───────────────────
    SeedRow {
        metric_key: "code_quality_bullet_rows.prs_per_dev",
        label: "Pull Requests Merged / Developer",
        sublabel: Some(
            "Bitbucket \u{b7} authored and merged \u{b7} per developer \u{b7} period total",
        ),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["bitbucket"],
        good: 6.0,
        warn: 3.0,
    },
    SeedRow {
        metric_key: "code_quality_bullet_rows.build_success",
        label: "Build Success Rate",
        sublabel: Some("CI \u{b7} passed \u{f7} total runs \u{b7} target \u{2265}90%"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["ci"],
        good: 90.0,
        warn: 80.0,
    },
    SeedRow {
        metric_key: "code_quality_bullet_rows.pr_cycle_time",
        label: "Pull Request Cycle Time",
        sublabel: Some("Bitbucket \u{b7} PR opened \u{2192} merged \u{b7} lower = better"),
        description: None,
        unit: Some("h"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["bitbucket"],
        good: 22.0,
        warn: 28.0,
    },
    // BULLET `bugs_fixed` + IC_KPI `bugs_fixed` merged onto
    // `ic_kpis.bugs_fixed` (real column, validator returns Ok).
    SeedRow {
        metric_key: "ic_kpis.bugs_fixed",
        label: "Bugs Fixed",
        sublabel: Some("Jira \u{b7} bug-type issues closed \u{b7} period total"),
        description: Some(
            "Bug-type Jira issues closed in the selected period. Reflects \
             quality contribution and team reliability.",
        ),
        unit: Some("count"),
        format: Some("integer"),
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 3.0,
        warn: 1.0,
    },
    // ─────────────────── BULLET_DEFS / estimation (placeholders) ───────
    // FE section is `estimation` (`threshold-config.ts` lines 187-191),
    // but the FE comment notes "backend does not yet emit an estimation
    // bullet view". When the backend ships one, these `metric_key`s will
    // re-target the new gold view. For now they route through
    // `task_delivery_bullet_rows` because the underlying Jira data flows
    // through Jira-task-delivery silver — the closest existing storage.
    // Validator marks them `column_not_found`, accurate signal.
    SeedRow {
        metric_key: "task_delivery_bullet_rows.overrun_ratio",
        label: "Median overrun ratio",
        sublabel: Some("Jira \u{b7} actual \u{f7} estimated \u{b7} lower = better"),
        description: None,
        unit: Some("\u{d7}"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 1.5,
        warn: 2.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.scope_completion",
        label: "Scope Completion Rate",
        sublabel: Some("Jira \u{b7} tasks done \u{f7} committed at sprint start"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 75.0,
        warn: 60.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.scope_creep",
        label: "Scope Creep Rate",
        sublabel: Some("Jira \u{b7} added mid-sprint \u{f7} original count \u{b7} lower = better"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 19.0,
        warn: 30.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.on_time_delivery",
        label: "On-time Delivery Rate",
        sublabel: Some("Jira \u{b7} closed by due date"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 70.0,
        warn: 55.0,
    },
    SeedRow {
        metric_key: "task_delivery_bullet_rows.avg_slip",
        label: "Avg Slip When Late",
        sublabel: Some("Jira \u{b7} days past due date \u{b7} lower = better"),
        description: None,
        unit: Some("d"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 3.1,
        warn: 4.5,
    },
    // ─────────────────── BULLET_DEFS / ai_adoption ───────────────────
    SeedRow {
        metric_key: "ai_bullet_rows.active_ai_members",
        label: "Active members",
        sublabel: Some("Cursor \u{b7} Claude Code \u{b7} Codex \u{b7} any activity this period"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: true,
        source_tags: &["cursor", "claude_code", "codex"],
        good: 6.0,
        warn: 3.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.cursor_active",
        label: "Cursor \u{2014} active members",
        sublabel: Some("Cursor \u{b7} any activity this period"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: true,
        source_tags: &["cursor"],
        good: 5.0,
        warn: 3.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.cc_active",
        label: "Claude Code \u{2014} active members",
        sublabel: Some("Anthropic Enterprise API \u{b7} any activity this period"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: true,
        source_tags: &["claude_code"],
        good: 3.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.codex_active",
        label: "Codex \u{2014} active members",
        sublabel: Some("OpenAI API \u{b7} any activity this period"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: true,
        source_tags: &["codex"],
        good: 2.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.team_ai_loc",
        label: "Team AI Accepted Lines",
        sublabel: Some("Cursor + Claude Code \u{b7} accepted lines \u{b7} period total"),
        description: None,
        unit: Some("lines"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["cursor", "claude_code"],
        good: 1000.0,
        warn: 300.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.cursor_acceptance",
        label: "Cursor Acceptance Rate",
        sublabel: Some("Cursor \u{b7} accepted \u{f7} shown completions \u{b7} daily avg"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["cursor"],
        good: 55.0,
        warn: 35.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.cc_tool_acceptance",
        label: "Claude Code Tool Acceptance",
        sublabel: Some("Anthropic Enterprise API \u{b7} accepted \u{f7} offered \u{b7} daily avg"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["claude_code"],
        good: 60.0,
        warn: 40.0,
    },
    // FE alias of cc_tool_acceptance — same metric, kept as separate row
    // per the FE constant. A future FE refactor can collapse them.
    SeedRow {
        metric_key: "ai_bullet_rows.cc_tool_accept",
        label: "Claude Code Tool Acceptance",
        sublabel: Some("Anthropic Enterprise API \u{b7} accepted \u{f7} offered \u{b7} daily avg"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["claude_code"],
        good: 60.0,
        warn: 40.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.cursor_lines",
        label: "Cursor Accepted Lines",
        sublabel: Some("Cursor \u{b7} lines accepted from AI suggestions \u{b7} period total"),
        description: None,
        unit: Some("lines"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["cursor"],
        good: 100.0,
        warn: 30.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.cursor_agents",
        label: "Cursor Agent Interactions",
        sublabel: Some("Cursor \u{b7} agent-mode actions \u{b7} period total"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["cursor"],
        good: 10.0,
        warn: 3.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.cursor_completions",
        label: "Cursor Completions",
        sublabel: Some("Cursor \u{b7} inline completions offered \u{b7} period total"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["cursor"],
        good: 30.0,
        warn: 10.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.cc_lines",
        label: "Claude Code Accepted Lines",
        sublabel: Some("Anthropic Enterprise API \u{b7} accepted lines \u{b7} period total"),
        description: None,
        unit: Some("lines"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["claude_code"],
        good: 50.0,
        warn: 10.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.cc_sessions",
        label: "Claude Code Sessions",
        sublabel: Some("Anthropic Enterprise API \u{b7} sessions \u{b7} period total"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["claude_code"],
        good: 4.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.chatgpt",
        label: "ChatGPT Activity",
        sublabel: Some("ChatGPT Team \u{b7} interactions \u{b7} period total"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["chatgpt"],
        good: 10.0,
        warn: 0.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.claude_web",
        label: "Claude.ai Activity",
        sublabel: Some("Claude.ai web \u{b7} interactions \u{b7} period total"),
        description: None,
        unit: None,
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["claude_web"],
        good: 10.0,
        warn: 0.0,
    },
    SeedRow {
        metric_key: "ai_bullet_rows.ai_loc_share2",
        label: "AI Code Acceptance",
        sublabel: Some("Cursor + Claude Code \u{b7} accepted \u{f7} clean LOC \u{b7} daily avg"),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["cursor", "claude_code"],
        good: 14.0,
        warn: 8.0,
    },
    // ─────────────────── BULLET_DEFS / collaboration ───────────────────
    SeedRow {
        metric_key: "collab_bullet_rows.slack_messages_sent",
        label: "Messages Sent",
        sublabel: Some("Slack \u{b7} messages sent \u{b7} period total"),
        description: None,
        unit: Some("messages"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["slack"],
        good: 100.0,
        warn: 40.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.slack_channel_posts",
        label: "Channel Posts",
        sublabel: Some("Slack \u{b7} posts in public channels \u{b7} period total"),
        description: None,
        unit: Some("messages"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["slack"],
        good: 25.0,
        warn: 10.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.slack_active_days",
        label: "Active Days",
        sublabel: Some("Slack \u{b7} days with any messages \u{b7} period total"),
        description: None,
        unit: Some("days"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["slack"],
        good: 15.0,
        warn: 8.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.slack_msgs_per_active_day",
        label: "Messages per Active Day",
        sublabel: Some("Slack \u{b7} messages \u{f7} active days \u{b7} daily avg"),
        description: None,
        unit: Some("messages/day"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["slack"],
        good: 10.0,
        warn: 4.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.slack_dm_ratio",
        label: "DM Ratio",
        sublabel: Some(
            "Slack \u{b7} DMs \u{f7} all messages \u{b7} daily avg \u{b7} lower = more open",
        ),
        description: None,
        unit: Some("%"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["slack"],
        good: 30.0,
        warn: 50.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.m365_active_days",
        label: "Active Days",
        sublabel: Some("M365 \u{b7} days with any sent / chat / file activity \u{b7} period total"),
        description: None,
        unit: Some("days"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365"],
        good: 18.0,
        warn: 10.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.m365_emails_sent",
        label: "Emails Sent",
        sublabel: Some("M365 \u{b7} emails sent \u{b7} period total"),
        description: None,
        unit: Some("emails"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365"],
        good: 30.0,
        warn: 5.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.m365_emails_received",
        label: "Emails Received",
        sublabel: Some("M365 \u{b7} inbox volume \u{b7} period total"),
        description: None,
        unit: Some("emails"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365"],
        good: 100.0,
        warn: 30.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.m365_emails_read",
        label: "Emails Read",
        sublabel: Some("M365 \u{b7} emails read \u{b7} period total"),
        description: None,
        unit: Some("emails"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365"],
        good: 200.0,
        warn: 50.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.m365_teams_chats",
        label: "Teams Chats",
        sublabel: Some("Microsoft Teams \u{b7} DMs and group chats \u{b7} period total"),
        description: None,
        unit: Some("messages"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365"],
        good: 50.0,
        warn: 20.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.m365_files_engaged",
        label: "Files Engaged",
        sublabel: Some("M365 \u{b7} files viewed or edited \u{b7} period total"),
        description: None,
        unit: Some("files"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365"],
        good: 40.0,
        warn: 15.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.m365_files_shared_internal",
        label: "Files Shared (Internal)",
        sublabel: Some("M365 \u{b7} files shared inside org \u{b7} period total"),
        description: None,
        unit: Some("files"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365"],
        good: 6.0,
        warn: 2.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.m365_files_shared_external",
        label: "Files Shared (External)",
        sublabel: Some(
            "M365 \u{b7} files shared outside org \u{b7} period total \u{b7} governance signal",
        ),
        description: None,
        unit: Some("files"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365"],
        good: 1.0,
        warn: 0.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.meeting_hours",
        label: "Meeting Hours",
        sublabel: Some("Teams + Zoom \u{b7} longest modality per meeting \u{b7} period total"),
        description: None,
        unit: Some("h"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["m365", "zoom"],
        good: 40.0,
        warn: 80.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.meetings_count",
        label: "Meetings Attended",
        sublabel: Some("Teams + Zoom \u{b7} distinct meetings joined \u{b7} period total"),
        description: None,
        unit: Some("meetings"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365", "zoom"],
        good: 40.0,
        warn: 10.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.teams_meeting_hours",
        label: "Teams Meeting Hours",
        sublabel: Some("Microsoft Teams \u{b7} longest modality per meeting \u{b7} period total"),
        description: None,
        unit: Some("h"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["m365"],
        good: 30.0,
        warn: 60.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.zoom_meeting_hours",
        label: "Zoom Meeting Hours",
        sublabel: Some("Zoom \u{b7} longest modality per meeting \u{b7} period total"),
        description: None,
        unit: Some("h"),
        format: None,
        higher_is_better: false,
        is_member_scale: false,
        source_tags: &["zoom"],
        good: 20.0,
        warn: 40.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.teams_meetings",
        label: "Teams Meetings Attended",
        sublabel: Some("Microsoft Teams \u{b7} distinct meetings joined \u{b7} period total"),
        description: None,
        unit: Some("meetings"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365"],
        good: 25.0,
        warn: 5.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.zoom_meetings",
        label: "Zoom Meetings Attended",
        sublabel: Some("Zoom \u{b7} distinct meetings joined \u{b7} period total"),
        description: None,
        unit: Some("meetings"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["zoom"],
        good: 15.0,
        warn: 3.0,
    },
    SeedRow {
        metric_key: "collab_bullet_rows.meeting_free",
        label: "Meeting-Free Days",
        sublabel: Some(
            "Teams + Zoom \u{b7} days with any record but no meeting time \u{b7} period total",
        ),
        description: None,
        unit: Some("days"),
        format: None,
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365", "zoom"],
        good: 4.0,
        warn: 2.0,
    },
    // ─────────────────── IC_KPI_DEFS unique-to-IC rows ───────────────────
    // IC `ai_loc_share` → real column `ic_kpis.ai_loc_share_pct`
    // (the IC raw_field already names this mapping). good/warn from
    // FE `metric-semantics.ts` `METRIC_SEMANTICS.ai_loc_share_pct`
    // — same FE source family (no external default invented).
    SeedRow {
        metric_key: "ic_kpis.ai_loc_share_pct",
        label: "AI Code Acceptance",
        sublabel: Some("Cursor + Claude Code"),
        description: Some(
            "Share of authored lines accepted from AI suggestions (Cursor + Claude Code). \
             Reflects how much AI tooling contributes to actual output.",
        ),
        unit: Some("%"),
        format: Some("percent"),
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["cursor", "claude_code"],
        good: 20.0,
        warn: 10.0,
    },
    // good/warn from `METRIC_SEMANTICS.focus_time_pct`.
    SeedRow {
        metric_key: "ic_kpis.focus_time_pct",
        label: "Focus Time",
        sublabel: Some("Calendar / M365"),
        description: Some(
            "Share of work time spent in uninterrupted 60-minute+ blocks. \
             Higher means fewer context switches and more deep work.",
        ),
        unit: Some("%"),
        format: Some("percent"),
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["m365"],
        good: 60.0,
        warn: 50.0,
    },
    // No FE good/warn for `tasks_closed`. Reused BULLET sibling
    // `tasks_completed` (5/3) — same Jira-closed-tasks semantic and the
    // closest defensible numeric default. Documented here so it isn't
    // mistaken for product policy if a reviewer revisits.
    SeedRow {
        metric_key: "ic_kpis.tasks_closed",
        label: "Tasks Closed",
        sublabel: Some("Jira"),
        description: Some(
            "Jira tasks moved to Done in the selected period. Direct measure of \
             delivery throughput.",
        ),
        unit: None,
        format: Some("integer"),
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["jira"],
        good: 5.0,
        warn: 3.0,
    },
    // No FE good/warn for `ai_sessions`. Reused BULLET sibling
    // `cc_sessions` (4/1) — same Cursor-sessions-class semantic.
    SeedRow {
        metric_key: "ic_kpis.ai_sessions",
        label: "AI Sessions",
        sublabel: Some("Cursor"),
        description: Some(
            "Distinct Cursor sessions in the selected period. Proxy for how often \
             AI tooling is engaged.",
        ),
        unit: None,
        format: Some("integer"),
        higher_is_better: true,
        is_member_scale: false,
        source_tags: &["cursor"],
        good: 4.0,
        warn: 1.0,
    },
];

// `source_tags` is JSON-typed. MariaDB stores JSON as a LONGTEXT alias and
// rejects the explicit `CAST(? AS JSON)` syntax that MySQL accepts (the
// parser breaks at the `JSON)` token on 11.8.7). A bare string bind works
// — the JSON-shape invariant is enforced by the CHECK on `source_tags` plus
// `every_seed_has_nonempty_source_tags` at the SEEDS-list level. See
// MDEV-13252 for the MariaDB stance on CAST AS JSON. Refs #523.
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
            "metric_catalog seed migration applied"
        );

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // we have only forward migrations
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::domain::schema_validator::parse::parse_metric_key;

    #[test]
    fn seed_list_is_non_empty() {
        assert!(
            !SEEDS.is_empty(),
            "seed migration must emit at least one row"
        );
    }

    #[test]
    fn seed_count_matches_unique_fe_metric_keys() {
        // Pinned count derived from BULLET_DEFS (65) + IC_KPI_DEFS minus
        // the 3 BULLET/IC duplicates merged onto ic_kpis.*. Bumping this
        // requires re-running the FE inventory in the plan and updating
        // the row comments — silent drift here would mean a stale catalog
        // vs the rendered FE.
        assert_eq!(SEEDS.len(), 69, "expected 69 unique catalog rows");
    }

    #[test]
    fn every_metric_key_passes_db_check_regex() {
        for row in SEEDS {
            parse_metric_key(row.metric_key).unwrap_or_else(|e| {
                panic!(
                    "seed metric_key {key:?} fails app-layer parser \
                     (would also violate chk_metric_catalog_metric_key_shape): {e}",
                    key = row.metric_key
                )
            });
        }
    }

    #[test]
    fn no_duplicate_metric_keys() {
        let mut seen: HashSet<&str> = HashSet::with_capacity(SEEDS.len());
        for row in SEEDS {
            assert!(
                seen.insert(row.metric_key),
                "duplicate seed metric_key {:?} would violate \
                 uq_metric_catalog_metric_key on first apply",
                row.metric_key
            );
        }
    }

    #[test]
    fn every_metric_key_uses_a_known_clickhouse_table_segment() {
        // Pin the set of CH tables we route metric_keys through. Adding a
        // new segment requires touching this list AND adding the gold view
        // (or accepting that the schema-validator will report
        // table_not_found for it). Catches typos like
        // `task_dlivery_bullet_rows.foo` slipping into the seed.
        const KNOWN_TABLES: &[&str] = &[
            "ic_kpis",
            "task_delivery_bullet_rows",
            "code_quality_bullet_rows",
            "collab_bullet_rows",
            "ai_bullet_rows",
            "git_bullet_rows",
        ];
        for row in SEEDS {
            let parsed = parse_metric_key(row.metric_key)
                .unwrap_or_else(|e| panic!("unparseable metric_key {:?}: {e}", row.metric_key));
            assert!(
                KNOWN_TABLES.contains(&parsed.table),
                "metric_key {key:?} routes to unknown CH table {table:?}; \
                 either add the table to KNOWN_TABLES (and confirm the gold \
                 view exists in src/ingestion/scripts/migrations/) or fix \
                 the typo",
                key = row.metric_key,
                table = parsed.table,
            );
        }
    }

    #[test]
    fn no_alert_columns_seeded_in_v1() {
        // Per #523: alert_trigger / alert_bad only seeded if the FE has
        // them. BULLET_DEFS + IC_KPI_DEFS carry neither — admin CRUD
        // (#525) lands alerts post-seed. The INSERT SQL omits both
        // columns, which means MariaDB defaults them to NULL. This test
        // pins that the seed migration does NOT reference them.
        assert!(
            !INSERT_THRESHOLD_SQL.contains("alert_trigger"),
            "v1 seed must NOT insert alert_trigger — alert values land via #525"
        );
        assert!(
            !INSERT_THRESHOLD_SQL.contains("alert_bad"),
            "v1 seed must NOT insert alert_bad — alert values land via #525"
        );
    }

    #[test]
    fn does_not_touch_legacy_analytics_thresholds() {
        // Cross-file guard: the legacy `analytics.thresholds` DROP is a
        // separate follow-on migration per DESIGN §3.6. Catch a copy-paste
        // mistake before review.
        for sql in [INSERT_CATALOG_SQL, INSERT_THRESHOLD_SQL] {
            assert!(
                !sql.to_lowercase().contains("analytics.thresholds"),
                "seed migration must not reference legacy analytics.thresholds; \
                 that DROP ships in a follow-on migration ≥1 release later"
            );
        }
    }

    #[test]
    fn upsert_keeps_id_stable_on_replay() {
        // ON DUPLICATE KEY UPDATE must NOT overwrite `id` — the UUIDv7
        // generated on first apply is the stable wire identifier
        // consumers cache against. A replay (after a partial failure)
        // re-runs the INSERT with a fresh `Uuid::now_v7()`, but ON
        // DUPLICATE KEY UPDATE only touches the columns we name. Pin
        // that `id` is absent from the SET clause.
        let upper = INSERT_CATALOG_SQL.to_uppercase();
        let Some(set_clause) = upper.split("ON DUPLICATE KEY UPDATE").nth(1) else {
            panic!("upsert must contain ON DUPLICATE KEY UPDATE");
        };
        assert!(
            !set_clause.contains("ID ="),
            "ON DUPLICATE KEY UPDATE must not touch `id` — \
             UUIDv7 is the stable wire identifier per DESIGN §3.7"
        );
        assert!(
            !set_clause.contains("METRIC_KEY ="),
            "ON DUPLICATE KEY UPDATE must not touch `metric_key` — \
             it's the unique-conflict column"
        );
    }

    #[test]
    fn source_tags_json_array_is_well_formed_for_empty() {
        assert_eq!(source_tags_json(&[]), "[]");
    }

    #[test]
    fn source_tags_json_array_is_well_formed_for_one() {
        assert_eq!(source_tags_json(&["jira"]), "[\"jira\"]");
    }

    #[test]
    fn source_tags_json_array_is_well_formed_for_many() {
        assert_eq!(
            source_tags_json(&["cursor", "claude_code", "codex"]),
            "[\"cursor\",\"claude_code\",\"codex\"]"
        );
    }

    #[test]
    fn source_tags_json_escapes_special_chars() {
        // Defensive — every value in `SEEDS` is a controlled identifier
        // today (no embedded quotes or backslashes), but the escaper
        // is the boundary that has to stay safe if a future tag
        // includes them.
        assert_eq!(source_tags_json(&["he\"y"]), "[\"he\\\"y\"]");
        assert_eq!(source_tags_json(&["a\\b"]), "[\"a\\\\b\"]");
    }

    #[test]
    fn every_seed_has_nonempty_source_tags() {
        // `source_tags` is `JSON NOT NULL` in the schema (DESIGN §3.7
        // line 963: "always an array"). Empty arrays would pass the
        // NOT NULL but break the connector-readiness diagnostics in
        // PRD §13. Guard against accidental `&[]` in a future row.
        for row in SEEDS {
            assert!(
                !row.source_tags.is_empty(),
                "metric {:?} has empty source_tags — every metric MUST list \
                 at least one connector",
                row.metric_key
            );
        }
    }
}

# GitHub Copilot Connector Specification

> Version 1.0 — March 2026
> Based on: `docs/CONNECTORS_REFERENCE.md` Source 15 (GitHub Copilot)

Standalone specification for the GitHub Copilot (AI Dev Tool) connector. Expands Source 15 in the main Connector Reference with full table schemas, identity mapping, Silver/Gold pipeline notes, and open questions.

<!-- toc -->

- [Overview](#overview)
- [Bronze Tables](#bronze-tables)
  - [`copilot_seats` — Seat assignment and last activity](#copilotseats-seat-assignment-and-last-activity)
  - [`copilot_usage` — Org-level daily usage totals](#copilotusage-org-level-daily-usage-totals)
  - [`copilot_usage_breakdown` — Daily breakdown by language and editor](#copilotusagebreakdown-daily-breakdown-by-language-and-editor)
  - [`copilot_collection_runs` — Connector execution log](#copilotcollectionruns-connector-execution-log)
- [Identity Resolution](#identity-resolution)
- [Silver / Gold Mappings](#silver-gold-mappings)
- [Open Questions](#open-questions)
  - [OQ-COP-1: No per-user daily metrics — impact on Silver unification](#oq-cop-1-no-per-user-daily-metrics-impact-on-silver-unification)
  - [OQ-COP-2: `last_activity_at` as a proxy for active usage](#oq-cop-2-lastactivityat-as-a-proxy-for-active-usage)

<!-- /toc -->

---

## Overview

**API**: GitHub REST API — `/orgs/{org}/copilot/*` endpoints

**Category**: AI Dev Tool

**Authentication**: GitHub App installation token or PAT with `manage_billing:copilot` scope

**Identity**: `user_email` in `copilot_seats` — resolved to canonical `person_id` via Identity Manager. `copilot_usage` and `copilot_usage_breakdown` are org-level and have no user attribution.

**Field naming**: snake_case — GitHub API uses snake_case; preserved as-is at Bronze level.

**Why three tables**: Seats (per-user roster), org-level daily totals, and per-language/editor breakdown are three distinct entities that cannot be merged. `copilot_usage` and `copilot_usage_breakdown` have no user-level data.

**Key structural difference from Cursor/Windsurf**: The GitHub Copilot API does not expose per-user daily usage. It provides:
- Per-seat last-activity timestamps (`copilot_seats`)
- Org-level daily aggregates (`copilot_usage`)
- Language × editor breakdown without per-user data (`copilot_usage_breakdown`)

No per-user token counts or per-user daily metrics exist in the standard API.

---

## Bronze Tables

### `copilot_seats` — Seat assignment and last activity

| Field | Type | Description |
|-------|------|-------------|
| `user_login` | String | GitHub login of the seat holder |
| `user_email` | String | Email (from linked GitHub account) — identity resolution key |
| `plan_type` | String | `business` / `enterprise` |
| `pending_cancellation_date` | Date | If seat is scheduled for cancellation (NULL otherwise) |
| `last_activity_at` | DateTime64(3) | Last recorded Copilot activity across all editors |
| `last_activity_editor` | String | Editor used in last activity, e.g. `vscode`, `jetbrains` |
| `created_at` | DateTime64(3) | When the seat was assigned |
| `updated_at` | DateTime64(3) | Last seat record update |

One row per user. `last_activity_at` is the only per-user usage signal available in the Copilot API.

---

### `copilot_usage` — Org-level daily usage totals

| Field | Type | Description |
|-------|------|-------------|
| `date` | Date | Usage date — primary key |
| `total_suggestions_count` | Float64 | Code completion suggestions shown |
| `total_acceptances_count` | Float64 | Suggestions accepted (tab) |
| `total_lines_suggested` | Float64 | Lines of code suggested |
| `total_lines_accepted` | Float64 | Lines of code accepted |
| `total_active_users` | Float64 | Users with at least one completion interaction |
| `total_chat_turns` | Float64 | Copilot Chat interactions (IDE + github.com) |
| `total_chat_acceptances` | Float64 | Code blocks accepted from chat |
| `total_active_chat_users` | Float64 | Users who used Copilot Chat |

Org-level only — no per-user breakdown. Enables trend analysis of overall adoption without individual attribution.

---

### `copilot_usage_breakdown` — Daily breakdown by language and editor

| Field | Type | Description |
|-------|------|-------------|
| `date` | Date | Usage date |
| `language` | String | Programming language, e.g. `python`, `typescript`, `go` |
| `editor` | String | Editor, e.g. `vscode`, `jetbrains`, `neovim`, `vim`, `xcode` |
| `suggestions_count` | Float64 | Suggestions shown for this language × editor |
| `acceptances_count` | Float64 | Suggestions accepted |
| `lines_suggested` | Float64 | Lines suggested |
| `lines_accepted` | Float64 | Lines accepted |
| `active_users` | Float64 | Active users for this language × editor combination |

One row per `(date, language, editor)`. Enables analysis of adoption by editor and language without per-user resolution.

---

### `copilot_collection_runs` — Connector execution log

| Field | Type | Description |
|-------|------|-------------|
| `run_id` | String | Unique run identifier |
| `started_at` | DateTime64(3) | Run start time |
| `completed_at` | DateTime64(3) | Run end time |
| `status` | String | `running` / `completed` / `failed` |
| `seats_collected` | Float64 | Rows collected for `copilot_seats` |
| `usage_records_collected` | Float64 | Rows collected for `copilot_usage` |
| `breakdown_records_collected` | Float64 | Rows collected for `copilot_usage_breakdown` |
| `api_calls` | Float64 | API calls made |
| `errors` | Float64 | Errors encountered |
| `settings` | String | Collection configuration (org, lookback period) |

Monitoring table — not an analytics source.

---

## Identity Resolution

Only `copilot_seats` has user-level data. `user_email` is the primary identity key — resolved to canonical `person_id` via Identity Manager.

`user_login` (GitHub username) is a secondary identifier — useful for cross-referencing with `github_commits.author_login` but not used as the primary identity key.

`copilot_usage` and `copilot_usage_breakdown` have no user attribution — org-level aggregate data only.

---

## Silver / Gold Mappings

| Bronze table | Silver target | Status |
|-------------|--------------|--------|
| `copilot_seats` | `class_ai_dev_usage` | Planned — `last_activity_at` as binary active signal (no completions metrics) |
| `copilot_usage` + `copilot_usage_breakdown` | `class_ai_org_usage` | Planned — org-level aggregates, no `person_id`; keyed by `(workspace_id, date, tool)` |

**Gold**: GitHub Copilot adoption metrics (active users trend, acceptance rate, lines accepted per day, editor/language distribution) read from Silver `class_ai_org_usage`. Per-user activity signal (binary active/inactive per period) read from Silver `class_ai_dev_usage`. Gold never reads Bronze directly.

---

## Open Questions

### OQ-COP-1: No per-user daily metrics — impact on Silver unification

GitHub Copilot does not expose per-user daily usage. When building `class_ai_dev_usage`:

- Cursor and Windsurf contribute per-user daily rows with acceptance rates, lines added, etc.
- Copilot can only contribute org-level aggregate rows and seat-level last-activity timestamps.

**CLOSED.** `class_ai_org_usage` Silver stream IS created for org-level GitHub Copilot data. Rationale: Bronze data cannot be read directly at Gold level — Identity Resolution has not run on Bronze, and `workspace_id` isolation is enforced at Silver. `copilot_usage` and `copilot_usage_breakdown` feed into `class_ai_org_usage` (keyed by `workspace_id + date + tool`, no `person_id`). For per-user data: `copilot_seats.last_activity_at` feeds into `class_ai_dev_usage` as a binary activity signal (no completions metrics).

### OQ-COP-2: `last_activity_at` as a proxy for active usage

`copilot_seats.last_activity_at` is the only per-user usage timestamp. It is:
- Updated on any Copilot interaction (completion or chat)
- Not granular — no count of interactions, no token usage

- Is `last_activity_at` sufficient to classify a user as "active" for a given period?
- Should the connector be run daily and snapshot `last_activity_at` changes to reconstruct a daily activity binary signal?

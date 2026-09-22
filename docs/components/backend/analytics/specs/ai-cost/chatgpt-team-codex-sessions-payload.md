# `daily-sessions-messages-counts` — vendor payload

Every field the ChatGPT Team endpoint `wham/analytics/daily-sessions-messages-counts`
returns, what we do with it today, and what is simply not established.

The purpose is to let a reader decide what else is worth collecting without
re-reading the vendor. Nothing here is a commitment: a field marked
`not selected yet` is a field nobody has asked for, not one that was rejected.

The endpoint is undocumented by the vendor. Field names below are the vendor's;
semantics are inferred from the names and from the shape of the responses, and
where inference would be a guess the row says so.

## Envelope

The response is `{data: [...], group_by}`. `group_by` reports the grouping the
rows were built with. There is **no count, total or cursor of any kind** — see
"Completeness" below.

## Grain

One row per `(user_id, date)`. The endpoint publishes no email and no display
name, so a row is bound to a person only through the roster, which carries the
same id in its `id` field.

## Fields

`Stored` = reaches `bronze_chatgpt_team.chatgpt_team_codex_sessions_daily`.
`Surfaced` = reaches `class_ai_dev_usage.tool_action_breakdown_json` through
`chatgpt_team__ai_dev_usage`.

### Identity and date

| Vendor field | Type | Semantics | Stored | Surfaced |
|---|---|---|---|---|
| `date` | string `YYYY-MM-DD` | The day the usage falls on. The row carries its own date, unlike the leaderboard's. | yes | join key |
| `user_id` | string, `user-` prefix | The vendor's account id. Equals the roster's `id`. | yes | join key |

### Credits

A credit is the vendor's own consumption unit. `credit_total` is the sum of the
per-surface credits. What is **not** established: why some usage is credited and
some is not — whether an allowance covers the uncredited part or it is simply
unmetered. Either way the credited figure is what an on-demand charge is levied
on.

| Vendor field | Type | Semantics | Stored | Surfaced |
|---|---|---|---|---|
| `credit_total` | float | Sum of the per-surface credits below. | yes | yes, for reconciliation only |
| `on_demand_credits` | float | The vendor's own name for the same figure. Observed equal to `credit_total` wherever both are published. | yes | yes, for reconciliation only |
| `credit_cli` | float | Credits attributed to the CLI surface. | yes | not selected yet |
| `credit_vscode` | float | VS Code extension. | yes | not selected yet |
| `credit_exec` | float | Named `exec`; the vendor does not document what it covers. Inferred to be non-interactive execution, **not established**. | yes | not selected yet |
| `credit_sdk_ts` | float | TypeScript SDK. | yes | not selected yet |
| `credit_desktop` | float | Desktop app. | yes | not selected yet |
| `credit_web` | float | Web surface. | yes | not selected yet |
| `credit_slack` | float | Slack integration. | yes | not selected yet |
| `credit_github_code_review` | float | GitHub code-review integration. | yes | not selected yet |
| `credit_github_turn` | float | GitHub, per turn. How this differs from `credit_github_code_review` is **not established**. | yes | not selected yet |

> **Authority.** These credits are collected to reconcile against the usage
> leaderboard, which remains the authoritative source of the total. The
> leaderboard publishes a `total_users` envelope and a read that lost somebody
> is rejected whole; this endpoint publishes no envelope, so no read of it can
> be judged. Equal values do not make equal guarantees.

### Tokens

| Vendor field | Type | Semantics | Stored | Surfaced |
|---|---|---|---|---|
| `uncached_text_input_tokens` | int | Input tokens billed without cache benefit. | yes | yes |
| `cached_text_input_tokens` | int | Input tokens served from cache. | yes | yes |
| `text_output_tokens` | int | Output tokens. | yes | yes |
| `text_total_tokens` | int | Total. Whether it is exactly the sum of the three above is **not established**. | yes | yes |

### Sessions

`n_new_sessions_*` counts sessions **started** on the day, not sessions active.
A person continuing yesterday's session can record messages and credits while
every one of these reads zero. This is why they are not used to decide activity.

| Vendor field | Type | Stored | Surfaced |
|---|---|---|---|
| `n_new_sessions_total` | int | yes | yes |
| `n_new_sessions_cli` | int | yes | not selected yet |
| `n_new_sessions_vscode` | int | yes | not selected yet |
| `n_new_sessions_exec` | int | yes | not selected yet |
| `n_new_sessions_sdk_ts` | int | yes | not selected yet |
| `n_new_sessions_desktop` | int | yes | not selected yet |
| `n_new_sessions_work_desktop` | int | yes | not selected yet |
| `n_new_sessions_work_web` | int | yes | not selected yet |
| `n_new_sessions_work_mobile` | int | yes | not selected yet |
| `n_new_sessions_other` | int | yes | not selected yet |

The `work_*` prefix distinguishes a work surface from a personal one. What the
vendor counts as `other` is **not established**.

### Messages

| Vendor field | Type | Stored | Surfaced |
|---|---|---|---|
| `n_user_messages_total` | int | yes | yes |
| `n_user_messages_cli` | int | yes | not selected yet |
| `n_user_messages_vscode` | int | yes | not selected yet |
| `n_user_messages_exec` | int | yes | not selected yet |
| `n_user_messages_sdk_ts` | int | yes | not selected yet |
| `n_user_messages_desktop` | int | yes | not selected yet |
| `n_user_messages_work_desktop` | int | yes | not selected yet |
| `n_user_messages_work_web` | int | yes | not selected yet |
| `n_user_messages_work_mobile` | int | yes | not selected yet |
| `n_user_messages_other` | int | yes | not selected yet |

Counts messages the person sent. The assistant's replies are not counted.

### Web task counters

| Vendor field | Type | Semantics | Stored | Surfaced |
|---|---|---|---|---|
| `n_tasks_web` | int | Tasks started on the web surface. | yes | yes |
| `n_code_reviews_web` | int | Code reviews on the web surface. | yes | yes |

### Not collected

| Vendor field | Type | Why not |
|---|---|---|
| `clients` | array of objects | Per-client breakdown, each carrying a `client_id` and its own credit and token counters. Overlaps the per-surface columns above at a different granularity. Nothing downstream reads it; collecting it means either a JSON column or a second relation. |
| `models` | array of objects | Per-model breakdown with `model`, `speed`, credits and token counters. This is the only place the model is named, so it is the field to collect first if per-model cost is ever wanted. |
| `n_users_used_codex` | int | Workspace-wide headcount repeated on every person's row. Copying an organisation-level fact onto each person multiplies it by the roster and makes any sum wrong. Collect as its own day-grain stream if wanted — the pattern exists in `chatgpt_team_codex_user_daily_org`. |
| `n_users_used_work` | int | Same, for the work surfaces. |

## Completeness

The endpoint has no pagination: `page_size` is ignored and `page` is rejected as
an unparseable token. That proves one specific thing — **a page-boundary loss
cannot occur, because there are no pages**.

It does not prove the response is complete. There is no envelope, no expected
count and no cursor, so a partial answer cannot be detected from the payload
itself. Any completeness statement about this endpoint has to come from
somewhere else, which is why the leaderboard, and not this endpoint, carries the
authoritative total.

## Schema drift

The vendor renames fields on these `wham/*` routes without notice; a sibling
endpoint's credit fields were renamed with no version change and no
announcement. A field added to a surface list later arrives as a key this
connector does not declare, and is dropped rather than stored. Re-read this
document against the live payload before relying on a field it lists.

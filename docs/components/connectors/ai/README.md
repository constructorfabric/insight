# AI Providers — Metrics Coverage Matrix

What each connector exposes natively. Snapshot date: 2026-04-28 (verified against public API docs).

**Legend:** ✅ available · 🟡 partial / indirect · ❌ not available

## Changelog

- **2026-04-28** — initial document. Verified against public docs for Cursor, Claude Admin, Claude Enterprise (where Anthropic publishes; remainder against local bronze schema), OpenAI, GitHub Copilot, Windsurf. Added Anthropic source-resolution rules (Admin vs Enterprise vs multi-org).
- **2026-04-28** — added **JetBrains AI** as 7th provider, against the public IDE Services analytics API (`/api/analytics/ai/*`). Only concepts confirmed in published docs are marked ✅; per-tool taxonomy (`junie`, `aia`, `claude_code`, `code_completion`, `nes`) is built into the API. Internal spec `connectors/ai/jetbrains/jetbrains.md` has known divergences from the real API (wrong endpoint name, wrong subscription-gating claim, several speculative fields like `most_used_model` / `models_used` / `ide_product_code` / `chat_sessions`) — see notes in the JetBrains AI section.
- **2026-04-28** — refactored "Anthropic source resolution" → **"Cross-vendor source resolution"**. Silver layer now stores per-source rows verbatim — no field-level merge logic in staging, no priority resolution between Admin/Enterprise/JB at silver. Three gold view contracts handle the merge: `v_ai_usage_per_provider` (pass-through, per-source debugging), `v_ai_usage_unified` (column-level priority list documented in SQL header), `v_ai_users_dau` (`COUNT(DISTINCT email)` for DAU). Resolves prior open question about `claude_code:admin` / `:enterprise` tool-value split — `source` discriminator is sufficient, no tool-name prefixing needed. Documents cross-vendor vantage-point overlap (e.g. JB AI's `tool='claude_code'` row + Anthropic Admin's `tool='claude_code'` row describe the same coding session from different angles — merge by complementary columns, never sum).
- **2026-04-28** — **split "OpenAI" column into three**: `OpenAI API` (platform.openai.com Admin Usage / Cost / Audit Logs), `ChatGPT Enterprise` (Compliance Logs Platform JSONL + SCIM 2.0 — covers Enterprise + Edu), `ChatGPT Business` (no public admin API — documentation column). Corrected prior framing: Compliance Logs Platform is the **ChatGPT Enterprise** surface, not part of platform.openai.com Admin API; ChatGPT app activity and API token usage are **separate** billing/identity domains, not "mixed in usage_completions". ChatGPT Team SKU was renamed to ChatGPT Business on 2025-08-29; our `connectors/ai/chatgpt-team/` directory uses a stale name. The PRD in `chatgpt-team/specs/PRD.md` describes a per-user-per-day-per-client admin endpoint that **does not exist publicly** — needs rewrite against Compliance Logs JSONL semantics. Stateful Compliance API deprecates 2026-06-05.

Connectors:
- **Cursor** (`bronze_cursor.*`) — IDE dev tool
- **Claude Admin** (`bronze_claude_admin.*`) — Anthropic API admin endpoints (`/v1/organizations/usage_report/messages`, `/v1/organizations/usage_report/claude_code`, `/v1/organizations/cost_report`, plus `/v1/organizations/{users,api_keys,workspaces,invites}`)
- **Claude Enterprise** (`bronze_claude_enterprise.*`) — Anthropic Enterprise plan analytics. **Note:** the wire-format endpoint feeding this connector is not publicly documented by Anthropic; only the Claude Code Analytics API (`/v1/organizations/usage_report/claude_code`) overlaps with the `code_*` columns. Field names below are **bronze-table names internal to the connector**, not Anthropic's public API field names.
- **OpenAI API** (`bronze_openai.*`) — `platform.openai.com` Admin Usage API (`/v1/organization/usage/{completions,embeddings,moderations,images,audio_speeches,audio_transcriptions,vector_stores,code_interpreter_sessions}`), Cost API, Audit Logs API. **Pay-per-token billing domain.** Within this stream, Codex CLI vs. raw API requests are separable only by `model ILIKE '%codex%'` heuristic (per [help article 8957039](https://help.openai.com/en/articles/8957039)). ChatGPT app activity does **not** appear in this surface — different domain entirely.
- **ChatGPT Enterprise** (`bronze_chatgpt_enterprise.*`) — *planned only*. Covers ChatGPT **Enterprise** and **Edu** SKUs (same surface). Two endpoints: SCIM 2.0 at `api.openai.com/scim/v2` (seat roster: `/Users`, `/Groups`) and **Compliance Logs Platform** (immutable JSONL streams, HTTP Bearer auth, **30-day retention** so consumers must download continuously, minute-level latency). Event categories in Compliance Logs: `conversations`, `uploaded_files`, `workspace_gpts`, `memories`, `workspace_users`, `admin_audit`, `user_authentication`, `codex_usage`, `app_calls`. **Workspace Analytics is dashboard-only — no API.** Stateful Compliance route deprecates 2026-06-05.
- **ChatGPT Business** (formerly ChatGPT Team — renamed 2025-08-29) — **no public REST admin API.** Web admin console only; SCIM is Enterprise/Edu-gated, not available on Business. Listed here as a documentation column. Internal spec at `connectors/ai/chatgpt-team/specs/PRD.md` claims per-user/day admin endpoints — these do not exist publicly. Connector dir name is stale (still `chatgpt-team/`).
- **GitHub Copilot** (`bronze_github_copilot.*`) — pending (PR #234 — PRD/DESIGN drafted, connector code not yet written). Targets the new signed-URL NDJSON metrics API (`/orgs/{org}/copilot/metrics/reports/{users-1-day,organization-1-day}`, plus 28-day variants with `/latest`). Old `/copilot/metrics` was decommissioned 2026-04-02. Signed-URL host changed once on 2026-02-26 and another change is upcoming per the 2026-04-22 changelog — implementations must not hardcode the host.
- **Windsurf** (`bronze_windsurf.*`) — *planned only*. API verified per [docs.windsurf.com](https://docs.windsurf.com/plugins/accounts/api-reference/api-introduction): 6 endpoints on `https://server.codeium.com/api/v1/` — 3 analytics (`/Analytics`, `/CascadeAnalytics`, `/UserPageAnalytics`) plus 3 config/billing (`/UsageConfig`, `/GetUsageConfig`, `/GetTeamCreditBalance`). Service-key auth (in request body), **Enterprise plan only**.
- **JetBrains AI** (`bronze_jetbrains_ai.*`) — *planned only*. API verified per JetBrains [IDE Services AI Analytics docs](https://www.jetbrains.com/help/ide-services/ai-analytics-api.html). Endpoints under IDE Services / Central Console: `GET /api/analytics/ai/users-adoption`, `/effectiveness/metrics`, `/effectiveness/metrics-by-feature`, `/effectiveness/metrics/totals`, `/effectiveness/metrics/users.paged`; user directory at `GET /api/v1/users`. Auth: `Authorization: Automation <token>` or OAuth 2.0. Granularity: per `(user, tool, date)` daily aggregates; tool taxonomy `tool ∈ {junie, aia, claude_code, code_completion, nes}` is built into the API. Authoritative response field names live in the **tenant-served Swagger** (`https://<your_org>/swagger-ui.html`) — public help pages name the metric concepts but not the JSON keys. Available to any AI Pro / Ultimate / All Products Pack customer running IDE Services or Central Console (no separate "AI Enterprise" SKU required, contrary to internal spec).

---

## Identity & metadata

| Metric | Cursor | Claude Admin | Claude Enterprise | OpenAI API | ChatGPT Ent. † | ChatGPT Business ‡ | GitHub Copilot * | Windsurf ** | JetBrains AI *** |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| User directory (email-keyed) | ✅ `cursor_members` | ✅ `claude_admin_users` | ✅ `claude_enterprise_users` | ✅ `users` | ✅ SCIM `/Users` | ❌ (no API) | ✅ `copilot_seats` (login + email) | ✅ implicit via `daily_usage.email` | ✅ `/api/v1/users` (`id`, `email`, `username`, `role`, `isActive`, `createdDate`) |
| API key inventory | ❌ | ✅ `claude_admin_api_keys` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Workspace structure | ❌ | ✅ `claude_admin_workspaces` + `*_members` (flat list) | ❌ | ❌ | 🟡 SCIM `/Groups` (workspace groups, not hierarchy) | ❌ | ❌ | ❌ | 🟡 IDE Services profiles (`profileId` filter) |
| User invites | ❌ | ✅ `claude_admin_invites` | ❌ | ❌ | 🟡 SCIM `POST /Users` provisioning events | ❌ | ❌ | ❌ | ❌ |
| Seat assignment (plan/billing tier) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ `copilot_seats.plan_type` | 🟡 implicit via `subscription_included_reqs` vs `usage_based_reqs` | ❌ |
| Audit logs | ✅ `cursor_audit_logs` | ❌ (Compliance API not integrated) | ❌ | ✅ Audit Logs API (per [help 9687866](https://help.openai.com/en/articles/9687866)) | ✅ Compliance Logs `admin_audit` + `user_authentication` events | ❌ | ❌ | ❌ | ❌ |

\* GitHub Copilot — *pending PR #234, OPEN*. All cells indicate what **will be available** after implementation.
\*\* Windsurf — *planned only*. No connector code yet; schema below derived from public API docs.
\*\*\* JetBrains AI — *planned only*. API endpoints public ([IDE Services AI Analytics](https://www.jetbrains.com/help/ide-services/ai-analytics-api.html)) but exact JSON field names require tenant-served Swagger to confirm. Cells reflect documented metric concepts, not verbatim wire-format names.
† ChatGPT Enterprise — *planned only*. Covers Enterprise + Edu SKUs (same surface). Compliance Logs Platform (JSONL streaming, 30-day retention) + SCIM 2.0. Stateful Compliance route deprecates 2026-06-05.
‡ ChatGPT Business — formerly "ChatGPT Team" (renamed 2025-08-29). **No public admin API.** Column included as documentation; all cells ❌ unless OpenAI ships an API.

## Code / IDE / CLI dev activity

| Metric | Cursor | Claude Admin | Claude Enterprise | OpenAI API | ChatGPT Ent. | ChatGPT Business | GitHub Copilot | Windsurf | JetBrains AI |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| AI-accepted lines added | ✅ `acceptedLinesAdded` | ✅ `lines_added` | ✅ `code_lines_added` | 🟡 (see §)| ❌ | ❌ | ✅ `loc_added_sum` | ✅ Analytics:User Data `num_lines_accepted` + Cascade `linesAccepted` | ✅ "lines added" (concept) |
| AI-accepted lines removed | ✅ `acceptedLinesDeleted` | ✅ `lines_removed` | ✅ `code_lines_removed` | ❌ | ❌ | ❌ | ❌ | 🟡 Command Data `lines_removed` (per command) | ✅ "lines deleted" (concept) — also "lines modified" |
| Total lines / bytes (incl. manual keystrokes) | ✅ `totalLinesAdded` / `totalLinesDeleted` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ **PCW data `total_bytes` / `user_bytes`** (unique honest signal) | ❌ |
| `ai_loc_share` (accepted/total) computable | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ **PCW `percent_code_written` directly** | ❌ |
| Sessions per day | 🟡 `isActive` boolean only | ✅ `session_count` | ✅ `code_session_count` | 🟡 (heuristic via `model ILIKE '%codex%'`)| 🟡 derive from `codex_usage` events | ❌ | 🟡 (via `last_activity_at` heuristic) | 🟡 derive from per-day rows + `lastAutocompleteUsageTime` etc. | 🟡 "invocations" (concept; not session count) |
| Suggestions/completions accepted | ✅ `totalTabsAccepted` | ✅ `tool_use_accepted` | ✅ `code_tool_accepted_count` | ❌ | ❌ | ❌ | ✅ `code_acceptance_activity_count` | ✅ Analytics:User Data `num_acceptances` | ✅ "suggestions accepted" (concept) |
| Suggestions/completions offered | ✅ `totalTabsShown` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ (no «shown» counter in API) | ✅ "suggestions shown" (concept) |
| Tool invocations rejected | ❌ | ✅ `tool_use_rejected` | ✅ `code_tool_rejected_count` | ❌ | ❌ | ❌ | ❌ | ❌ (only accepted) | ✅ "suggestions rejected" / "lines rejected" (concept) |
| Tool acceptance rate | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ (no offered counter) | ❌ (no shown counter) | ✅ "AI code acceptance rate" (dashboard metric) |
| Agent / multi-step sessions | ✅ `agentRequests` | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ `used_agent` (boolean) | ✅ Cascade Runs (filter `mode != 'CONVERSATIONAL_PLANNER_MODE_NO_TOOL'`) | 🟡 via `tool='junie'` filter |
| Agent tool actions breakdown | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ Cascade Tool Usage breakdown | ❌ |
| Chat usage flag | ❌ | ❌ | ❌ | ❌ | ✅ Compliance Logs `conversations` events (raw, aggregate consumer-side) | ❌ | ✅ `used_chat` (boolean) | ✅ via Chat Data presence | ✅ via `tool='aia'` filter |
| CLI usage flag | ❌ | ❌ | ❌ | ❌ | ✅ Compliance Logs `codex_usage` events | ❌ | ✅ `used_cli` (boolean) | 🟡 `ide_types` filter includes 'cli' | ❌ |
| Editor breakdown (vscode/jetbrains/cli) | ✅ `tabMostUsedExtension` | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ `last_activity_editor` | ✅ Analytics `ide_types ∈ {editor, jetbrains, cli}` + dimension | ❌ in public API (spec speculates `ide_product_code`) |
| Most used model (per day) | ✅ `mostUsedModel` | ❌ | ❌ | implicit `model` per row | 🟡 derivable from event `model` field if present in Compliance Logs | ❌ | ❌ | ✅ Cascade Runs `model` | ❌ (no per-user model attribution) |
| Commits attributed to AI | ❌ | ✅ `core_metrics.commits_by_claude_code` | ✅ `code_commit_count` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| PRs attributed to AI | ❌ | ✅ `core_metrics.pull_requests_by_claude_code` | ✅ `code_pull_request_count` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Subscription tier / billing | ✅ `subscriptionIncludedReqs` / `usageBasedReqs` | ❌ | ❌ | ✅ `service_tier` | ❌ (flat-seat per-workspace) | ❌ (flat-seat) | ✅ via `plan_type` | ✅ UserPageAnalytics `promptCreditsUsed` (cents) | ❌ |
| Active-days / last-activity per modality | ❌ | ❌ | ❌ | ❌ | 🟡 derive from event timestamps | ❌ | ✅ `last_activity_at` (one) | ✅ 3 separate timestamps in UserPageAnalytics | 🟡 `users-adoption` rollup |
| Per-tool sub-categorization | (Cursor IDE-internal) | ✅ per terminal_type | ✅ `claude_code_metrics_json` | ❌ | ✅ Compliance Logs event categories (`conversations` / `codex_usage` / `app_calls` etc.) | ❌ | ❌ | ✅ Command Data `command_source` (8 sources), Cascade `mode` | ✅ **`tool` enum is built-in axis**: `{junie, aia, claude_code, code_completion, nes}` |

§ OpenAI API: `usage_completions` exposes `input_tokens` / `output_tokens`, not lines. "Lines" is a Codex-CLI concept; not visible in this stream. Use Compliance Logs `codex_usage` events (ChatGPT Enterprise) when available.

§§ OpenAI API: Codex CLI activity within `usage_completions` is identifiable only via `model ILIKE '%codex%'` heuristic (models `gpt-5-codex`, `gpt-5.3-codex`). Not exact — not every Codex request necessarily uses a codex-specific model. Compliance Logs Platform `codex_usage` events (ChatGPT Enterprise surface) are the authoritative source when available.

GitHub Copilot also exposes an **org-level** rollup (`organization-1-day` / `organization-28-day` reports) in addition to per-user — unique to Copilot. ⚠️ The legacy `/copilot/metrics` endpoint exposed `total_active_user_count` / `total_engaged_user_count` / `total_code_acceptance_activity_count` directly; the **new NDJSON reports API does NOT carry those `total_*` fields** — active/engaged user counts must be derived by aggregating the per-user `users-1-day` rows. Planned bronze table `copilot_org_metrics` will hold the derived rollup:

| Org metric | Source |
|---|---|
| `total_active_users` | derived: `COUNT(DISTINCT user_id) WHERE user_initiated_interaction_count > 0` from `users-1-day` |
| `total_engaged_users` | derived: substantive-activity threshold over `users-1-day` (GitHub's threshold is internal — pick our own or skip) |
| `total_code_acceptances_org` | derived: `SUM(code_acceptance_activity_count)` from `users-1-day` |

This feeds the **planned** `class_ai_org_usage` (see below).

## Chat / Assistant activity (Claude.ai web/desktop, ChatGPT app — consumer-facing chat products)

| Metric | Cursor | Claude Admin | Claude Enterprise | OpenAI API | ChatGPT Ent. | ChatGPT Business | GitHub Copilot | Windsurf | JetBrains AI |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| Chat messages | ❌ | ❌ | ✅ `chat_message_count` | ❌ | 🟡 derive from Compliance Logs `conversations` events (consumer-side aggregation) | ❌ | ❌ | ❌ ‡ | ❌ § |
| Chat conversations | ❌ | ❌ | ✅ `chat_conversation_count` | ❌ | 🟡 same — `COUNT(DISTINCT conversation_id)` on events | ❌ | ❌ | ❌ ‡ | ❌ § |
| Files uploaded in chat | ❌ | ❌ | ✅ `chat_files_uploaded_count` | ❌ | ✅ Compliance Logs `uploaded_files` events | ❌ | ❌ | ❌ | ❌ |
| Skills used | ❌ | ❌ | ✅ `chat_skills_used_count` | ❌ | 🟡 GPT/skill invocations in `app_calls` / `workspace_gpts` events | ❌ | ❌ | ❌ | ❌ |
| Connectors used | ❌ | ❌ | ✅ `chat_connectors_used_count` | ❌ | 🟡 same — derive from `app_calls` events | ❌ | ❌ | ❌ | ❌ |
| Projects created / used | ❌ | ❌ | ✅ `chat_projects_created_count` / `..._used_count` | ❌ | ❌ (no documented Projects events) | ❌ | ❌ | ❌ | ❌ |
| Artifacts created | ❌ | ❌ | ✅ `chat_artifacts_created_count` | ❌ | ❌ (Anthropic-specific concept) | ❌ | ❌ | ❌ | ❌ |
| "Thinking" turns (extended reasoning) | ❌ | ❌ | ✅ `chat_thinking_message_count` | ❌ | 🟡 raw event payload may carry reasoning_tokens for o1/o3 — derive | ❌ | ❌ | ❌ | ❌ |
| Skills / connectors / projects directories | ❌ | ❌ | ✅ `claude_enterprise_skills` / `..._connectors` / `..._chat_projects` | ❌ | 🟡 `workspace_gpts` event metadata | ❌ | ❌ | ❌ | ❌ |

(Removed stale claim — ChatGPT app activity does **not** appear in `usage_completions`. ChatGPT and platform.openai.com are separate billing/identity domains. ChatGPT chat usage lives in the Compliance Logs `conversations` event stream — Enterprise/Edu only.)

‡ Windsurf: the «chat» exposed by Windsurf API is **IDE-level Cascade chat** (in-editor), not a consumer-facing chat product. Metrics for that IDE-chat (intent type, `chat_loc_used`, etc.) are already in the Code/IDE table above. Standalone chat product **does not exist** for Windsurf.

§ JetBrains AI: in-IDE chat is covered in the Code/IDE table via `tool='aia'`. No consumer-facing chat product.

## Office / desktop integrations

| Metric | Cursor | Claude Admin | Claude Enterprise | OpenAI API | ChatGPT Ent. | ChatGPT Business | GitHub Copilot | Windsurf | JetBrains AI |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| Excel sessions | ❌ | ❌ | ✅ `excel_session_count` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Excel messages | ❌ | ❌ | ✅ `excel_message_count` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| PowerPoint sessions | ❌ | ❌ | ✅ `powerpoint_session_count` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| PowerPoint messages | ❌ | ❌ | ✅ `powerpoint_message_count` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Cowork (desktop assistant) sessions | ❌ | ❌ | ✅ `cowork_session_count` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Cowork messages / actions | ❌ | ❌ | ✅ `cowork_message_count` / `cowork_action_count` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Cowork dispatch turns | ❌ | ❌ | ✅ `cowork_dispatch_turn_count` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Web search (cross-surface chat+code) | ❌ | ❌ | ✅ `web_search_count` | ❌ | 🟡 derive from Compliance Logs `conversations` event tool-use payload | ❌ | ❌ | ❌ | ❌ |

> This domain is **only** covered by Claude Enterprise. Cursor / Copilot / Windsurf are IDE-only — no Office integration. OpenAI / Claude Admin are API-tier, not consumer products.

## API tokens / billing

| Metric | Cursor | Claude Admin | Claude Enterprise | OpenAI API | ChatGPT Ent. | ChatGPT Business | GitHub Copilot | Windsurf | JetBrains AI |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| Input tokens | 🟡 `cursor_usage_events.tokenUsage` (per-event JSON) | ✅ `claude_admin_messages_usage` | ❌ | ✅ `usage_completions.input_tokens` | ❌ (flat-seat — tokens not exposed) | ❌ | ❌ | ❌ | ❌ |
| Output tokens | 🟡 same | ✅ | ❌ | ✅ `output_tokens` | ❌ | ❌ | ❌ | ❌ | ❌ |
| Cached input tokens | 🟡 `tokenUsage.cacheReadTokens` / `cacheWriteTokens` | ✅ `cache_read_input_tokens` + `cache_creation.ephemeral_{5m,1h}_input_tokens` | ❌ | ✅ `input_cached_tokens` | ❌ | ❌ | ❌ | ❌ | ❌ |
| Audio tokens | ❌ | ❌ | ❌ | ✅ `input_audio_tokens` / `output_audio_tokens` | ❌ | ❌ | ❌ | ❌ | ❌ |
| Per-model breakdown | 🟡 `mostUsedModel` (daily) + per-event `model` | ✅ | ❌ | ✅ `model` | 🟡 model in event payload (Compliance Logs) | ❌ | ❌ | ✅ Cascade Runs `model` | ❌ |
| Per-project breakdown | ❌ | ❌ | ❌ | ✅ `project_id` | 🟡 SCIM `/Groups` | ❌ | ❌ | 🟡 `group_name` (team groups) | 🟡 `profileId` filter |
| Per-API-key breakdown | ❌ | ✅ (via `actor_type='api_actor'`) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ (service_keys are admin-level only) | ❌ |
| Service tier (default/scale) | ✅ `subscriptionIncludedReqs` / `usageBasedReqs` | ❌ | ❌ | ✅ `service_tier` | ❌ (single tier per workspace) | ❌ | ✅ via `plan_type` | ❌ (has `promptCreditsUsed` per cycle) | ❌ |
| Batch flag | ❌ | ❌ | ❌ | ✅ `batch` | ❌ | ❌ | ❌ | ❌ | ❌ |
| Cost ($, per line item) | ✅ `cursor_usage_events.chargedCents` (per-event) | ✅ `claude_admin_cost_report` | ❌ | ✅ `costs` | ❌ (flat per-seat, not exposed via API) | ❌ | ❌ | ✅ Cascade Runs `promptsUsed` (cents); UserPageAnalytics `promptCreditsUsed` per user | ❌ |
| Credit / usage-based billing flag | ✅ `isChargeable`, `isFreeBugbot`, `isTokenBasedCall` | ❌ | ❌ | ✅ `is_batch` | ❌ | ❌ | ❌ | 🟡 implicit (prompts billed in credits) | ❌ |

## Specialized API surfaces (model API only)

| Metric | Cursor | Claude Admin | Claude Enterprise | OpenAI API | ChatGPT Ent. | ChatGPT Business | GitHub Copilot | Windsurf | JetBrains AI |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| Embeddings API usage | ❌ | ❌ | ❌ | ✅ `usage_embeddings` | ❌ | ❌ | ❌ | ❌ | ❌ |
| Moderations API usage | ❌ | ❌ | ❌ | ✅ `usage_moderations` | ❌ | ❌ | ❌ | ❌ | ❌ |
| Image generation (DALL-E) | ❌ | ❌ | ❌ | ✅ `usage_images` | ❌ | ❌ | ❌ | ❌ | ❌ |
| Audio TTS (`speeches`) | ❌ | ❌ | ❌ | ✅ `usage_audio_speeches` | ❌ | ❌ | ❌ | ❌ | ❌ |
| Audio Speech-to-Text (Whisper) | ❌ | ❌ | ❌ | ✅ `usage_audio_transcriptions` | ❌ | ❌ | ❌ | ❌ | ❌ |
| Vector store usage | ❌ | ❌ | ❌ | ✅ `usage_vector_stores` | ❌ | ❌ | ❌ | ❌ | ❌ |
| Code Interpreter (Assistants tool) | ❌ | ❌ | ❌ | ✅ `usage_code_interpreter` | ❌ | ❌ | ❌ | ❌ | ❌ |

> This domain is **OpenAI-API-only**. Anthropic does not expose separate modality streams (everything goes through the unified Messages API). ChatGPT Enterprise/Business operate at the chat-product layer — modality is internal, not exposed as separate streams. Cursor / Copilot / Windsurf / JetBrains AI are IDE/dev tools, single-modality, no specialized surface streams.

## Local data availability (in current dump)

| | Cursor | Claude Admin | Claude Enterprise | OpenAI API | ChatGPT Ent. | ChatGPT Business | GitHub Copilot | Windsurf | JetBrains AI |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| `bronze_*` schema in CH | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Connector running in prod at dump time | ✅ | ⚠️ pending | ✅ | ⚠️ pending | ⚠️ planned only (PRD has phantom endpoints — needs rewrite against Compliance Logs JSONL) | ❌ no public API | ⚠️ PRD only (PR #234) | ⚠️ planned only (no PR) | ⚠️ spec only (spec has known divergences) |

---

## Provider specialization summary

- **Cursor** — most detailed for **dev/code** activity (in-IDE only). Unique features: total typed lines (for `ai_loc_share`), agent sessions.
- **Claude Enterprise** — broadest coverage: code + chat + cowork + office + web search. Does not expose API tokens / cost. Unique features: web_search, Office integration, projects, artifacts, thinking. Code surface overlaps with Claude Admin's Claude Code Analytics — same commits/PRs/per-tool breakdown.
- **Claude Admin** — API-focused: tokens, cost, identity, workspaces (flat list, not hierarchical). Code activity via the Claude Code Analytics endpoint covers sessions, lines, commits/PRs, and per-tool accept/reject. Unique: per-API-key attribution, flat workspace inventory.
- **OpenAI API** — broad modality coverage (audio/images/embeddings/moderations/audio/vector_stores/code_interpreter) at the platform.openai.com Admin Usage API. Pay-per-token domain. Codex CLI activity within `usage_completions` is identifiable only via `model ILIKE '%codex%'` heuristic. Unique features: per-project, service_tier, cost report.
- **ChatGPT Enterprise** *(planned)* — covers Enterprise + Edu SKUs (single surface). Compliance Logs Platform JSONL streams (`conversations`, `uploaded_files`, `workspace_gpts`, `memories`, `workspace_users`, `admin_audit`, `user_authentication`, `codex_usage`, `app_calls`) + SCIM 2.0 for seat roster. 30-day retention, minute-level latency. Workspace Analytics is dashboard-only (no API). Stateful Compliance route deprecates 2026-06-05.
- **ChatGPT Business** — formerly "ChatGPT Team" (renamed 2025-08-29). **No public admin API** for seats or activity — admin console only. Connector dir name (`chatgpt-team/`) is stale; PRD references endpoints that do not exist.
- **GitHub Copilot** *(pending)* — per-user + org-level (unique). Includes activity flags (`used_chat`, `used_agent`, `used_cli`). Identity via login → email.
- **Windsurf** *(planned)* — second source after Cursor with honest `ai_loc_share` signal (PCW data). Rich Cascade tool breakdown. Per-modality last-activity timestamps.

## Coverage by current and proposed silver classes

| Silver class | Cursor | Claude Admin | Claude Enterprise | OpenAI API | ChatGPT Ent. | ChatGPT Business | GitHub Copilot | Windsurf | JetBrains AI |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| `class_ai_dev_usage` (existing) | ✅ feeds | ✅ feeds (broken locally) | ⚠️ only after local fix; upstream does NOT feed | 🟡 codex heuristic (NEW) | 🟡 `codex_usage` events (planned) | ❌ no API | 🟡 planned (PR #234) | 🟡 planned | 🟡 planned |
| `class_ai_api_usage` (existing) | ❌ N/A | ✅ feeds | ✅ feeds | ❌ tagged as `openai`, not silver-class | ❌ N/A (flat-seat) | ❌ no API | ❌ N/A | ❌ N/A | ❌ N/A |
| `class_ai_assistant_usage` (proposed, NEW) | ❌ N/A | ❌ N/A | ✅ feeds (chat/cowork/office/web_search) | ❌ N/A | ✅ feeds (planned — Compliance Logs `conversations` events) | ❌ no API | ❌ N/A | 🟡 chat surface | ❌ N/A |
| `class_ai_cost` (proposed, NEW) | ❌ | ✅ feeds | ❌ | ✅ feeds | ❌ (flat-seat, not exposed) | ❌ | ❌ | ✅ `promptCreditsUsed` | ❌ |

## Known gaps (todo backlog)

1. **Claude Enterprise → `class_ai_dev_usage`** — staging written locally, needs upstream PR.
2. **Claude Admin → `class_ai_dev_usage`** — staging exists but bronze database missing locally (PR #237 fix unverified locally).
3. **OpenAI staging → `class_ai_api_usage`** — fix tag in `to_ai_tool_usage.sql`.
4. **OpenAI Codex sub-stream** — heuristic via `model LIKE '%codex%'` to feed `class_ai_dev_usage`.
5. **OpenAI extra streams** — embeddings/moderations/audio/images not in silver at all. `vector_stores` and `code_interpreter_sessions` carry only one metric each (`usage_bytes`, `num_sessions` — no per-call/per-file counts), so lower priority.
6. **`class_ai_assistant_usage`** — table doesn't exist, no dbt model.
7. **`class_ai_cost`** — table doesn't exist.
8. **OpenAI Compliance Logs Platform** — separate connector for accurate ChatGPT vs Codex vs API break-down not integrated. Codex Usage logs ship as a dedicated stream in this endpoint (GA late 2025) — once integrated, replaces the `model ILIKE '%codex%'` heuristic.
9. **Claude Admin Compliance API** — same (still on Anthropic's roadmap).
10. **ChatGPT Team product analytics** — not integrated (if a separate admin endpoint exists at all).

---

# Silver-layer split proposal

Goal: each class is one **product domain** with a homogeneous schema. No NULL-padded columns from sources that "don't expose this kind of data". When a provider grows (Anthropic adds Compliance API, OpenAI splits ChatGPT/Codex, etc.) — rows get added to existing classes or a new class appears, but **existing class schemas don't get smeared**.

## Proposed structure (5 core + 2 optional classes)

```
silver/
├── ai/
│   ├── class_ai_dev_usage         (per-person-day code activity)
│   ├── class_ai_assistant_usage   (per-person-day chat/cowork/office/web)
│   ├── class_ai_api_usage         (per-key-or-project-day tokens + multi-modal API)
│   ├── class_ai_cost              (per-line-item-day financial)
│   └── class_ai_audit_log         (event-stream — admin actions, compliance)
└── _shared/
    ├── class_people               (existing) — identity unification
    └── class_ai_directories?      (optional — skills, connectors, projects metadata)
```

## Per-class schemas and provider feeding

### 1. `class_ai_dev_usage` (extending existing)

**Grain:** `(insight_tenant_id, email, day, tool)` — one row per person × day × tool

**Schema:** [already exists — see `silver/ai/schema.yml`]
```
email, api_key_id, day, tool, source, data_source
session_count, lines_added, lines_removed, total_lines_added, total_lines_removed
tool_use_offered, tool_use_accepted, completions_count
agent_sessions, chat_requests, cost_cents, collected_at
```

**Provider feeding:**

| Provider | Source | tool value |
|---|---|---|
| Cursor | `cursor__ai_dev_usage` | `'cursor'` |
| Claude Admin | `claude_admin__ai_dev_usage` | `'claude_code'` (actor_type ∈ user/api_actor) |
| Claude Enterprise | `claude_enterprise__ai_dev_usage` (NEW — my local fix) | `'claude_code'` |
| OpenAI | `openai__ai_dev_usage` (NEW — heuristic `model LIKE '%codex%'`) | `'codex'` |

**Schema extension:** add `commits_count`, `pull_requests_count` (both Claude Admin via Code Analytics API **and** Claude Enterprise expose them; Cursor/OpenAI = NULL).

### 2. `class_ai_assistant_usage` (NEW)

**Grain:** `(insight_tenant_id, email, day, tool, surface)`

**Schema:**
```
insight_tenant_id, source_id, unique_key
email, day
tool                ('claude' | 'chatgpt' | 'gemini' future)
surface             ('chat' | 'cowork' | 'excel' | 'powerpoint' | 'cross')

-- common:
session_count, message_count, action_count

-- chat-specific (NULL for non-chat surfaces):
conversation_count, files_uploaded_count, artifacts_created_count
projects_created_count, projects_used_count, skills_used_count
connectors_used_count, thinking_message_count

-- cross-surface (web_search, etc.) — written with surface='cross':
search_count

-- common metadata:
cost_cents, source, data_source, collected_at
```

**Provider feeding:**

| Provider | Source | What it provides |
|---|---|---|
| Cursor | ❌ | N/A |
| Claude Admin | ❌ | N/A (Admin API doesn't cover chat) |
| Claude Enterprise | `claude_enterprise__ai_assistant_usage` (NEW) | rows for surface ∈ chat / cowork / excel / powerpoint / cross |
| OpenAI | `openai__ai_assistant_usage` (FUTURE) | requires Compliance API connector — without it, ChatGPT app activity is not separable |

**Web search nuance:** `web_search_count` arrives as a single counter cross-surface (chat + code). Recorded as a row with `surface='cross'`, `tool='claude'`, other counts NULL.

### 3. `class_ai_api_usage` (extending existing)

**Grain:** `(insight_tenant_id, day, model, [api_key_id | project_id | user_id], surface)`

**Schema extension:** existing schema (input_tokens, output_tokens, cached_tokens) + extension for OpenAI multi-modal:
```
day, model, api_key_id, project_id, user_id, service_tier, is_batch
surface             ('messages' | 'embeddings' | 'moderations' | 'images'
                     | 'audio_speech' | 'audio_transcription' | 'vector_stores'
                     | 'code_interpreter')
input_tokens, output_tokens, input_cached_tokens, input_audio_tokens, output_audio_tokens
num_model_requests, num_audio_seconds, num_images, num_embeddings_calls
cost_cents
source, data_source, collected_at
```

**Provider feeding:**

| Provider | Source | surface values |
|---|---|---|
| Cursor | ❌ | N/A |
| Claude Admin | `claude_admin__ai_api_usage` | `'messages'` |
| Claude Enterprise | ❌ | N/A (does not expose per-token) |
| OpenAI | `openai__ai_api_usage` (NEW — extend existing `to_ai_tool_usage`) | all 8 surfaces |

**Re-tagging:** `to_ai_tool_usage.sql` is currently tagged `['openai']`, should be `['openai', 'silver:class_ai_api_usage']` + extend the SELECT for all surfaces (currently only completions).

### 4. `class_ai_cost` (NEW, separate from api_usage)

**Grain:** `(insight_tenant_id, day, line_item, [api_key_id | project_id])`

**Why separate from api_usage:** cost rows sometimes don't bind to a specific (model, user) combination — e.g. Anthropic charges for Claude Code subscription seats, OpenAI charges for provisioned throughput. These are billing line items not reducible to per-token level.

**Schema:**
```
day, line_item, api_key_id, project_id
amount_value, amount_currency
provider, source, data_source, collected_at
```

**Provider feeding:**

| Provider | Source |
|---|---|
| Cursor | ❌ |
| Claude Admin | `claude_admin__ai_cost` (existing `cost_report` stream — needs new staging tagged `silver:class_ai_cost`) |
| Claude Enterprise | ❌ (no cost data) |
| OpenAI | `openai__ai_cost` (existing `to_ai_cost.sql` — re-tag `silver:class_ai_cost`) |

### 5. `class_ai_audit_log` (NEW)

**Grain:** event-stream, `(event_id, event_at)`

**Schema:**
```
event_id, event_at
actor_email, actor_id
event_kind         ('user_invited', 'role_changed', 'api_key_created',
                    'workspace_created', 'compliance_query', etc.)
target_kind        ('user', 'api_key', 'workspace', 'project', 'session')
target_id
metadata_json
provider, source, data_source, collected_at
```

**Provider feeding:**

| Provider | Source |
|---|---|
| Cursor | `cursor__ai_audit_log` (NEW — `cursor_audit_logs` stream) |
| Claude Admin | `claude_admin__ai_audit_log` (FUTURE — Compliance API not yet integrated) |
| Claude Enterprise | ❌ |
| OpenAI | `openai__ai_audit_log` (FUTURE — Compliance Logs Platform endpoint) |

### Optional 6. `class_ai_directories`

For static catalogs — skills, connectors, chat projects (metadata, not activity).

**Provider feeding:**
- Claude Enterprise: `claude_enterprise_skills`, `claude_enterprise_connectors`, `claude_enterprise_chat_projects`

Useful for answering "what projects did people work on", "which connectors are popular".

### Optional 7. `class_ai_workspaces`

For admin structure — workspace hierarchy, role assignments.

**Provider feeding:**
- Claude Admin: `claude_admin_workspaces`, `claude_admin_workspace_members`

Useful for multi-workspace organizations (typical at the Anthropic API tier).

---

## Coverage matrix of the proposed split

> Note: this is the original 4-provider proposal scope (pre-2026-04-28). For full coverage including Copilot, Windsurf, JetBrains AI, ChatGPT Enterprise, and ChatGPT Business, see "Extended coverage matrix of proposed silver classes" at the bottom of this document.

| Silver class | Cursor | Claude Admin | Claude Enterprise | OpenAI API |
|---|:-:|:-:|:-:|:-:|
| `class_ai_dev_usage` | ✅ existing | ✅ existing | 🟡 local fix | 🟡 codex heuristic (NEW) |
| `class_ai_assistant_usage` | ❌ | ❌ | 🟡 NEW staging | ❌ (different domain) |
| `class_ai_api_usage` | ❌ | ✅ existing | ❌ | 🟡 retag + extend |
| `class_ai_cost` | ❌ | 🟡 NEW staging | ❌ | 🟡 retag |
| `class_ai_audit_log` | 🟡 NEW staging | 🟡 needs Compliance API | ❌ | ✅ Audit Logs API (existing, untagged) |
| `class_ai_directories` (opt.) | ❌ | ❌ | 🟡 NEW staging | ❌ |
| `class_ai_workspaces` (opt.) | ❌ | 🟡 NEW staging | ❌ | ❌ |

---

## Migration path (if going all the way)

**Phase 1 — Quick wins (doable now):**
1. `class_ai_dev_usage`: add `claude_enterprise__ai_dev_usage` staging (my local fix → upstream PR)
2. `class_ai_dev_usage`: add `commits_count`, `pull_requests_count`, `tool_action_breakdown_json` columns + Claude Enterprise **and** Claude Admin (Claude Code Analytics) feeds for them
3. `class_ai_api_usage`: re-tag OpenAI staging + extend for all surfaces
4. `class_ai_cost`: create silver class + re-tag existing `to_ai_cost` + create `claude_admin__ai_cost` staging
5. Gold view fix: `ai_bullet_rows` filter cursor_/cc_ by tool (my local fix-up)

This delivers real data for all existing FE metrics + correct attribution.

**Phase 2 — Architectural expansion:**
1. `class_ai_assistant_usage`: create silver class + Claude Enterprise staging
2. New gold view branches for `cc_chat_messages`, `cc_web_searches`, `cc_cowork_*`, etc.
3. FE config: new bullet entries
4. Catalog: new rows

**Phase 3 — Advanced:**
1. `class_ai_audit_log`: Cursor staging (uses `cursor_audit_logs` already in bronze)
2. OpenAI Codex heuristic feed for `class_ai_dev_usage` (model LIKE '%codex%') — temporary, until Compliance Logs Platform connector lands
3. OpenAI multi-modal surfaces in `class_ai_api_usage` (audio, images, embeddings, moderations) — vector_stores / code_interpreter optional (single-metric streams)

**Phase 4 — Long-term (waiting on vendor):**
1. Anthropic Compliance API connector → audit log
2. OpenAI Compliance Logs Platform connector → **replaces** the `model ILIKE '%codex%'` heuristic with first-class Codex Usage logs; also gives Admin Audit + User Authentication streams
3. ChatGPT Team admin endpoint (if/when it appears)

---

## Cross-vendor source resolution

Multiple sources can describe the same person × day × tool from different vantage points. The same `tool='claude_code'` row may arrive from Claude Admin (token-level, all clients), Claude Enterprise (user-level, no api_actor), and JetBrains AI (only when used through a JetBrains IDE plugin). Same applies to OpenAI/Codex potentially leaking through JetBrains as `tool='code_completion'` / `tool='aia'`.

Rather than baking merge priorities into silver, **silver stores per-source rows verbatim; gold provides standardized views with documented column-level rules**.

### Silver grain

```
(insight_tenant_id, email, day, tool, provider_org_id, source)
```

- `source ∈ {cursor, claude_admin, claude_enterprise, openai_api, chatgpt_enterprise, github_copilot, windsurf, jetbrains_ai}` — names which connector wrote the row. ChatGPT Business has no admin API, so no source value reserved for it.
- `provider_org_id` — vendor-side organization UUID (Anthropic org id, GitHub org login, JetBrains tenant id, etc.). Distinguishes the same vendor's multiple org accounts under one Insight tenant.
- One silver row per source: each row carries **only** the metrics that source actually reports; everything else is NULL. No COALESCE between sources, no cross-source priority. Adding a new source = adding new rows, not revising existing ones.
- Three `tool='claude_code'` rows for the same person/day (Admin + Enterprise + JetBrains) are not duplicates — they are three independent observations.

### Configurations

The grain handles all configurations the same way:

1. **Single source** (e.g. only Cursor) — one silver row per person/day; trivial.
2. **One vendor, two sources, one org** (e.g. Claude Admin + Enterprise on the same Anthropic org) — two silver rows; same `(email, day, tool)`, different `source`.
3. **One vendor, multiple orgs** (e.g. several Anthropic Team orgs + an Enterprise org under one Insight tenant) — one row per `(source, provider_org_id)`. Different orgs = different rows, never collapse.
4. **Cross-vendor overlap on same `tool`** (e.g. `tool='claude_code'` from Claude Admin AND from JetBrains because the user runs Claude through the JB plugin) — different `source` values, different rows. Treated identically to (2) at silver.

Worst-case: a person with the same email in 3 Anthropic orgs + JetBrains tenant who actually uses Claude Code through JB → up to 5 silver rows for one day (3 Admin + 1 Enterprise + 1 JB). All correct.

### Gold view contracts

Three standard gold views consume silver. Each is **explicitly documented** in its SQL header — naive `SUM(...)` against silver is a bug.

#### 1. `v_ai_usage_per_provider`

Pass-through silver, no aggregation. One row per `(email, day, tool, provider_org_id, source)`. Use for "how much does each connector report" dashboards and DQ comparison. Filter by `source` for product-specific views.

#### 2. `v_ai_usage_unified`

One row per `(email, day, tool, provider_org_id)` — collapses across `source` with column-level rules. Each column has a documented priority list and merge mode:

| Column | Priority list (first non-null wins, unless noted) | Notes |
|---|---|---|
| `lines_added`, `lines_removed`, `total_lines_added`, `total_lines_removed` | Cursor → JetBrains AI → Claude Admin → Claude Enterprise → Windsurf → Copilot | IDE/CLI tools see editor state; API-tier sources don't see lines |
| `suggestions_offered`, `suggestions_accepted`, `suggestions_rejected` | Cursor → JetBrains AI → Copilot → Windsurf → Claude Admin → Claude Enterprise | IDE-side counts; only IDE-tier sources observe «shown» |
| `session_count`, `commits_count`, `pull_requests_count`, `tool_action_breakdown_json` | Claude Admin → Claude Enterprise → JetBrains AI → Cursor | Anthropic Code Analytics has the widest actor scope; JB sees only JB-driven sessions |
| `input_tokens`, `output_tokens`, `cache_*` | Claude Admin → OpenAI → Cursor (per-event JSON) | API-tier metering only. JetBrains, Copilot, Windsurf don't expose tokens |
| `cost_cents` | Claude Admin (`cost_report`) → OpenAI (`costs`) → Cursor (`chargedCents`) → Windsurf (`promptCreditsUsed`) | Per-line-item billing; never derive from token counts |
| `chat_*`, `cowork_*`, `excel_*`, `powerpoint_*`, `web_search_count` | Claude Enterprise (only) | No fallback — sole source |
| `ide_breakdown` (product/version) | JetBrains AI → Cursor (`tabMostUsedExtension`) → Copilot (`last_activity_editor`) | Editor-context dimension |
| `data_source` (audit tag) | computed | comma-joined list of contributing `source` values, e.g. `claude_admin,jetbrains_ai` |

Priority lists must be reflected verbatim as a SQL comment at the top of the view — drift between this doc and the view definition is a bug.

#### 3. `v_ai_users_dau`

`COUNT(DISTINCT email) GROUP BY day, tool` (or `tool, source` for per-product DAU). Never sum metrics here — DAU is a person-distinct count, not an activity sum. Avoids accidental double-count from the multi-source rows below it.

### Data-quality checks

For columns where multiple sources report the same concept (e.g. `lines_added` from Cursor and JB AI for the same user/day, or `session_count` from Admin and Enterprise on the same Anthropic org):
- If both sources non-NULL and diverge by >5%, fire a DQ alert. Priority is a default for the unified view, not a cover-up for connector bugs.
- The `v_ai_usage_per_provider` view exposes the raw per-source values for debugging.

### Cross-vendor caveat: vantage-point overlap

JetBrains AI's `tool='claude_code'` row and Claude Admin's `tool='claude_code'` row often describe the **same actual coding session** seen from two angles: JB sees IDE-level events (suggestions, lines), Anthropic sees API calls (tokens, cost). At silver they are independent rows; at gold (`v_ai_usage_unified`) they merge by COALESCE on **complementary** columns — JB fills `lines_*`/`suggestions_*`, Anthropic fills `tokens`/`cost`/`session_count`/`commits`. Neither overrides the other on its native fields.

The same applies to JB's `code_completion`/`aia`/`junie` tools when configured with bring-your-own LLM keys: those calls also light up Claude Admin or OpenAI usage. Currently silver does not attempt to attribute JB rows back to the underlying LLM provider (JB's API doesn't expose it), so the rows live independently as `tool='aia'` (from JB) and `tool='messages'`/`tool='codex'` (from Anthropic/OpenAI). For person/org rollups this is acceptable; for token-cost-per-IDE attribution it is not solvable without JB exposing the upstream model.

---

## Open architectural questions

1. **`class_ai_cost` separate or rolled into `class_ai_api_usage`?** — Arguments for separate: cost rows have grain `(line_item, project, day)`, not `(model, user, day)`; mixing would pollute schema. Arguments for rolled-in: fewer classes, simpler joins. **I'd keep them separate.**

2. **`web_search_count` placement** — separate class or surface='cross' in assistant_usage? **I'd go with the latter** — web_search is semantically assistant-related (cross-product), not its own domain.

3. **Identity surface (`class_people`) vs new `class_ai_workspaces`** — Anthropic workspace hierarchy is per-tenant admin structure, not global identity. Can be kept separate from `class_people` and joined via `api_key_id → person_id` chain in Identity Resolution.

4. **OpenAI multi-modal surfaces in one `class_ai_api_usage`** or separate (e.g., `class_ai_image_usage`, `class_ai_audio_usage`)? — **Single class with `surface` ENUM**. Each modality has different core counts (tokens / seconds / image count) — schema will have conditional NULL columns per-surface. Note that `vector_stores` carries only `usage_bytes` and `code_interpreter_sessions` only `num_sessions` — both single-metric, so most surface-specific columns are NULL for them. Alternative — per-modality classes — gives clean schemas but N classes for N modalities. I'd go with union + surface enum for practicality.

5. ~~**`tool='claude_code'` for Admin vs Enterprise rows**~~ — *resolved by silver-per-source design*: silver keeps Admin and Enterprise as separate rows discriminated by `source`, no merge at silver. Gold view `v_ai_usage_unified` collapses them via documented column-level priority (see Cross-vendor source resolution). No `claude_code:admin` / `claude_code:enterprise` tool-value split needed — `source` already discriminates.

6. **Cost in `class_ai_dev_usage` or `class_ai_cost`?** — Cursor/Admin staging have a `cost_cents` column in the current dev_usage schema. That's per-user cost (when billing is per-seat / per-token). Could **duplicate** for gold-query convenience (avoid join), but creates a perpetual sync risk on truth changes. **Cleaner: drop `cost_cents` from dev_usage**, keep only in the cost class. Minor migration.

---

## Final map (target state after Phase 1-3)

```
                     ┌─────────────┐
        ┌────────────│  Cursor     │────────────┐
        │            └─────────────┘            │
        ▼                                       ▼
   ┌─────────┐                            ┌──────────┐
   │  dev    │                            │ audit    │
   │ usage   │                            │  log     │
   └─────────┘                            └──────────┘
        ▲                                       ▲
        │            ┌─────────────┐            │
        ├────────────│ Claude Admin│────────┐   │
        │            └─────────────┘        │   │
        │            ┌─────────────┐        ▼   │
        ├────────────│Claude Enter.│──┐  ┌─────────┐
        │            └─────────────┘  │  │ api     │
        │                             │  │ usage   │
        │            ┌─────────────┐  │  └─────────┘
        ├────────────│   OpenAI    │──┤        ▲
        │            └─────────────┘  │        │
        │                             │        │
        │                             ▼        │
        │      ┌────────────┐  ┌─────────┐    │
        └─────►│ assistant  │  │  cost   │◄───┘
               │  usage     │  └─────────┘
               └────────────┘
```

Each provider can feed ≥1 silver class. Each silver class can receive data from ≥1 provider. Clean domain separation, no "catch-all" class accumulating whatever didn't fit elsewhere.

---

# Exhaustive bronze→silver field-level mapping

Goal — capture **maximum data from every source** in silver. Each bronze field is either attributed to a specific silver class with a specific target column, or explicitly marked as "out of scope" with justification.

## Cursor

### `bronze_cursor.cursor_daily_usage` → `class_ai_dev_usage`

| Bronze field | Silver target | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `unique_key`, `email`, `userId` | identity columns + `unique_key` | base |
| `day` | `day` | dimension |
| `isActive` | filter only (`WHERE isActive=true`) | discard inactive rows |
| `acceptedLinesAdded` | `lines_added` | core |
| `acceptedLinesDeleted` | `lines_removed` | core |
| `totalLinesAdded` | `total_lines_added` | unique to Cursor (denominator for ai_loc_share) |
| `totalLinesDeleted` | `total_lines_removed` | unique to Cursor |
| `totalTabsShown` | `tool_use_offered` | core |
| `totalTabsAccepted` | `tool_use_accepted` + `completions_count` | core |
| `totalAccepts` / `totalApplies` / `totalRejects` | aggregate into `tool_use_*` (sum'd) | breakdown by action types |
| `agentRequests` | `agent_sessions` | core |
| `chatRequests` + `composerRequests` | `chat_requests` | core |
| `apiKeyReqs` | NEW column `api_request_count` | API-tier requests (when on pro pricing) |
| `cmdkUsages`, `bugbotUsages` | NEW columns `cmdk_usages`, `bugbot_usages` | Cursor-specific tools |
| `usageBasedReqs`, `subscriptionIncludedReqs` | NEW columns `usage_based_requests`, `subscription_included_requests` | billing model |
| `mostUsedModel`, `clientVersion` | NEW columns `most_used_model`, `client_version` | segmentation |
| `tabMostUsedExtension`, `applyMostUsedExtension` | NEW columns | which language extension is used most |
| (computed) `tool='cursor'`, `data_source='insight_cursor'`, `source='cursor'` | fixed values | discriminator |

### `bronze_cursor.cursor_usage_events` → `class_ai_api_usage` (per-event level → daily aggregate)

| Bronze field | Silver target | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `unique_key`, `userEmail` | identity | aggregated by (email, day, model) |
| `timestamp` → `date(timestamp)` | `day` | dimension |
| `model` | `model` | per-model breakdown |
| `kind` | `surface` (chat/composer/agent/tab) | new surface enum |
| `tokenUsage` (JSON: `inputTokens`, `outputTokens`, `cacheReadTokens`, `cacheWriteTokens`, `totalCents`) | `input_tokens` + `output_tokens` + NEW `cache_read_tokens`, `cache_write_tokens` (requires JSON parse) | token detail |
| `chargedCents`, `cursorTokenFee`, `requestsCosts` | `cost_cents` (sum) | billing |
| `isChargeable`, `isFreeBugbot`, `isTokenBasedCall`, `isHeadless`, `maxMode` | NEW columns boolean flags | classification |

### `bronze_cursor.cursor_members` → `class_people` (existing identity unification)

| Bronze field | Silver target | Notes |
|---|---|---|
| `email`, `id`, `name`, `role` | join keys into class_people | identity |
| `isRemoved` | filter | active employees only |

### `bronze_cursor.cursor_audit_logs` → `class_ai_audit_log` (NEW)

| Bronze field | Silver target | Notes |
|---|---|---|
| `event_id`, `timestamp` | grain | event-stream |
| `event_type`, `event_data` | `event_kind`, `metadata_json` | typed audit events |
| `user_email` | `actor_email` | who did it |
| `ip_address` | `ip_address` (NEW column) | compliance need |

## Claude Admin

### `bronze_claude_admin.claude_admin_code_usage` → `class_ai_dev_usage`

Source: [Claude Code Analytics API](https://platform.claude.com/docs/en/api/claude-code-analytics-api), `GET /v1/organizations/usage_report/claude_code`. Single-day grain per call (`starting_at` = one UTC day). Top-level structure: `core_metrics` + `tool_actions`. Note actor enum value is `user_actor` / `api_actor` (not `user`).

| Bronze field | Silver target | Notes |
|---|---|---|
| `actor_type ∈ {user_actor, api_actor}`, `actor_identifier` | resolved → `email` or `api_key_id` | via JOIN on api_keys table for api_actor |
| `terminal_type` | aggregated across | per terminal_type → sum'd |
| `date` | `day` | |
| `core_metrics.num_sessions` (bronze: `session_count`) | `session_count` | |
| `core_metrics.lines_of_code.added` / `.removed` (bronze: `lines_added` / `lines_removed`) | `lines_added`, `lines_removed` | |
| `core_metrics.commits_by_claude_code` | NEW column `commits_count` | available here too — not Enterprise-only |
| `core_metrics.pull_requests_by_claude_code` | NEW column `pull_requests_count` | available here too — not Enterprise-only |
| `tool_actions.{edit_tool,multi_edit_tool,write_tool,notebook_edit_tool}.accepted` | `tool_use_accepted` (sum across 4 tools) | per-tool dict, not single field |
| `tool_actions.*.rejected` | NEW column `tool_use_rejected` (sum across 4 tools) | per-tool dict |
| `tool_action_breakdown_json` (NEW, derived) | preserve per-tool accepted/rejected for drill-down | parity with Enterprise `claude_code_metrics_json` |
| (no `total_lines_*`) | NULL columns | semantic gap (Claude doesn't see manual keystrokes) |
| (computed) `tool='claude_code'`, `source='claude_admin'` | fixed | |

### `bronze_claude_admin.claude_admin_messages_usage` → `class_ai_api_usage`

API field names (per [Messages Usage Report](https://platform.claude.com/docs/en/api/admin-api/usage-cost/get-messages-usage-report)): `uncached_input_tokens`, `cache_read_input_tokens`, `cache_creation.ephemeral_5m_input_tokens`, `cache_creation.ephemeral_1h_input_tokens`, `output_tokens`. Bronze flattens / renames.

| Bronze field | Silver target | Notes |
|---|---|---|
| `api_key_id`, `workspace_id` | `api_key_id`, `workspace_id` | |
| `model`, `service_tier`, `context_window` | `model`, `service_tier`, NEW `context_window` | sub-model breakdown |
| `uncached_input_tokens` | `input_tokens` (raw uncached only — does **not** include cache reads/writes) | API has no single combined "input_tokens" — must sum 4 fields if you want total |
| `cache_read_input_tokens` (API name) | NEW column `cache_read_tokens` | reads from prompt cache |
| `cache_creation.ephemeral_5m_input_tokens`, `cache_creation.ephemeral_1h_input_tokens` (API names) | NEW columns or `cache_creation_tokens` (sum) | TTL detail |
| `output_tokens` | `output_tokens` | |
| `surface = 'messages'` | fixed | |

### `bronze_claude_admin.claude_admin_cost_report` → `class_ai_cost`

API: `/v1/organizations/cost_report`. **Cost-report does NOT support `api_key_id` grouping** — only `workspace_id` or `description`. API fields are `amount`, `currency`, `cost_type`, `context_window`, `model`, `service_tier`, `token_type`, `description`, `inference_geo`, `workspace_id`.

| Bronze field | Silver target | Notes |
|---|---|---|
| `date`, `workspace_id`, `description` | grain | no per-API-key cost breakdown |
| `amount`, `currency` (API names) | `amount_value`, `amount_currency` | |
| `cost_type` ∈ `{tokens, web_search, code_execution, session_usage}` | `line_item` | actual enum from API |
| `model`, `service_tier`, `context_window`, `token_type`, `inference_geo` | dimensions / NEW columns | additional grain |
| (computed) `provider='anthropic'` | fixed | |

### `bronze_claude_admin.claude_admin_users` → `class_people`

| Bronze field | Silver target | Notes |
|---|---|---|
| `email`, `name`, `role`, `added_at` (API name; bronze may rename to `joined_at`) | identity unification | |
| `workspace_assignments` | NEW table `class_ai_workspaces` membership | optional |

### `bronze_claude_admin.claude_admin_api_keys` → `class_ai_api_keys` (opt.) + identity resolution

| Bronze field | Silver target | Notes |
|---|---|---|
| `key_id`, `key_name`, `created_by_email`, `workspace_id` | NEW class_ai_api_keys table | for resolution api_key_id → person_id |

### `bronze_claude_admin.claude_admin_workspaces` + `_workspace_members` → `class_ai_workspaces` (opt.)

⚠️ Public Anthropic Admin API documents only `/v1/organizations/workspaces` (flat list) and `/v1/organizations/workspaces/{id}/members`. There is no documented `parent_workspace_id` / hierarchical tree. The `_workspaces_parent` bronze table (if present) is connector-specific.

| Bronze field | Silver target | Notes |
|---|---|---|
| `workspace_id`, `name`, `created_at` | structure | flat list (no documented hierarchy) |
| `member_email`, `role` | membership | who's where |

### `bronze_claude_admin.claude_admin_invites` → `class_ai_audit_log`

| Bronze field | Silver target | Notes |
|---|---|---|
| `id`, `email`, `invited_at`, `expires_at`, `role`, `status` ∈ `{accepted, expired, deleted, pending}` | `event_kind='user_invited'` audit row | API exposes no acceptance timestamp — only terminal status |

### `bronze_claude_admin.claude_admin_collection_runs` → out-of-scope

Connector self-reporting metadata about its own API calls — debugging info, not a product metric.

## Claude Enterprise

> ⚠️ **Disclaimer:** field names below are **bronze-table column names internal to our connector**, verified against the local ClickHouse `bronze_claude_enterprise.*` schema (5 tables: `claude_enterprise_users`, `claude_enterprise_summaries`, `claude_enterprise_chat_projects`, `claude_enterprise_skills`, `claude_enterprise_connectors`). They are **NOT Anthropic's public wire-format names**. The only public Anthropic endpoint that overlaps is the [Claude Code Analytics API](https://platform.claude.com/docs/en/api/claude-code-analytics-api), which uses `core_metrics.num_sessions`, `core_metrics.lines_of_code.added`, `tool_actions.{edit_tool,multi_edit_tool,write_tool,notebook_edit_tool}.{accepted,rejected}`, etc. — and covers only the `code_*` family. The `chat_*`, `cowork_*`, `excel_*`, `powerpoint_*`, `web_search_count` columns come from an undocumented Anthropic Enterprise analytics surface (or audit-log roll-up). When validating against vendor docs, the only wire-format reference is the Claude Code endpoint.

### `bronze_claude_enterprise.claude_enterprise_users` (RICHEST source)

**→ `class_ai_dev_usage` (code_* fields, where `code_session_count > 0` or `code_lines_added > 0`):**

| Bronze field | Silver target |
|---|---|
| `user_email`, `date` | identity + dimension |
| `code_session_count` | `session_count` |
| `code_lines_added` | `lines_added` |
| `code_lines_removed` | `lines_removed` |
| `code_tool_accepted_count` | `tool_use_accepted` |
| `code_tool_rejected_count` | sum into `tool_use_offered` |
| `code_commit_count` | NEW column `commits_count` (Cursor/Admin = NULL) |
| `code_pull_request_count` | NEW column `pull_requests_count` (Cursor/Admin = NULL) |
| `claude_code_metrics_json` | NEW column `tool_action_breakdown_json` (edit/multi_edit/notebook/write breakdown) |
| (computed) `tool='claude_code'`, `source='claude_enterprise'` | fixed |

**→ `class_ai_assistant_usage` — chat surface (`chat_message_count > 0`):**

| Bronze field | Silver target |
|---|---|
| `chat_message_count` | `message_count` |
| `chat_conversation_count` | `conversation_count` |
| `chat_files_uploaded_count` | `files_uploaded_count` |
| `chat_skills_used_count` | `skills_used_count` |
| `chat_connectors_used_count` | `connectors_used_count` |
| `chat_projects_created_count` | `projects_created_count` |
| `chat_projects_used_count` | `projects_used_count` |
| `chat_artifacts_created_count` | `artifacts_created_count` |
| `chat_thinking_message_count` | `thinking_message_count` |
| `chat_metrics_json` | NEW column `surface_metrics_json` |
| (computed) `surface='chat'`, `tool='claude'` | fixed |

**→ `class_ai_assistant_usage` — cowork surface (`cowork_session_count > 0`):**

| Bronze field | Silver target |
|---|---|
| `cowork_session_count` | `session_count` |
| `cowork_message_count` | `message_count` |
| `cowork_action_count` | `action_count` |
| `cowork_dispatch_turn_count` | NEW column `dispatch_turn_count` |
| `cowork_skills_used_count` | `skills_used_count` |
| `cowork_metrics_json` | `surface_metrics_json` |
| (computed) `surface='cowork'` | fixed |

**→ `class_ai_assistant_usage` — excel surface (`excel_session_count > 0`):**

| Bronze field | Silver target |
|---|---|
| `excel_session_count` | `session_count` |
| `excel_message_count` | `message_count` |
| `office_metrics_json` (excel slice) | `surface_metrics_json` |
| (computed) `surface='excel'` | fixed |

**→ `class_ai_assistant_usage` — powerpoint surface:**

| Bronze field | Silver target |
|---|---|
| `powerpoint_session_count` | `session_count` |
| `powerpoint_message_count` | `message_count` |
| `office_metrics_json` (ppt slice) | `surface_metrics_json` |
| (computed) `surface='powerpoint'` | fixed |

**→ `class_ai_assistant_usage` — cross surface (`web_search_count > 0`):**

| Bronze field | Silver target |
|---|---|
| `web_search_count` | `search_count` (+`message_count = web_search_count`) |
| (computed) `surface='cross'`, `tool='claude'` | fixed |

### `bronze_claude_enterprise.claude_enterprise_chat_projects` → `class_ai_directories` (opt.)

| Bronze field | Silver target | Notes |
|---|---|---|
| `project_id`, `project_name`, `created_by_id`, `created_by_email`, `created_at` | identity + metadata | static catalog |
| `message_count`, `distinct_user_count`, `distinct_conversation_count` | per-project rollup | activity |

### `bronze_claude_enterprise.claude_enterprise_skills` → `class_ai_directories` (opt.)

| Bronze field | Silver target |
|---|---|
| `skill_name` | identity |
| `distinct_user_count` | activity |
| `code_session_skill_used_count`, `chat_conversation_skill_used_count`, `cowork_session_skill_used_count`, `excel_session_skill_used_count`, `powerpoint_session_skill_used_count` | per-surface counts (long-format rows: skill × surface × day → count) |
| `surface_metrics_json` | metadata |

### `bronze_claude_enterprise.claude_enterprise_connectors` → `class_ai_directories` (opt.)

| Bronze field | Silver target |
|---|---|
| `connector_name`, `distinct_user_count` | identity |
| `code_session_connector_used_count`, `chat_conversation_connector_used_count`, `cowork_session_connector_used_count`, `excel_session_connector_used_count`, `powerpoint_session_connector_used_count` | per-surface counts |

### `bronze_claude_enterprise.claude_enterprise_summaries` → NEW `class_ai_org_summaries` (opt.)

| Bronze field | Silver target |
|---|---|
| `assigned_seat_count`, `pending_invite_count` | seats/billing |
| `daily_active_user_count`, `weekly_active_user_count`, `monthly_active_user_count` | DAU/WAU/MAU |
| `cowork_daily_active_user_count`, `cowork_weekly_active_user_count`, `cowork_monthly_active_user_count` | per-surface DAU |

This is **organization-level rollup** — separate grain (no user_id). Can be kept separate, or skipped entirely (recomputable at the gold level from class_ai_*_usage aggregations).

## OpenAI API

### `bronze_openai.usage_completions` → `class_ai_api_usage` (extend) + `class_ai_dev_usage` (heuristic)

**→ `class_ai_api_usage` (for all rows):**

| Bronze field | Silver target |
|---|---|
| `tenant_id`, `source_id`, `bucket_start_time → toDate(...) AS day` | grain |
| `user_id`, `project_id` | identity (resolved later) |
| `model`, `service_tier`, `is_batch` | dimensions |
| `input_tokens`, `output_tokens`, `input_cached_tokens` | tokens |
| `input_audio_tokens`, `output_audio_tokens` | audio token detail |
| `num_model_requests` | request count |
| `surface = 'messages'` | fixed |

**→ `class_ai_dev_usage` (only WHERE `model ILIKE '%codex%'`):**

| Bronze field | Silver target |
|---|---|
| `bucket_start_time`, `user_id` | grain |
| `model` | NEW column `model_used` (sub-tool detail) |
| `num_model_requests` | `session_count` (heuristic — each request = one «session») |
| `input_tokens` + `output_tokens` | NEW columns `input_tokens`, `output_tokens` (Cursor/Claude = NULL) |
| (no lines_added — OpenAI usage doesn't expose editor-level metrics) | NULL |
| (computed) `tool='codex'`, `source='openai'` | fixed |

### `bronze_openai.usage_embeddings` → `class_ai_api_usage` (NEW staging)

| Bronze field | Silver target |
|---|---|
| `bucket_start_time`, `user_id`, `project_id`, `model` | grain |
| `num_model_requests` | request count |
| `input_tokens` | tokens |
| `surface='embeddings'` | fixed |

### `bronze_openai.usage_moderations` → `class_ai_api_usage`

| Bronze field | Silver target |
|---|---|
| Same shape as embeddings | tokens, requests |
| `surface='moderations'` | fixed |

### `bronze_openai.usage_images` → `class_ai_api_usage`

API result object exposes `num_images` and (when grouped) dimensions `size`, `source` (∈ `image.generation` / `image.edit` / `image.variation`), `model`, `project_id`. No `user_id` on the row itself — group only by project.

| Bronze field | Silver target |
|---|---|
| `bucket_start_time`, `project_id`, `model` (e.g. dall-e-3) | grain |
| `num_images` | NEW column `num_images` |
| `size` (API name; bronze may rename to `image_size`) | NEW column `image_size` |
| `source` (image.generation/edit/variation) | NEW dimension `image_op_type` |
| `surface='images'` | fixed |

### `bronze_openai.usage_audio_speeches` → `class_ai_api_usage`

| Bronze field | Silver target |
|---|---|
| `bucket_start_time`, `user_id`, `model` (e.g. tts-1-hd) | grain |
| `num_seconds`, `num_characters` | NEW columns |
| `surface='audio_speech'` | fixed |

### `bronze_openai.usage_audio_transcriptions` → `class_ai_api_usage`

| Bronze field | Silver target |
|---|---|
| `bucket_start_time`, `user_id`, `model` (e.g. whisper-1) | grain |
| `num_seconds` | column |
| `surface='audio_transcription'` | fixed |

### `bronze_openai.usage_vector_stores` → `class_ai_api_usage`

API result object exposes only `usage_bytes` plus group-by `project_id`. There is **no** `num_files`, `num_calls`, `user_id`, or `model` on this surface.

| Bronze field | Silver target |
|---|---|
| `bucket_start_time`, `project_id` | grain |
| `usage_bytes` | NEW column `storage_bytes` |
| `surface='vector_stores'` | fixed |

### `bronze_openai.usage_code_interpreter` → `class_ai_api_usage`

API endpoint is `/v1/organization/usage/code_interpreter_sessions`. Result object exposes only `num_sessions` plus group-by `project_id`. There is **no** `num_calls`, `user_id`, or `model` on this surface.

| Bronze field | Silver target |
|---|---|
| `bucket_start_time`, `project_id` | grain |
| `num_sessions` | NEW column / `session_count` |
| `surface='code_interpreter'` | fixed |

> ⚠️ **Note:** "Code Interpreter" (Assistants API tool, sandbox for Python execution) ≠ Codex CLI/IDE. This stream does NOT feed `class_ai_dev_usage` — it's a different product surface.

### `bronze_openai.costs` → `class_ai_cost`

| Bronze field | Silver target |
|---|---|
| `bucket_start_time → day` | grain |
| `line_item` | `line_item` |
| `project_id` | `project_id` |
| `amount_value`, `amount_currency` | `amount_value`, `amount_currency` |
| (computed) `provider='openai'` | fixed |

### `bronze_openai.users` → `class_people`

| Bronze field | Silver target |
|---|---|
| `id`, `email`, `name`, `role`, `created_at` | identity unification |

## ChatGPT Enterprise / Edu (planned, no connector)

**Reference:** [Compliance API for Enterprise and Edu — help 9261474](https://help.openai.com/en/articles/9261474-compliance-apis-for-enterprise-customers), [Compliance Logs Platform quickstart](https://developers.openai.com/cookbook/examples/chatgpt/compliance_api/logs_platform). Single connector covers both **ChatGPT Enterprise** and **ChatGPT Edu** (same surface). Auth: HTTP Bearer token issued in workspace admin console.

⚠️ **Internal spec divergence:** the existing PRD at `connectors/ai/chatgpt-team/specs/PRD.md` describes a per-user-per-day-per-client admin endpoint with `conversation_count`, `message_count`, input/output/reasoning tokens. **No such endpoint exists publicly.** The actual data path is JSONL streaming + consumer-side aggregation. Connector dir is also misnamed (`chatgpt-team/` — the SKU was renamed to ChatGPT Business on 2025-08-29; the API surface this connector should target is Enterprise/Edu, not Business). Treat the PRD as obsolete.

### Compliance Logs Platform → multiple silver targets

JSONL streams, **30-day retention** (consumers must download continuously). Minute-level latency. Stateful Compliance route deprecates 2026-06-05. Each event category maps to a different silver class:

| Compliance Logs event category | Silver target | Notes |
|---|---|---|
| `conversations` | `class_ai_assistant_usage` (chat surface) | One event per chat turn — aggregate to daily per user; populates `message_count`, `conversation_count` |
| `uploaded_files` | `class_ai_assistant_usage` | populates `files_uploaded_count` per day |
| `workspace_gpts` | `class_ai_directories` (planned) | GPT/skill catalog metadata |
| `app_calls` | `class_ai_assistant_usage` | tool/connector invocations — populates `connectors_used_count` / `skills_used_count` |
| `memories` | metadata / drill-down | per-user memory creation/update events |
| `workspace_users` | `class_people` enrichment | seat-state changes complementing SCIM roster |
| `admin_audit` | `class_ai_audit_log` | admin actions (role changes, settings) |
| `user_authentication` | `class_ai_audit_log` | logins, SSO events |
| `codex_usage` | `class_ai_dev_usage` | Codex CLI usage **within Enterprise/Edu workspace** — authoritative source when available; replaces the `model ILIKE '%codex%'` heuristic on `bronze_openai` |

Per-event field shape requires consuming raw JSONL — exact field names not reproduced here, derive from the [quickstart](https://developers.openai.com/cookbook/examples/chatgpt/compliance_api/logs_platform). Plan for connector: download JSONL daily, parse into per-event-category bronze tables (`bronze_chatgpt_enterprise.compliance_conversations`, `compliance_codex_usage`, etc.), aggregate to daily silver in staging.

### SCIM 2.0 → `class_people` + `class_ai_seats`

Base: `https://api.openai.com/scim/v2`. Methods: `GET/POST/PUT/PATCH/DELETE /Users`, `GET /Groups`, `PATCH /Groups/{id}`. Provides seat roster — no usage data.

| API field | Silver target | Notes |
|---|---|---|
| `id` (SCIM user id) | `class_people.openai_scim_id` (NEW) | Anthropic-vendor-internal id |
| `userName`, `emails[].value` | `class_people.email` | identity unification |
| `name.givenName` / `familyName` / `displayName` | `class_people.display_name` | |
| `active` | filter | |
| `groups[]` | `class_ai_seats` membership | workspace groups |

### Out-of-scope (per public docs as of 2026-04-28)

- **Per-token / per-cost data** — Enterprise SKU is flat per-seat; tokens not exposed via API. Consumer-side derivation from event payloads possible if model field is present.
- **Daily aggregate REST endpoints** — only raw JSONL streams. All aggregation is consumer-side.
- **Workspace Analytics dashboard data** — dashboard-only per [help 10875114](https://help.openai.com/en/articles/10875114), no API.
- **Stateful Compliance API** — deprecating 2026-06-05; do not build new code against it.

## ChatGPT Business (no connector possible)

**No public REST admin API** for seats or activity. SCIM is gated to Enterprise/Edu only. Listed in the matrix as a documentation column so future readers see this was investigated — not because a connector is planned. Re-evaluate when/if OpenAI ships an admin API for the Business SKU.

## GitHub Copilot (PR #234, pending)

### `bronze_github_copilot.copilot_seats` → `class_people` + (opt.) `class_ai_seats`

| Bronze field | Silver target | Notes |
|---|---|---|
| `user_login` | NEW table `class_ai_seats.github_login` (for login → email resolution) | Copilot-only signal |
| `user_email` | `class_people.email` | identity unification |
| `plan_type` | NEW column `class_ai_seats.plan_type` | business/enterprise tier |
| `pending_cancellation_date` | `class_ai_seats.pending_cancellation_date` | seat lifecycle |
| `last_activity_at`, `last_activity_editor` | `class_ai_seats.last_activity_*` | for inactive-seat detection |
| `last_authenticated_at` | `class_ai_seats.last_authenticated_at` | |
| `created_at`, `updated_at` | `class_ai_seats.created_at/updated_at` | |

> **Note:** seats are **inventory** (not activity). A separate `class_ai_seats` class (cross-provider) for seat tracking can serve Copilot, Cursor (members), Claude Enterprise (assigned_seat_count in org_summaries). For start, can be confined to `class_people` enrichment.

### `bronze_github_copilot.copilot_user_metrics` → `class_ai_dev_usage`

JOIN with `copilot_seats` ON `user_login` for resolution → `email`.

| Bronze field | Silver target | Notes |
|---|---|---|
| `user_login` | (resolution → `email`) | identity |
| `day` | `day` | dimension |
| `loc_added_sum` | `lines_added` | core metric |
| `code_acceptance_activity_count` | `tool_use_accepted` + `completions_count` | core |
| `user_initiated_interaction_count` | `chat_requests` or NEW column `user_initiated_interactions` | Copilot-specific signal |
| `used_chat` (bool) | NEW boolean column `used_chat_today` | activity flag |
| `used_agent` (bool) | NEW boolean column `used_agent_today` | activity flag |
| `used_cli` (bool) | NEW boolean column `used_cli_today` | activity flag |
| (computed) `tool='copilot'`, `source='github_copilot'`, `data_source='insight_github_copilot'` | fixed | discriminator |
| (computed) `total_lines_added` | NULL | semantic gap (Copilot doesn't expose total — same as Claude/Codex) |
| (computed) `tool_use_offered` | NULL | Copilot exposes only accepted, not offered |
| (computed) `agent_sessions` | NULL (only `used_agent` flag, not count) | partial |

## Windsurf (planned, no connector)

**Reference:** API per [docs.windsurf.com](https://docs.windsurf.com/plugins/accounts/api-reference/api-introduction).

6 endpoints (POST to `https://server.codeium.com/api/v1/`, service-key auth in body) — 3 analytics + 3 config/billing:

Analytics:
- `/Analytics` — flexible queryable, 4 data sources (USER_DATA / CHAT_DATA / COMMAND_DATA / PCW_DATA)
- `/CascadeAnalytics` — Cascade-specific (Lines, Runs, Tool Usage)
- `/UserPageAnalytics` — user roster + activity

Config / billing:
- `/UsageConfig` — write usage limits
- `/GetUsageConfig` — read usage limits
- `/GetTeamCreditBalance` — team credit balance

### `/UserPageAnalytics` → `class_people` + (opt.) `class_ai_seats`

| Bronze field | Silver target | Notes |
|---|---|---|
| `userTableStats[].email` | `class_people.email` | identity |
| `userTableStats[].name` | `class_people.display_name` | |
| `userTableStats[].apiKey` | NEW `class_ai_seats.windsurf_api_key_id` (encrypted) | Windsurf-only key |
| `userTableStats[].activeDays` | NEW column for org reports | rolling activity |
| `userTableStats[].role` | `class_people.role` | admin/member |
| `userTableStats[].signupTime`, `.lastUpdateTime` | `class_ai_seats.created_at`, `.last_activity_at` | |
| `userTableStats[].lastAutocompleteUsageTime` | NEW column `last_autocomplete_at` | per-modality recency |
| `userTableStats[].lastChatUsageTime` | NEW column `last_chat_at` | per-modality |
| `userTableStats[].lastCommandUsageTime` | NEW column `last_command_at` | per-modality |
| `userTableStats[].promptCreditsUsed` | `class_ai_cost.amount_value` (cents) | billing per-user-cycle |
| `userTableStats[].teamStatus` | filter only (PENDING/APPROVED/REJECTED) | |
| `userTableStats[].disableCodeium` | `class_ai_seats.is_disabled` | admin revoked |
| `billingCycleStart`, `billingCycleEnd` | metadata for billing window | not row-level |

### `/Analytics` (User Data) → `class_ai_dev_usage`

| Field | Silver target |
|---|---|
| `num_acceptances` | `tool_use_accepted` + `completions_count` |
| `num_lines_accepted` | `lines_added` |
| `num_bytes_accepted` | NEW column `bytes_added` |
| dimension `language` | NEW `language` column for granularity |
| dimension `IDE` (editor/jetbrains/cli) | NEW `editor` column |
| dimension `version` | `client_version` |
| dimension `date/hour` | `day` (truncate to day) |
| (computed) `tool='windsurf'`, `data_source='insight_windsurf'` | fixed |

### `/Analytics` (Chat Data) → `class_ai_assistant_usage`

| Field | Silver target |
|---|---|
| `num_chats_received` | `message_count` |
| `chat_accepted` | NEW column `chat_suggestions_accepted` |
| `chat_inserted_at_cursor` | NEW column |
| `chat_applied` | NEW column |
| `chat_loc_used` | NEW column `chat_lines_inserted` |
| dimension `latest_intent_type` (8 types) | NEW column `chat_intent` |
| (computed) `surface='chat'`, `tool='windsurf'` | fixed |

### `/Analytics` (Command Data) → `class_ai_dev_usage`

| Field | Silver target |
|---|---|
| `lines_added`, `lines_removed` | sum-merge with autocomplete `lines_added` |
| `bytes_added`, `bytes_removed` | new columns |
| selection metrics | NEW columns |
| dimension `command_source` (8 sources) | NEW `command_source` for sub-tool breakdown |
| dimension `provider_source` (generation/edit) | sub-mode |

### `/Analytics` (PCW Data) → `class_ai_dev_usage`

**Unique**: PCW = Percent of Code Written. This is a **full analog of Cursor's `ai_loc_share`** — Windsurf is one of the few providers that sees "total bytes" (everything the user typed) and can compute an honest ratio.

| Field | Silver target |
|---|---|
| `percent_code_written` | NEW column `pcw_percent` or used for derived `ai_loc_share` |
| `codeium_bytes` | aggregated bytes_added (AI-accepted) |
| `user_bytes` | NEW column `user_typed_bytes` (manual) |
| `total_bytes` | maps to `total_lines_added` semantically (but byte grain) |
| autocomplete vs command contribution breakdown | NEW per-source % |

### `/CascadeAnalytics` (Cascade Lines) → `class_ai_dev_usage`

| Field | Silver target |
|---|---|
| `day` | `day` |
| `linesSuggested` | `tool_use_offered` (if applying Cascade-as-tool model) |
| `linesAccepted` | augments `lines_added` (when Cascade lines = subset of total accepted) |

### `/CascadeAnalytics` (Cascade Runs) → `class_ai_dev_usage` (agent_sessions detail) + `class_ai_api_usage`

| Field | Silver target |
|---|---|
| `day`, `model` | grain |
| `mode` ∈ `{CONVERSATIONAL_PLANNER_MODE_DEFAULT, CONVERSATIONAL_PLANNER_MODE_READ_ONLY, CONVERSATIONAL_PLANNER_MODE_NO_TOOL, UNKNOWN}` (full string with prefix; 4 values) | NEW dimension `cascade_mode` |
| `messagesSent` | `chat_requests` (chat-mode) or `agent_sessions` (agent-mode) per `mode` |
| `cascadeId` | NEW `cascade_run_id` (not a unique_key, but useful for drill-down) |
| `promptsUsed` (credits in cents) | `cost_cents` |

### `/CascadeAnalytics` (Cascade Tool Usage) → `class_ai_dev_usage` (NEW column with breakdown)

| Field | Silver target |
|---|---|
| `tool` (CODE_ACTION/VIEW_FILE/RUN_COMMAND/etc.) | aggregate breakdown JSON or wide columns |
| `count` | per-tool count |

Can be aggregated into `tool_action_breakdown_json` column in `class_ai_dev_usage` — analog of `claude_code_metrics_json` but for Windsurf.

---

### `bronze_github_copilot.copilot_org_metrics` → `class_ai_org_usage` (NEW class!)

PR #234 explicitly declares a **new silver class `class_ai_org_usage`** (deferred — model doesn't yet exist). Org-level rollup, separate from per-user grain.

⚠️ The legacy `/copilot/metrics` endpoint exposed `total_active_user_count`, `total_engaged_user_count`, `total_code_acceptance_activity_count` directly. The **new** `organization-1-day` reports API (signed-URL NDJSON) does **NOT** carry those `total_*` fields — these must be derived by aggregating `users-1-day` rows. Field names below describe the bronze table after our connector materializes that aggregation.

| Bronze field | Silver target | Notes |
|---|---|---|
| `day` (= report_day) | grain | dimension |
| `total_active_user_count` (derived from `users-1-day` aggregation) | `total_active_users` | `COUNT(DISTINCT user_id) WHERE user_initiated_interaction_count > 0` |
| `total_engaged_user_count` (derived) | `total_engaged_users` | substantive-activity threshold — pick our own (legacy threshold is GitHub-internal) |
| `total_code_acceptance_activity_count` (derived) | `total_code_acceptances` | `SUM(code_acceptance_activity_count)` |
| (computed) `tool='copilot'`, `surface='aggregate'` | fixed | |

Per-IDE / per-language / per-feature breakdowns also exist in the new schema (`code_generation_activity_count`, `loc_added_sum`, `loc_changed_sum`, agent fields, etc.) — extend as needed.

## JetBrains AI (planned, no connector)

**Reference:** [JetBrains IDE Services AI Analytics API](https://www.jetbrains.com/help/ide-services/ai-analytics-api.html). Endpoints under `/api/analytics/ai/*`. Authoritative response field names live in tenant-served Swagger (`https://<your_org>/swagger-ui.html`); public help pages name metric concepts only. Bronze field names below use snake_case as a placeholder — confirm against tenant Swagger before connector implementation.

⚠️ **Internal spec divergence:** the existing spec at `connectors/ai/jetbrains/jetbrains.md` calls this "JetBrains AI Enterprise admin API" and gates the connector behind a "JetBrains AI Enterprise" subscription. Both are wrong per JetBrains' public docs — the API is the IDE Services / Central Console analytics surface, available to any AI Pro / Ultimate / All Products Pack customer running IDE Services. Several spec fields (`most_used_model`, `models_used`, `chat_sessions`, `ide_product_code`, `ide_version`, `plugin_version`, `last_seen_at`, `active_days_in_period`) are not documented publicly — likely speculative. Treat the spec as draft until validated against a real tenant Swagger.

### `/api/analytics/ai/effectiveness/metrics/users.paged` → `class_ai_dev_usage` (proposed bronze: `bronze_jetbrains_ai.activity`)

Per-user-per-day-per-tool aggregates. Tool taxonomy is built into the API: `tool ∈ {junie, aia, claude_code, code_completion, nes}` — every row is per-tool. Pagination via `cursor` + `limit` (1–1000). Filter by user UUID list via `query`.

| Bronze field (TBD via Swagger) | Silver target | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `unique_key` | identity | base |
| `user_uuid` (JB internal id) | join key — resolves via `/api/v1/users` to email | identity |
| `email` (resolved) | `email` | identity |
| `date` | `day` | dimension |
| `tool` ∈ {`junie`,`aia`,`claude_code`,`code_completion`,`nes`} | `tool` (verbatim) | `source='jetbrains_ai'` discriminates from Anthropic's `tool='claude_code'`; gold `v_ai_usage_unified` merges across `source` per documented rules |
| suggestions shown (concept) | `tool_use_offered` | unique non-Cursor signal |
| suggestions accepted (concept) | `tool_use_accepted` | |
| suggestions rejected (concept) | NEW column `tool_use_rejected` | |
| lines added (concept) | `lines_added` | |
| lines deleted (concept) | `lines_removed` | |
| lines modified (concept) | NEW column `lines_modified` | JB-unique granularity |
| invocations (concept) | NEW column `invocations` (≠ session_count) | |
| AI code acceptance rate (concept) | derive `tool_use_accepted / tool_use_offered` in silver | `v_ai_usage_unified` reuses |
| (computed) `source='jetbrains_ai'`, `data_source='insight_jetbrains_ai'`, `provider_org_id=<JB tenant id>` | fixed | discriminator |
| (computed) `total_lines_added` / `total_lines_removed` | NULL | semantic gap (no manual-keystroke denominator) |
| (computed) `input_tokens` / `output_tokens` / `cost_cents` | NULL | not exposed |

### `/api/analytics/ai/users-adoption` → org rollup (`class_ai_org_usage` planned)

Period-grouped (`groupBy ∈ DAY/WEEK/MONTH/YEAR`) "active users" / "users with AI access" counts. Feeds the same `class_ai_org_usage` class as Copilot's `organization-1-day` rollup — different vendor, same concept.

| Bronze field | Silver target | Notes |
|---|---|---|
| `period_start`, `period_end` | grain | matches `groupBy` |
| `active_users` | `total_active_users` | concept-level |
| `users_with_access` | `total_seats` (proxy) | seat inventory analog |
| (computed) `tool='jb:any'`, `surface='aggregate'` | fixed | |

### `/api/analytics/ai/effectiveness/metrics-by-feature` → optional drill-down

Same metrics as the per-user endpoint but pivoted by tool/feature. Useful for org dashboards "what's the most-used JB AI feature" without per-user noise. Not strictly needed if the per-user staging is in place — gold views can pivot.

### `/api/v1/users` → `class_people`

| API field | Silver target | Notes |
|---|---|---|
| `id` (user UUID) | `class_people.jetbrains_user_uuid` (NEW) | join key for per-user analytics rows |
| `email` | `class_people.email` | identity unification |
| `username` | metadata | optional |
| `role` (`member`/`admin`) | `class_people.role` | |
| `isActive` (boolean) | filter (active=true) | |
| `createdDate`, `createdBy` | metadata | |

### Out-of-scope (per public docs as of 2026-04-28)

- Per-token / per-cost data — not exposed publicly. AI Credit consumption is dashboard-only per the [Feb 2026 JetBrains blog](https://blog.jetbrains.com/ai/2026/02/enhanced-ai-management-and-analytics-for-organizations/).
- Per-event invocation log — only daily aggregates published.
- Per-IDE / per-model breakdown — public docs do not name these as exposed via API. Internal spec speculates `ide_product_code` / `most_used_model` — validate against tenant Swagger before assuming present.

---

## Extended `class_ai_dev_usage` schema (for maximum data)

Old schema (until today):
```
session_count, lines_added, lines_removed, total_lines_added, total_lines_removed,
tool_use_offered, tool_use_accepted, completions_count,
agent_sessions, chat_requests, cost_cents
```

**Extended** (incorporating fields from all 4 providers):
```
-- core (existing):
session_count, lines_added, lines_removed,
total_lines_added, total_lines_removed,
tool_use_offered, tool_use_accepted, tool_use_rejected,
completions_count, agent_sessions, chat_requests, cost_cents,

-- Cursor-specific (NEW):
api_request_count           -- apiKeyReqs
cmdk_usages                 -- cmdkUsages
bugbot_usages               -- bugbotUsages
usage_based_requests        -- usageBasedReqs
subscription_included_requests -- subscriptionIncludedReqs
most_used_model             -- mostUsedModel
client_version              -- clientVersion

-- Claude Code (Admin via Code Analytics API + Enterprise via users.code_*) (NEW):
commits_count               -- core_metrics.commits_by_claude_code (Admin) / code_commit_count (Enterprise)
pull_requests_count         -- core_metrics.pull_requests_by_claude_code (Admin) / code_pull_request_count (Enterprise)
tool_action_breakdown_json  -- tool_actions.{edit,multi_edit,write,notebook_edit}_tool.{accepted,rejected} (Admin) / claude_code_metrics_json (Enterprise)

-- OpenAI Codex-specific (NEW):
input_tokens, output_tokens -- only for tool='codex'
model_used                  -- e.g. 'gpt-5-codex' vs 'gpt-5.3-codex'

-- GitHub Copilot-specific (NEW, post-PR-#234):
used_chat_today, used_agent_today, used_cli_today  -- boolean activity flags
user_initiated_interactions                        -- user_initiated_interaction_count
last_activity_editor                               -- vscode/jetbrains/etc. (also from Cursor as variant)

-- JetBrains AI-specific (NEW):
lines_modified              -- JB exposes added/modified/deleted (3-way), unlike others' 2-way
invocations                 -- ≠ session_count; JB's per-tool call count
                            -- (tool axis: junie/aia/claude_code/code_completion/nes — already in `tool` column)
```

NULL-policy: per-source enforce that columns without semantic meaning = NULL (not 0). E.g. `total_lines_added` for Claude/Copilot rows = NULL, `commits_count` for Cursor/Copilot/OpenAI rows = NULL, `tool_use_offered` for Claude/Copilot rows = NULL.

## Extended `class_ai_assistant_usage` schema (max coverage)

```
insight_tenant_id, source_id, unique_key
email, day
tool                    ('claude' | 'chatgpt' | future)
surface                 ('chat' | 'cowork' | 'excel' | 'powerpoint' | 'cross')

-- core (applies to all surfaces):
session_count, message_count, action_count

-- chat-specific (NULL for other surfaces):
conversation_count, files_uploaded_count, artifacts_created_count
projects_created_count, projects_used_count
skills_used_count, connectors_used_count
thinking_message_count

-- cowork-specific:
dispatch_turn_count

-- cross-surface (web search):
search_count

-- meta:
surface_metrics_json    -- raw JSON breakdown (chat_metrics_json, cowork_metrics_json,
                            office_metrics_json) for drill-down
cost_cents, source, data_source, collected_at
```

## Extended `class_ai_api_usage` schema (max coverage across all 4 providers)

```
insight_tenant_id, source_id, unique_key
day, model, api_key_id, project_id, user_id, workspace_id
surface                 ('messages' | 'embeddings' | 'moderations' | 'images'
                         | 'audio_speech' | 'audio_transcription'
                         | 'vector_stores' | 'code_interpreter')
service_tier, is_batch, context_window

-- token counts (NULL for non-token surfaces):
input_tokens, output_tokens, input_cached_tokens
cache_read_tokens, cache_creation_tokens
input_audio_tokens, output_audio_tokens

-- request count:
num_model_requests

-- modality-specific (NULL for irrelevant surfaces):
num_seconds              -- audio
num_characters           -- audio TTS
num_images               -- image gen
image_size               -- image gen
num_files                -- vector_stores
storage_bytes            -- vector_stores
num_sessions             -- code_interpreter

cost_cents
provider                 ('anthropic' | 'openai' | 'cursor')
source, data_source, collected_at
```

---

## Extended coverage matrix of proposed silver classes

| Silver class | Cursor | Claude Admin | Claude Enterprise | OpenAI API | ChatGPT Ent. † | ChatGPT Business ‡ | GitHub Copilot * | Windsurf ** | JetBrains AI *** |
|---|---|---|---|---|---|---|---|---|---|
| `class_ai_dev_usage` | ✅ daily_usage (rich) | ✅ code_usage | ✅ users.code_* (incl. commits/PRs) | 🟡 completions WHERE model ILIKE '%codex%' | ✅ Compliance Logs `codex_usage` events (authoritative for Codex within Enterprise workspace) | ❌ | ✅ user_metrics + seats join | ✅ /Analytics User+Command+PCW + /CascadeAnalytics | ✅ `/effectiveness/metrics/users.paged` (per-user × tool × day) |
| `class_ai_assistant_usage` | ❌ | ❌ | ✅ users.chat_*+cowork_*+office_*+web_search | ❌ (different domain) | ✅ Compliance Logs `conversations`+`uploaded_files`+`app_calls`+`workspace_gpts` | ❌ | ❌ | 🟡 /Analytics Chat | ❌ |
| `class_ai_api_usage` (extended) | 🟡 usage_events (NEW staging) | ✅ messages_usage | ❌ | ✅ all 8 surfaces (vector_stores / code_interpreter only `usage_bytes` / `num_sessions`) | ❌ (flat-seat — no token data) | ❌ | ❌ | 🟡 /CascadeAnalytics Runs (`promptsUsed` in cents — billing, not tokens) | ❌ |
| `class_ai_cost` | ❌ | ✅ cost_report | ❌ | ✅ costs | ❌ (flat-seat — not exposed via API) | ❌ | 🟡 only seat-level (derived from `plan_type`) | ✅ /UserPageAnalytics `promptCreditsUsed`, /Cascade per-run | ❌ |
| `class_ai_audit_log` | ✅ audit_logs | 🟡 invites + Compliance future | ❌ | ✅ Audit Logs API (per [help 9687866](https://help.openai.com/en/articles/9687866)) | ✅ Compliance Logs `admin_audit`+`user_authentication` events | ❌ | ❌ | ❌ | ❌ |
| `class_ai_org_usage` ⭐ NEW | ❌ | ❌ | 🟡 summaries (assigned/active counts) | ❌ | 🟡 derived from event stream aggregation | ❌ | ✅ org_metrics (DAU/engagement) | 🟡 derived from /UserPageAnalytics roster + activity | ✅ `/users-adoption` (period-grouped) |
| `class_ai_seats` (opt.) NEW | ✅ members | ❌ | 🟡 users (assigned implicitly) | ❌ | ✅ SCIM `/Users` + `/Groups` (rich) | ❌ | ✅ seats (rich plan info) | ✅ UserPageAnalytics (rich) | 🟡 `/api/v1/users` (role/isActive — no per-user plan tier) |
| `class_ai_directories` (opt.) | ❌ | ❌ | ✅ skills + connectors + chat_projects | ❌ | 🟡 `workspace_gpts` event metadata | ❌ | ❌ | 🟡 Cascade Tool Usage — limited | ❌ |
| `class_ai_workspaces` (opt.) | ❌ | ✅ workspaces + members | ❌ | 🟡 projects (separate from workspaces) | 🟡 SCIM `/Groups` | ❌ | ❌ (different org concept) | ✅ groups (`group_name` filter) | 🟡 IDE Services profiles (`profileId` filter) |
| `class_ai_api_keys` (opt.) | ❌ | ✅ api_keys | ❌ | ❌ | ❌ | ❌ | ❌ (PAT-based) | 🟡 service_keys (admin-level only) | ❌ |
| `class_people` (existing) | ✅ members | ✅ users | ✅ users | ✅ users | ✅ SCIM `/Users` | ❌ | ✅ seats (login + email) | ✅ UserPageAnalytics | ✅ `/api/v1/users` |

\* GitHub Copilot — *pending PR #234*. Implementation expected after PRD/DESIGN review.
\*\* Windsurf — *planned only*, real API verified per [docs.windsurf.com](https://docs.windsurf.com/plugins/accounts/api-reference/api-introduction). No connector PR open.
\*\*\* JetBrains AI — *planned only*. Public API surface confirmed; bronze schema TBD via tenant-served Swagger.
† ChatGPT Enterprise — *planned only*. Covers Enterprise + Edu SKUs (same surface). Compliance Logs Platform JSONL streaming + SCIM 2.0. 30-day retention; stateful route deprecates 2026-06-05.
‡ ChatGPT Business — formerly "ChatGPT Team". No public admin API; documentation column.

## Implementation effort (rough)

| Class | Provider feeds | Effort |
|---|---|---|
| `class_ai_dev_usage` extension | + claude_enterprise + cursor extras + codex heuristic | M (3 staging changes + schema columns) |
| `class_ai_assistant_usage` NEW | claude_enterprise (5 surface-row variants) | L (new silver model + multi-row staging) |
| `class_ai_api_usage` extension | + cursor usage_events + 7 OpenAI surfaces | L (new cursor staging + extend OpenAI staging) |
| `class_ai_cost` NEW | claude_admin + openai (re-tag) | S (new silver model, retag staging) |
| `class_ai_audit_log` NEW | cursor (existing stream) | S (one staging) |
| `class_ai_directories` NEW (opt.) | claude_enterprise (3 streams) | M |
| `class_ai_workspaces` NEW (opt.) | claude_admin (3 streams) | M |
| `class_ai_api_keys` NEW (opt.) | claude_admin (1 stream) | S |
| `class_ai_org_summaries` NEW (opt.) | claude_enterprise (1 stream) | S |
| FE bullet entries | per metric | depends on scope |
| Catalog rows | per metric | per metric |

**Core total** (without opt.): 5 silver classes, 3 NEW + 2 extending. ~2-3 backend PRs, 1 FE PR, 1 catalog PR.

**Maximum total** (with opt.): 9 silver classes. 7-9 backend PRs, 3-4 FE PRs, catalog updates.

I'd build incrementally: core first, optional classes per product request when concrete metrics are needed (e.g. "the director wants to see skill popularity" → implement `class_ai_directories`).

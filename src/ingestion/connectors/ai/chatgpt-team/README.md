# ChatGPT Team Connector

Extracts ChatGPT Team/Enterprise workspace data (seats, per-user chat &
Codex activity, subscription spend) into the Bronze layer.

**Source**: chatgpt.com web API (`/backend-api/*`) — reached through a
customer-deployed **browser proxy**, NOT the OpenAI Admin API.

**Auth model**: Insight authenticates to the proxy with a shared bearer
token (`proxy_auth_token`). The chatgpt.com session and the derived
access_token live only on the proxy — Insight never sees them.

## Specification

- **Proxy**: `https://gitlab.constr.dev/insight/secure-enclave` → `proxies/chatgpt_team/`

## Prerequisites

1. A **workspace Owner/Admin** account on the ChatGPT Team/Enterprise workspace
   (the analytics/subscription endpoints are admin-only).
2. A deployed `chatgpt-team-proxy` with the session installed
   (`POST /admin/session-key`) and reachable from Insight.
3. The workspace `account_id` and `org_id` (from `chatgpt.com/api/auth/session`).

## K8s Secret

See [`src/ingestion/secrets/connectors/chatgpt-team.yaml.example`](../../../secrets/connectors/chatgpt-team.yaml.example).
Required: `chatgpt_account_id`, `proxy_url`, `proxy_auth_token`.
Optional: `chatgpt_org_id` (only for the subscription streams; `analytics-viewer`
accounts have no billing visibility and omit it — the subscription streams then
tolerate the 403/404), `start_date`. No OpenAI admin key, no session — those are
not Insight's concern.

## Streams

| Stream | Endpoint (proxy `/api/*` → `chatgpt.com/backend-api/*`) | Sync mode | Pagination | `unique_key` |
|--------|----------|-----------|------------|--------------|
| `chatgpt_team_seats` | `/api/accounts/{account_id}/users` | Full refresh | offset/limit | `{tenant}-{source}-{user_id}` |
| `chatgpt_team_chat_activity` | `/api/accounts/{account_id}/analytics/user_list` | Incremental (`date`) | cursor (`after_cursor`) | `{tenant}-{source}-{date}-{email}` |
| `chatgpt_team_codex_user_daily` | `/api/wham/analytics/usage-leaderboard` | Incremental (`date`) | page-number | `{tenant}-{source}-{date}-{email}` |
| `chatgpt_team_codex_user_daily_org` | `/api/wham/analytics/usage-leaderboard` | Incremental (`date`) | none (`page_size=1`) | `{tenant}-{source}-{date}-{read_at}` |
| `chatgpt_team_subscription_usage` | `/api/subscriptions/{org_id}/usage` | Full refresh (snapshot) | none | `{tenant}-{source}-{snapshot_date}-{model}` |
| `chatgpt_team_subscription_balance` | `/api/subscriptions/{org_id}/usage` | Full refresh (snapshot) | none | `{tenant}-{source}-{snapshot_date}` |
| `chatgpt_team_account_settings` | `/api/accounts/{account_id}/settings` | Full refresh (snapshot) | none | `{tenant}-{source}-{snapshot_date}` |

### Notes

- **Per-day streams** (`chat_activity`, `codex_user_daily`) walk one day per
  request via a `DatetimeBasedCursor` (`step: P1D`), injecting the day as
  `date` (the per-user objects don't carry it). Backfill from `start_date`
  (default 7 days ago), with no floor — the reachable history is whatever the
  workspace has.
- **Seat caps.** `credit_limits` on the roster is an *override* with three
  distinct states — absent, an empty array that removes the cap, and an array
  holding `{enforcement_mode, limit, limit_mode}`. The default that governs
  everyone without an override lives in `account_settings`
  (`seat_type_credit_limits`, keyed by seat type). Caps are denominated in
  credits, not currency.
- **`codex_user_daily_org`** asks the same endpoint for one row and keeps only
  the envelope's `total_users`. It is the reference the completeness gate in
  `chatgpt_team__ai_dev_usage` judges a read against; the read is in its
  `unique_key` so each read keeps its own headcount.
- **Codex `credits` is on-demand usage, not total consumption.** It matches
  the vendor's own on-demand figure wherever one is published, and Codex
  activity routinely records zero credits while still reporting tokens — so a
  person-day with no credits is ordinary work, not a gap. Whether the
  uncredited part is allowance-covered or simply unmetered is not established;
  either way it is excluded, so never read the figure as a total. Nothing in
  this connector turns it into money.
- **Subscription streams** hit the same endpoint (one extracts `usage_detail`,
  the other the root `current_balance`), inject a `snapshot_date` so daily
  snapshots accumulate, and **tolerate HTTP 401, 403 and 404** (role below
  account-admin, session gated out of billing, or a blank/wrong org id →
  stream skipped, sync stays green — watched by
  `assert_chatgpt_subscription_stream_not_silent`, with the sync log carrying
  which of the three it was). Billing-cycle alignment (the cycle resets
  mid-month) is deferred; the request uses a fixed `[-30d, today]` window that
  deliberately ignores `start_date`.

## Validation

```bash
./src/ingestion/tools/declarative-connector/source.sh validate-strict ai/chatgpt-team
./src/ingestion/tools/declarative-connector/source.sh check           ai/chatgpt-team <tenant>
```

### What has been read back from a workspace

| Surface | State |
|---|---|
| `/api/accounts/{account_id}/users` | **Verified.** Declared fields match, including the three `credit_limits` states, `deactivated_time` and `pending_seat_type`. |
| `/api/accounts/{account_id}/settings` | **Verified.** `seat_type_credit_limits` and `default_seat_type` present as declared. |
| `/api/wham/analytics/usage-leaderboard` | **Verified.** The envelope carries `total_users` even at `page_size=1`; `code_attribution.lines_of_code` holds only `added`; the endpoint is 1-indexed. |
| `/api/subscriptions/{org_id}/usage` | **Permission-limited.** A role below account-admin answers `401`, so no response body has been read. Field shapes still come from the `data_collector` prototype and are unconfirmed. |

> ⚠️ Read the two subscription streams' field lists as a declared shape, not an
> observed one. They are reachable only with an account-admin or account-owner
> role; raising it is the only way to confirm them.

## Silver Targets

Shipped (this connector's `dbt/`):
- `chatgpt_team_codex_user_daily` → `chatgpt_team__ai_dev_usage` → **`class_ai_dev_usage`** (`tool='codex'`, alongside Claude Code / Cursor).
- `chatgpt_team_chat_activity` → `chatgpt_team__ai_assistant_usage` → **`class_ai_assistant_usage`** (`tool='chatgpt'`, `surface='chat'`).

- `chatgpt_team_seats` → `chatgpt_team__seats_latest` → snapshot → fields
  history → `chatgpt_team__identity_inputs` → **`identity_inputs`**
  (`source_type='chatgpt-team'`, contributing `email` and `display_name`).

The class relations then feed the `ai_usage` gold models and the metric registry.

## Related

- `claude-team` — the reference browser-proxy connector (same architecture).
- The OpenAI **Admin API** (`api.openai.com`) is a distinct programmatic
  surface measuring API-key spend rather than workspace seats. No connector
  for it ships today.

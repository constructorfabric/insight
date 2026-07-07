# ChatGPT Team Connector

Extracts ChatGPT Team/Enterprise workspace data (seats, per-user chat &
Codex activity, subscription spend) into the Bronze layer.

**Source**: chatgpt.com web API (`/backend-api/*`) — reached through a
customer-deployed **browser proxy**, NOT the OpenAI Admin API. See
[ADR-001](../../../../docs/components/connectors/ai/chatgpt-team/specs/ADR/ADR-001-browser-proxy-architecture.md).

**Auth model**: Insight authenticates to the proxy with a shared bearer
token (`proxy_auth_token`). The chatgpt.com session and the derived
access_token live only on the proxy — Insight never sees them.

## Specification

- **PRD**: [`docs/components/connectors/ai/chatgpt-team/specs/PRD.md`](../../../../docs/components/connectors/ai/chatgpt-team/specs/PRD.md)
- **DESIGN**: [`docs/components/connectors/ai/chatgpt-team/specs/DESIGN.md`](../../../../docs/components/connectors/ai/chatgpt-team/specs/DESIGN.md)
- **ADR-001** (browser-proxy architecture): [`specs/ADR/ADR-001-browser-proxy-architecture.md`](../../../../docs/components/connectors/ai/chatgpt-team/specs/ADR/ADR-001-browser-proxy-architecture.md)
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
| `chatgpt_team_subscription_usage` | `/api/subscriptions/{org_id}/usage` | Full refresh (snapshot) | none | `{tenant}-{source}-{snapshot_date}-{model}` |
| `chatgpt_team_subscription_balance` | `/api/subscriptions/{org_id}/usage` | Full refresh (snapshot) | none | `{tenant}-{source}-{snapshot_date}` |

### Notes

- **Per-day streams** (`chat_activity`, `codex_user_daily`) walk one day per
  request via a `DatetimeBasedCursor` (`step: P1D`), injecting the day as
  `date` (the per-user objects don't carry it). Backfill from `start_date`
  (default 7 days ago); `min_datetime: 2026-01-14` is the earliest observed
  data point — verify per tenant.
- **Subscription streams** hit the same endpoint (one extracts `usage_detail`,
  the other the root `current_balance`), inject a `snapshot_date` so daily
  snapshots accumulate, and **tolerate HTTP 403** (session lacks billing
  visibility → stream skipped, sync stays green). Billing-cycle alignment
  (resets on the 14th) is simplified to a `[start_date or -30d, today]`
  window — see spec OQ.

## Validation

```bash
./src/ingestion/tools/declarative-connector/source.sh validate-strict ai/chatgpt-team
./src/ingestion/tools/declarative-connector/source.sh check           ai/chatgpt-team <tenant>
```

> ⚠️ **Unverified against a live instance.** Endpoint shapes, fields, and the
> proxy's access-token flow are derived from the `data_collector` prototype
> and must be confirmed once workspace credentials are available
> (spec OQ-CGT-3/4/6).

## Silver Targets

Shipped (this connector's `dbt/`):
- `chatgpt_team_codex_user_daily` → `chatgpt_team__ai_dev_usage` → **`class_ai_dev_usage`** (`tool='codex'`, alongside Claude Code / Cursor).
- `chatgpt_team_chat_activity` → `chatgpt_team__ai_assistant_usage` → **`class_ai_assistant_usage`** (`tool='chatgpt'`, `surface='chat'`).

Plus the bronze→RMT promotion (`chatgpt_team__bronze_promoted`). The keys then flow to Gold (`ai_bullet_rows`) and the analytics query_ref / `metric_catalog`.

## Related

- `claude-team` — the reference browser-proxy connector (same architecture).
- `openai` — the OpenAI **Admin API** connector (`class_ai_api_usage`); distinct
  programmatic surface, not collected here.

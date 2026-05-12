# Claude Admin Connector

Extracts Anthropic organization administrative data (users, workspaces, workspace members, pending invites, API keys, daily messages token usage, daily cost report, and daily Claude Code usage) from the Anthropic Admin API into the Bronze layer.

Authentication: Anthropic Admin API key (Organization admin scope), sent via `x-api-key` header.

This connector consolidates the previous `claude-api` (programmatic API usage + cost) and `claude-team` (seats, code usage, workspaces) connectors into a single package. Both previous connectors hit the same API (`api.anthropic.com`) with the same credential, had two overlapping endpoints (`/v1/organizations/workspaces`, `/v1/organizations/invites`), and produced 10 Bronze streams combined. After deduplicating the overlaps this connector produces 8 Bronze streams.

## Specification

- **PRD**: [../../../../../docs/components/connectors/ai/claude-admin/specs/PRD.md](../../../../../docs/components/connectors/ai/claude-admin/specs/PRD.md)
- **DESIGN**: [../../../../../docs/components/connectors/ai/claude-admin/specs/DESIGN.md](../../../../../docs/components/connectors/ai/claude-admin/specs/DESIGN.md)

## Prerequisites

1. The deploying organization must be on an Anthropic plan with Admin API access (Team, Enterprise, or API organization).
2. An organization admin creates an Admin API key at [console.anthropic.com](https://console.anthropic.com/) with organization-level read scope.
3. The API enforces a maximum date range of 31 days per request for usage/cost endpoints; the connector steps through dates at `P1D` granularity (one day per request) to avoid boundary-day loss.

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-claude-admin-main
  namespace: data
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: claude-admin
    insight.cyberfabric.com/source-id: claude-admin-main
type: Opaque
stringData:
  admin_api_key: "<your-key>"
  # start_date: "2026-01-01T00:00:00Z"  # optional, full ISO 8601 required; default = 90 days ago
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `admin_api_key` | Yes | Anthropic Admin API key with organization-level read scope. Marked `airbyte_secret: true` — never logged. |
| `start_date` | No | Earliest date to collect usage / cost / code-usage data from. Must be full ISO 8601 (`YYYY-MM-DDThh:mm:ssZ`, e.g. `2026-01-01T00:00:00Z`). Bare `YYYY-MM-DD` is rejected by the messages/cost streams' strict datetime parser. Default: 90 days ago. |

> `insight_source_id` is **not** a `stringData` field — it is injected from the `insight.cyberfabric.com/source-id` annotation on the Secret (see Automatically injected below). Setting it in `stringData` has no effect.

### Automatically injected

These fields are added to every record by the connector — do **not** put them in the K8s Secret:

| Field | Source |
|-------|--------|
| `tenant_id` | `tenant_id` from tenant YAML (`connections/<tenant>.yaml`) |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation on the K8s Secret |
| `data_source` | Always `insight_claude_admin` |
| `collected_at` | UTC ISO-8601 timestamp at extraction time |
| `unique` / `id` | Primary key; varies per stream (see Streams below) |

### Local development

```bash
cp src/ingestion/secrets/connectors/claude-admin.yaml.example src/ingestion/secrets/connectors/claude-admin.yaml
# Edit the .yaml with the real admin API key, then apply:
kubectl apply -f src/ingestion/secrets/connectors/claude-admin.yaml
```

## Streams

| Stream | Endpoint | Sync Mode | Cursor | Step | Pagination |
|--------|----------|-----------|--------|------|-----------|
| `claude_admin_users` | `GET /v1/organizations/users` | Full refresh | — | — | Cursor (`after_id` token) |
| `claude_admin_messages_usage` | `GET /v1/organizations/usage_report/messages?group_by[]=model&group_by[]=api_key_id&group_by[]=workspace_id&group_by[]=service_tier&group_by[]=context_window` | Incremental | `date` | P1D | Cursor (`next_page`) |
| `claude_admin_cost_report` | `GET /v1/organizations/cost_report?group_by[]=workspace_id&group_by[]=description` | Incremental | `date` | P1D | Cursor (`next_page`) |
| `claude_admin_code_usage` | `GET /v1/organizations/usage_report/claude_code` | Incremental | `date` | P1D | Cursor (`next_page`) |
| `claude_admin_api_keys` | `GET /v1/organizations/api_keys` | Full refresh | — | — | Offset (`limit` + `offset`) |
| `claude_admin_workspaces` | `GET /v1/organizations/workspaces` | Full refresh | — | — | Offset |
| `claude_admin_workspace_members` | `GET /v1/organizations/workspaces/{id}/members` (substream) | Full refresh | — | — | None (iterated per workspace) |
| `claude_admin_invites` | `GET /v1/organizations/invites` | Full refresh | — | — | Offset |

A ninth Bronze table — `claude_admin_collection_runs` — is produced by the orchestrator (one row per pipeline run), not by Airbyte. The manifest does not define it as a stream.

### Identity Keys

- `claude_admin_users.email` — primary identity key (one row per seat)
- `claude_admin_code_usage.actor_identifier` where `actor_type = 'user'` — secondary identity key (daily code usage per user)

Other streams carry organization-level dimension data:
- `claude_admin_api_keys.created_by_id` / `created_by_name` — for API-key-creator attribution (resolved to `person_id` via Identity Manager when `created_by_type = 'user'`)
- `claude_admin_invites.email` — for invitee resolution
- `claude_admin_workspace_members.user_id` — joins to `claude_admin_users.id` for workspace-level membership

## Silver Targets

Two Silver models ship with this connector and run under the `tag:claude-admin` dbt selector:

- `claude_admin__ai_api_usage` — feeds `class_ai_api_usage` (programmatic API consumption: tokens, costs, per-key and per-workspace attribution). Source: `claude_admin_messages_usage` joined with `claude_admin_api_keys` and `claude_admin_workspaces` for dimension enrichment.
- `claude_admin__ai_dev_usage` — feeds `class_ai_dev_usage` (Claude Code developer usage alongside Cursor/Windsurf). Source: `claude_admin_code_usage` filtered to `actor_type = 'user'` with `actor_identifier` treated as email.

Silver-level `silver:class_*` tags will be added in a separate PR; this connector currently routes only via the `claude-admin` dbt tag.

## Operational Constraints

- **Rate limits**: organization-level, enforced by the Anthropic Admin API. The connector follows `Retry-After` on HTTP 429 and retries transient 5xx with exponential backoff.
- **31-day window**: usage/cost endpoints cap date ranges at 31 days per request. The connector steps at `P1D` (one day per request) to avoid boundary-day loss caused by Airbyte's inclusive-inclusive cursor arithmetic.
- **`cursor_granularity: PT1S`**: applied on incremental streams to prevent empty date-boundary windows (`starting_at == ending_at`) that the API rejects with HTTP 400. See the historical `claude-api` ADRs for background.
- **No 3-day reporting lag**: unlike the Enterprise Analytics API (`claude-enterprise`), the Admin API makes day `D` data queryable the same day it is aggregated. This connector's default `start_date` is 90 days ago (not 14).
- **Substream for workspace members**: the members stream iterates over every workspace ID from the parent stream. Large organizations with many workspaces may see proportional sync time.

## Validation

```bash
cypilot validate --artifact docs/components/connectors/ai/claude-admin/specs/PRD.md
cypilot validate --artifact docs/components/connectors/ai/claude-admin/specs/DESIGN.md
```

## Related

- Sibling connector (different API): `claude-enterprise` — Anthropic Enterprise Analytics API for DAU/WAU/MAU, chat project engagement, skill and connector adoption. Complementary to this connector: admin data (tokens, costs, seats) vs engagement data (per-user activity, active-user summaries).

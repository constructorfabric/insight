# Zendesk Connector

Extracts Zendesk tickets, satisfaction ratings, and agent directory into the Bronze layer.

**API**: Zendesk REST API v2 (`https://{subdomain}.zendesk.com/api/v2/`)

**Auth model**: HTTP Basic Auth — `{email}/token:{api_token}` Base64-encoded. Token created under Admin → Apps & Integrations → Zendesk API.

## Prerequisites

1. API token access is enabled (Admin Center → Apps and integrations → APIs → Zendesk API → Settings → Token access: ON). Only an administrator can create a token.
2. `zendesk_email` is the address of an **administrator**. API tokens carry no scopes of their own: the paired email decides the permissions, and both the incremental ticket export and the satisfaction-ratings list are admin-only. `check` exercises both, so a non-admin email fails at `check` rather than three streams into the first sync.
3. CSAT ratings come from the legacy CSAT endpoint. An account on Zendesk's newer survey-based CSAT, or with CSAT disabled, gets an empty ratings stream — not an error.

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-zendesk-main
  namespace: insight
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: zendesk
    insight.cyberfabric.com/source-id: zendesk-main
type: Opaque
stringData:
  zendesk_subdomain: "<your-subdomain>"          # e.g. "acme" for acme.zendesk.com
  zendesk_email:     "<service-account@example.com>"
  zendesk_api_token: "<api-token>"
  # start_date: "2024-01-01"                     # optional; default: 90 days ago (YYYY-MM-DD)
```

> **Multiple Zendesk instances**: change `name` and `source-id` annotation accordingly, e.g. `insight-zendesk-staging` / `zendesk-staging`.

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `zendesk_subdomain` | Yes | Your Zendesk subdomain (the `{subdomain}` part of `{subdomain}.zendesk.com`). Used to build all API URLs. |
| `zendesk_email` | Yes | Email of the Zendesk user associated with the API token. Used as the Basic Auth username with `/token` suffix. |
| `zendesk_api_token` | Yes | API token generated under Admin → Apps & Integrations → Zendesk API. Sent as the Basic Auth password. |
| `start_date` | No | Earliest date for historical backfill (YYYY-MM-DD). Default: 90 days ago. First run fetches all tickets and ratings updated since this date. |

### Automatically injected

These fields are added to every record by the connector — do **not** put them in the K8s Secret:

| Field | Source |
|-------|--------|
| `tenant_id` | `insight_tenant_id` from tenant YAML (`connections/<tenant>.yaml`) |
| `source_id` | `insight.cyberfabric.com/source-id` annotation on the K8s Secret |
| `unique_key` | Composite primary key (varies per stream — see Streams below) |
| `data_source` | Always `insight_zendesk` |
| `collected_at` | UTC ISO-8601 timestamp at extraction time |

## Streams

| Stream | Endpoint | Sync Mode | Cursor | `unique_key` |
|--------|----------|-----------|--------|-------------|
| `support_tickets` | `GET /api/v2/incremental/tickets/cursor.json?include=metric_sets` | Incremental (lookback P1D) | `updated_at` (Unix ts) | `{tenant}-{source}-{ticket_id}` |
| `support_ticket_ids` | `GET /api/v2/incremental/tickets/cursor.json` (id + updated_at only) | Incremental | `updated_at` (Unix ts) | `{tenant}-{source}-{ticket_id}` |
| `support_agents` | `GET /api/v2/users?role[]=agent&role[]=admin` | Full refresh | — | `{tenant}-{source}-{agent_id}` |
| `zendesk_satisfaction_ratings` | `GET /api/v2/satisfaction_ratings` | Incremental (lookback P1D) | `created_at` (Unix ts) | `{tenant}-{source}-{rating_id}` |
| `support_ticket_events` | `GET /api/v2/tickets/{id}/audits` | Incremental (substream over `support_ticket_ids`, `incremental_dependency`) | `created_at` (formal — see below) | `{tenant}-{source}-{audit_id}` |

Pagination: the two ticket streams use Zendesk's **cursor-based** incremental
export (`cursor` parameter; the response's `after_url` becomes the next request
path and `end_of_stream` stops it); `support_agents` and
`zendesk_satisfaction_ratings` use Zendesk **cursor pagination** (`page[size]`
/ `page[after]`, `meta.after_cursor`; `meta.has_more` ends the read only as an
explicit false — otherwise an empty page or a missing cursor does). Neither
offset paging (`per_page` / `next_page`) nor the time-based export is used:
mixing either with a cursor-shaped paginator yields an empty token and ends
the read after one page without an error.

Rate limits: every `/api/v2/incremental/*` call shares one budget of 10
requests per minute per account (30 with the High Volume add-on), so the two
ticket streams draw on it together. The per-ticket audits requests count
against the account-wide limit instead.

### Notes

- **`support_tickets`**: uses Zendesk's cursor-based incremental export endpoint (1000 tickets/page). Sideloads `metric_sets` to retrieve timing fields without extra API calls. Both business-hours and calendar-hours timing variants are stored (`first_reply_time_seconds` / `first_reply_time_calendar_seconds` and `full_resolution_time_seconds` / `full_resolution_time_calendar_seconds`). The export's `support_type_scope` is left at its default `agent`, so tickets handled by AI agents are not exported.
- **`support_agents`**: full refresh on every run — agent roster is small and Zendesk does not expose a reliable incremental endpoint for users. `group_name` is NULL in Phase 1 (group enrichment deferred); `is_active` is stored as an int and is 1 only for a user who is both `active` and not `suspended`; deactivated agents leave the listing entirely, so their last row keeps `is_active = 1` until a silver-side absence check lands.
- **`support_ticket_ids`**: slim incremental parent (id + `updated_at` only, no metadata) that drives the `support_ticket_events` SubstreamPartitionRouter. Kept separate from `support_tickets` so the audit fan-out only re-fetches tickets whose `updated_at` advanced.
- **`zendesk_satisfaction_ratings`**: CSAT ratings stored as a separate stream preserving full history. `support_tickets.satisfaction_score` is NULL in Phase 1; Silver layer derives per-ticket CSAT from this stream. Incremental by `created_at`, the axis the endpoint's `start_time` filter bounds: a rating edited after it left the one-day lookback keeps its first-seen score. Legacy CSAT endpoint — see Prerequisites.
- **`support_ticket_events`**: per-ticket audit log from `GET /api/v2/tickets/{id}/audits`, fanned out over `support_ticket_ids` with `incremental_dependency` (concurrency_level=4). A 404 (the audits of a deleted ticket, which the export still lists) is ignored and 429/503 are retried after `Retry-After`, so a single bad ticket does not fail the sync. Its `created_at` cursor is *formal*: the endpoint takes no time filter and nothing is dropped client-side. It exists so the stream is stateful, which is the only thing that lets `incremental_dependency` persist the parent's cursor — without it every sync re-enumerates the whole `start_date` window, one request per ticket.
- **`zendesk_ticket_ext`** (Phase 2, deferred): custom field key-value pairs from `ticket.custom_fields[]`.

## Silver targets

The `dbt/` models tagged `zendesk` populate:
- `staging.zendesk__support_event` — event-grain, actor-attributed audit facts (an intermediate of this connector; `updates` are counted as distinct audits, not distinct field changes)
- `staging.zendesk__support_activity` — per-person per-day metrics (updates / public_comments / private_comments / solved [distinct tickets] / csat_good / csat_total / kb [honest-NULL])
- `silver.class_support_activity` — the cross-vendor support class relation
- `silver.dim_support_agent`, `silver.dim_support_ticket` — support dimensions

**There is no Gold layer for support.** Nothing reads `class_support_activity`,
so no support metric reaches the product yet: the domain was built against the
`*_bullet_rows` gold contract, which was removed when metrics moved to the
unified `*_metric_observations` / `*_metric_evidence` relations. Support is the
one domain that was not ported across.

`dbt_select` in `descriptor.yaml` is scoped to `tag:zendesk+` — it selects the `zendesk`-tagged models and their downstream silver unions while keeping the run from touching other connectors' models.

## Validation

```bash
./src/ingestion/tools/declarative-connector/source.sh validate-strict support/zendesk
./src/ingestion/tools/declarative-connector/source.sh validate        support/zendesk
```

Mock tests (no credentials needed):

```bash
cd src/ingestion/tests/connectors
.venv/bin/pytest ../../connectors/support/zendesk/tests
```

## Related

- The `support_*` bronze tables and the `silver.class_support_activity` contract are cross-vendor by design; a second support connector would land beside this one rather than replace it. None exists today.

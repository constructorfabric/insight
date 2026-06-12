# Zendesk Connector

Extracts Zendesk tickets, satisfaction ratings, and agent directory into the Bronze layer.

**API**: Zendesk REST API v2 (`https://{subdomain}.zendesk.com/api/v2/`)

**Auth model**: HTTP Basic Auth — `{email}/token:{api_token}` Base64-encoded. Token created under Admin → Apps & Integrations → Zendesk API.

## Specification

- **Full spec + Bronze schemas**: [`docs/components/connectors/support/zendesk/zendesk.md`](../../../../../docs/components/connectors/support/zendesk/zendesk.md)
- **PRD**: [`docs/components/connectors/support/zendesk/specs/PRD.md`](../../../../../docs/components/connectors/support/zendesk/specs/PRD.md)
- **DESIGN**: [`docs/components/connectors/support/zendesk/specs/DESIGN.md`](../../../../../docs/components/connectors/support/zendesk/specs/DESIGN.md)

## Prerequisites

1. The customer has a Zendesk account with API token access enabled (Admin → Apps & Integrations → Zendesk API → Token Access: ON).
2. An API token has been created for a service account with `tickets:read`, `users:read`, and `satisfaction_ratings:read` permissions.
3. The customer is on a Zendesk Suite plan or equivalent that supports the incremental export endpoint (`/api/v2/incremental/tickets.json`).

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
| `support_tickets` | `GET /api/v2/incremental/tickets.json?include=metric_sets` | Incremental | `updated_at` (Unix ts) | `{tenant}-{source}-{ticket_id}` |
| `support_agents` | `GET /api/v2/users?role[]=agent&role[]=admin` | Full refresh | — | `{tenant}-{source}-{agent_id}` |
| `zendesk_satisfaction_ratings` | `GET /api/v2/satisfaction_ratings` | Incremental | `updated_at` (Unix ts) | `{tenant}-{source}-{rating_id}` |

### Notes

- **`support_tickets`**: uses Zendesk's incremental export endpoint (1000 tickets/page). Sideloads `metric_sets` to retrieve timing fields without extra API calls. Both business-hours and calendar-hours timing variants are stored (`first_reply_time_seconds` / `first_reply_time_calendar_seconds` and `full_resolution_time_seconds` / `full_resolution_time_calendar_seconds`).
- **`support_agents`**: full refresh on every run — agent roster is small and Zendesk does not expose a reliable incremental endpoint for users. Group names resolved via a startup call to `GET /api/v2/groups`.
- **`zendesk_satisfaction_ratings`**: CSAT ratings stored as a separate stream preserving full history. `support_tickets.satisfaction_score` is NULL in Phase 1; Silver layer derives per-ticket CSAT from this stream.
- **`support_ticket_events`** (Phase 2): per-ticket audit log from `GET /api/v2/tickets/{id}/audits`. Deferred — requires one API call per ticket, expensive for large accounts.
- **`zendesk_ticket_ext`** (Phase 2): custom field key-value pairs from `ticket.custom_fields[]`.

## Silver Targets

Silver transformations for Phase 1 are planned as `dbt/` models tagged `zendesk`. When implemented, they will populate:
- `staging.zendesk__support_activity` — per-agent per-day metrics
- `silver.class_support_activity` — unified support domain Silver table (Zendesk + JSM)

`dbt_select` in `descriptor.yaml` is scoped to `tag:zendesk+` — it selects nothing until Phase 2 Silver models are tagged `zendesk`, while keeping the Silver run from touching other connectors' models.

## Validation

```bash
./src/ingestion/tools/declarative-connector/source.sh validate-strict support/zendesk
./src/ingestion/tools/declarative-connector/source.sh validate        support/zendesk
```

## Related

- `jsm` — Jira Service Management connector; same unified `support_*` Bronze tables with `data_source = "insight_jsm"`. JSM also collects `support_ticket_events` and `support_sla` (Phase 2 for Zendesk).
- Support domain spec: `docs/components/connectors/support/README.md` — unified Bronze schema across Zendesk and JSM.

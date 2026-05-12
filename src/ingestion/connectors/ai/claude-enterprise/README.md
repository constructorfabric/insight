# Claude Enterprise Connector

Extracts organization-wide engagement analytics (per-user activity, DAU/WAU/MAU summaries, chat project usage, skill and connector adoption) from the Anthropic Enterprise Analytics API into the Bronze layer.

Authentication: API key with `read:analytics` scope, sent via `x-api-key` header.

## Specification

- **PRD**: [../../../../../docs/components/connectors/ai/claude-enterprise/specs/PRD.md](../../../../../docs/components/connectors/ai/claude-enterprise/specs/PRD.md)
- **DESIGN**: [../../../../../docs/components/connectors/ai/claude-enterprise/specs/DESIGN.md](../../../../../docs/components/connectors/ai/claude-enterprise/specs/DESIGN.md)

## Prerequisites

1. The deploying organization must be on Claude Enterprise plan with the Enterprise Analytics API enabled.
2. An organization Primary Owner creates an API key at [claude.ai/analytics/api-keys](https://claude.ai/analytics/api-keys) with the `read:analytics` scope.
3. The API has a **3-day reporting lag** — data for day `D` is queryable starting on day `D + 4`.
4. The API rejects dates **earlier than 2026-01-01** with HTTP 400 (the connector silently clamps any earlier `start_date`).

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-claude-enterprise-main
  namespace: insight
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: claude-enterprise
    insight.cyberfabric.com/source-id: claude-enterprise-main
type: Opaque
stringData:
  analytics_api_key: "<your-key>"
  # start_date: "2026-01-01"   # optional, default = 14 days ago
  # base_url:   "http://stub.dev.svc.cluster.local:8080"   # optional, default = https://api.anthropic.com
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `analytics_api_key` | Yes | Enterprise Analytics API key (`read:analytics` scope). Marked `airbyte_secret: true` — never logged. |
| `start_date` | No | Earliest date to collect (YYYY-MM-DD). Default: 14 days ago. Clamped to 2026-01-01. |
| `base_url` | No | API base URL override. Default: `https://api.anthropic.com`. Use only for local development against a stub. |

### Automatically injected

These fields are added to every record by the connector — do **not** put them in the K8s Secret:

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML (`connections/<tenant>.yaml`) |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation on the K8s Secret |
| `tenant_id` | Mirror of `insight_tenant_id` (Bronze convention) |
| `source_id` | Mirror of `insight_source_id` (Bronze convention) |
| `data_source` | Always `insight_claude_enterprise` |
| `collected_at` | UTC ISO-8601 timestamp at extraction time |
| `unique_key` | Composite primary key (varies per stream) |

### Local development

```bash
cp src/ingestion/secrets/connectors/claude-enterprise.yaml.example src/ingestion/secrets/connectors/claude-enterprise.yaml
# Edit the .yaml with the real API key, then apply:
kubectl apply -f src/ingestion/secrets/connectors/claude-enterprise.yaml
```

If you don't have a real Enterprise key, set `base_url` to a local stub service that mirrors the Enterprise Analytics API contracts. The stub is a separate dev tool and not part of this connector package.

## Streams

| Stream | Endpoint | Sync Mode | Cursor | Step | Pagination |
|--------|----------|-----------|--------|------|-----------|
| `claude_enterprise_summaries` | `GET /v1/organizations/analytics/summaries?starting_date=…` | Incremental | `date` | P1D | None (one day per request) |
| `claude_enterprise_users` | `GET /v1/organizations/analytics/users?date=…` | Incremental | `date` | P1D | Cursor (`page` token) |
| `claude_enterprise_chat_projects` | `GET /v1/organizations/analytics/apps/chat/projects?date=…` | Incremental | `date` | P1D | Cursor |
| `claude_enterprise_skills` | `GET /v1/organizations/analytics/skills?date=…` | Incremental | `date` | P1D | Cursor |
| `claude_enterprise_connectors` | `GET /v1/organizations/analytics/connectors?date=…` | Incremental | `date` | P1D | Cursor |

A sixth Bronze table — `claude_enterprise_collection_runs` — is documented in [DESIGN §3.7](../../../../../docs/components/connectors/ai/claude-enterprise/specs/DESIGN.md#37-database-schemas--tables) but is produced by the orchestrator (one row per pipeline run), not by Airbyte. The manifest does not define it as a stream.

### Identity Keys

- `claude_enterprise_users.user_email` — primary identity key
- `claude_enterprise_chat_projects.created_by_email` — secondary identity key (project ownership)

The other streams (`summaries`, `skills`, `connectors`) are pre-aggregated by the API and carry no per-user identity.

## Silver Targets

Routing is tag-driven via `dbt_select: "tag:claude-enterprise+"` in `descriptor.yaml`.

| Staging model | Silver class | Tag |
|---|---|---|
| `claude_enterprise__ai_dev_usage` | `class_ai_dev_usage` | `silver:class_ai_dev_usage` |
| `claude_enterprise__ai_assistant_usage` | `class_ai_assistant_usage` | `silver:class_ai_assistant_usage` |

## Operational Constraints

- **Rate limits**: organization-level, default values not documented; adjustable via Anthropic CSM. The connector honours `Retry-After` on HTTP 429 with exponential backoff, and retries on 5xx.
- **Reporting lag**: 3 days. The connector's effective upper bound is `today − 3 days`; later dates are deferred to the next run.
- **Minimum date**: 2026-01-01. Earlier `start_date` values are silently clamped.
- **`start_date` edge case**: if `start_date` is within the 3-day lag window (e.g. yesterday), the cursor window is empty (`start > end`). Airbyte's `DatetimeBasedCursor` skips empty windows silently — no error, zero records. Set `start_date` to at least 4 days in the past to guarantee data on first run.
- **`/summaries`**: uses P1D step with `starting_date` only (no `ending_date`). The API supports 31-day ranges, but `ending_date` is exclusive — P31D would silently skip boundary days. One request per day is safe and fast (tiny payload).
- **Concurrency**: `default_concurrency: 1` (sequential streams). Intentional to avoid rate-limit exhaustion during backfill; can be bumped to 2-3 after observing real API behavior.

## Validation

```bash
cypilot validate --artifact docs/components/connectors/ai/claude-enterprise/specs/PRD.md
cypilot validate --artifact docs/components/connectors/ai/claude-enterprise/specs/DESIGN.md
```

## Related

- Sibling connector (different API): `claude-admin` — Anthropic Admin API for organization metadata, token usage, cost reports, Claude Code usage, API keys, workspaces, and invites (merged from the former `claude-api` and `claude-team` connectors per [cyberfabric/insight#141](https://github.com/cyberfabric/insight/issues/141))

# Claude Team Connector

Extracts claude.ai Team plan data (organization roster, pending invites, overage spend, and per-user Claude Code metrics) into the Bronze layer.

**Authentication model**: this connector talks to a **customer-deployed proxy** (not to claude.ai directly). The proxy holds the claude.ai sessionKey cookie inside the customer's environment; Insight authenticates to the proxy with a shared bearer token. The session cookie never enters Insight infrastructure.

Proxy source code, Dockerfile, and deployment docs:
**[gitlab.constr.dev/insight/secure-enclave → proxies/claude_team/](https://gitlab.constr.dev/insight/secure-enclave)**

## Specification

- **PRD**: [../../../../../docs/components/connectors/ai/claude-team/specs/PRD.md](../../../../../docs/components/connectors/ai/claude-team/specs/PRD.md)
- **DESIGN**: [../../../../../docs/components/connectors/ai/claude-team/specs/DESIGN.md](../../../../../docs/components/connectors/ai/claude-team/specs/DESIGN.md)
- **FEATURE**: [../../../../../docs/components/connectors/ai/claude-team/specs/FEATURE.md](../../../../../docs/components/connectors/ai/claude-team/specs/FEATURE.md)

## Prerequisites

1. The customer is on a **claude.ai Team plan** with an active organization.
2. The customer has deployed the **claude-team-proxy** Docker container from `secure-enclave/proxies/claude_team/` on their infrastructure and exposed it over the network (typically behind TLS + reverse proxy).
3. The customer has installed a valid sessionKey into the proxy via `POST /admin/session-key`. The proxy returns 503 from `/api/*` until this is done.
4. Insight and the customer have exchanged the bearer token (`proxy_auth_token`) out-of-band.
5. To collect `claude_team_code_metrics`, the account associated with the sessionKey must have access to Claude Code usage metrics within the org.
6. To collect `claude_team_overage_spend`, the sessionKey must have `billing:view` permission. If absent, the stream is silently skipped (sync stays GREEN, zero records).

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-claude-team-main
  namespace: insight
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: claude-team
    insight.cyberfabric.com/source-id: claude-team-main
type: Opaque
stringData:
  claude_org_id:    "<uuid-from-claude.ai>"
  proxy_url:        "https://claude-team-proxy.customer.example.com"
  proxy_auth_token: "<shared-bearer-token>"
  # start_date: "2025-11-24"  # optional; earliest code_metrics backfill date (YYYY-MM-DD)
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `claude_org_id` | Yes | UUID of the claude.ai organisation. Find via DevTools console: `fetch('/api/organizations').then(r=>r.json()).then(console.table)` |
| `proxy_url` | Yes | Base URL of the customer-deployed claude-team-proxy. No default — must be set per installation. |
| `proxy_auth_token` | Yes | Bearer token sent as `Authorization: Bearer <token>` on every request to the proxy. Must match the proxy's `PROXY_AUTH_TOKEN` env var. |
| `start_date` | No | Earliest date for `claude_team_code_metrics` backfill (YYYY-MM-DD). Default: 7 days ago. Absolute earliest: `2025-11-24`. Has no effect on the three snapshot streams. |

> **The claude.ai sessionKey is NOT in this Secret.** It lives only on the customer's proxy container. Cookie rotation is the customer's responsibility — Insight never sees it.

### Automatically injected

These fields are added to every record by the connector — do **not** put them in the K8s Secret:

| Field | Source |
|-------|--------|
| `tenant_id` | `insight_tenant_id` from tenant YAML (`connections/<tenant>.yaml`) |
| `source_id` | `insight.cyberfabric.com/source-id` annotation on the K8s Secret |
| `unique_key` | Composite primary key (varies per stream — see Streams below) |
| `data_source` | Always `insight_claude_team` |
| `collected_at` | UTC ISO-8601 timestamp at extraction time |

## Streams

| Stream | Endpoint (via proxy) | Sync Mode | Cursor | Step | Pagination | unique_key |
|--------|----------------------|-----------|--------|------|-----------|------------|
| `claude_team_members` | `GET /api/organizations/{org}/members` | Full refresh | — | — | None (plain array) | `{tenant}-{source}-{account.uuid}` |
| `claude_team_invites` | `GET /api/organizations/{org}/invites` | Full refresh | — | — | None (plain array) | `{tenant}-{source}-{uuid}` |
| `claude_team_overage_spend` | `GET /api/organizations/{org}/overage_spend_limits` | Full refresh | — | — | PageIncrement (100/page) | `{tenant}-{source}-{account_uuid}` |
| `claude_team_code_metrics` | `GET /api/claude_code/metrics_aggs/users` | Incremental | `metric_date` | P1D | OffsetIncrement (100/page) | `{tenant}-{source}-{date}-{email}` |

### Notes

- **`claude_team_members` / `claude_team_invites`**: full snapshot — only the current state is returned. Historical invite events (accepted/expired) are not recoverable from this endpoint.
- **`claude_team_overage_spend`**: requires `billing:view` permission on the sessionKey. Returns HTTP 403 if absent; the error handler marks the stream as empty and continues the sync.
- **`claude_team_code_metrics`**: one API request per day in the backfill window (P1D step). The endpoint is the most expensive (~3–13 s per page due to API-side aggregation). The `metric_date` field is injected by the connector — the API omits it from per-user objects.
- **Hard floor `2025-11-24`**: the earliest date for which data exists in the reference org. Going earlier returns empty pages. Operators with older data can override via `start_date`.

## Silver Targets

Silver transformations are out of scope for this MVP (Phase 6+). `dbt_select` in `descriptor.yaml` is intentionally empty. Once Silver models land they will be tagged `claude-team` and selected via `tag:claude-team+`.

## Operational Constraints

- **sessionKey expiry**: when the cookie expires (~30–90 days, no published TTL), the proxy `/api/*` calls start returning 502 with "transport not ready". The customer rotates the cookie via `POST /admin/session-key` on the proxy — no change required on the Insight side.
- **Cloudflare challenge**: the proxy solves the CF challenge during `setSessionKey`, not at startup. First call after a key rotation may take up to 60 s. Insight retries on transient 503/502.
- **Token rotation**: `proxy_auth_token` rotation requires updating both the K8s Secret here and the `PROXY_AUTH_TOKEN` env on the proxy container. Coordinate via the customer.
- **One proxy = one org**: the proxy container is bound to a single `CLAUDE_ORG_ID`. Multiple claude organisations require multiple proxy deployments and multiple Insight connector instances.

## Validation

```bash
./src/ingestion/tools/declarative-connector/source.sh validate-strict ai/claude-team
./src/ingestion/tools/declarative-connector/source.sh validate        ai/claude-team
```

## Related

- `claude-admin` — Anthropic Admin API connector for organization metadata, token usage, cost reports, Claude Code usage via the programmatic API. Complementary to this connector: `claude-admin` covers the API-facing side; `claude-team` covers the claude.ai web UI side (Team plan roster + Code metrics for web-UI users).
- `claude-enterprise` — Anthropic Enterprise Analytics API for DAU/WAU/MAU summaries and engagement analytics.

# Claude Team Connector

Extracts claude.ai Team plan data (organization roster, pending invites, overage spend, and per-user Claude Code metrics) into the Bronze layer.

Authentication: sessionKey browser cookie, managed by the in-cluster **claude-team-proxy** service. The connector itself carries no credentials — it talks to the proxy over plain HTTP via in-cluster DNS. See `src/backend/services/claude-team-proxy/` for the proxy implementation.

## Specification

- **PRD**: [../../../../../docs/components/connectors/ai/claude-team/specs/PRD.md](../../../../../docs/components/connectors/ai/claude-team/specs/PRD.md)
- **DESIGN**: [../../../../../docs/components/connectors/ai/claude-team/specs/DESIGN.md](../../../../../docs/components/connectors/ai/claude-team/specs/DESIGN.md)
- **FEATURE**: [../../../../../docs/components/connectors/ai/claude-team/specs/FEATURE.md](../../../../../docs/components/connectors/ai/claude-team/specs/FEATURE.md)

## Prerequisites

1. The deploying organization must be on a **claude.ai Team plan** with an active organization.
2. A browser sessionKey must be extracted from claude.ai (DevTools → Application → Cookies → `sessionKey`) and loaded into the **claude-team-proxy** Helm release (see proxy README). The connector itself never sees the sessionKey.
3. The `claude-team-proxy` service must be deployed and reachable at `claude-team-proxy.insight.svc.cluster.local:3000` (default) before the first Airbyte sync.
4. To collect `claude_team_code_metrics`, the account associated with the sessionKey must have access to Claude Code usage metrics within the org.
5. To collect `claude_team_overage_spend`, the sessionKey must have `billing:view` permission. If absent, the stream is silently skipped (sync stays GREEN, zero records).

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
  claude_org_id: "<uuid-from-claude.ai>"
  # proxy_url: "http://claude-team-proxy.insight.svc.cluster.local:3000"  # optional, default shown
  # start_date: "2025-11-24"  # optional; earliest code_metrics backfill date (YYYY-MM-DD)
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `claude_org_id` | Yes | UUID of the claude.ai organisation. Find via DevTools console: `fetch('/api/organizations').then(r=>r.json()).then(console.table)` |
| `proxy_url` | No | Base URL of the claude-team-proxy. Default: `http://claude-team-proxy.insight.svc.cluster.local:3000`. Override only for local development. |
| `start_date` | No | Earliest date for `claude_team_code_metrics` backfill (YYYY-MM-DD). Default: 7 days ago. Absolute earliest: `2025-11-24`. Has no effect on the three snapshot streams. |

> **sessionKey is NOT in this Secret.** Authentication is handled entirely by the `claude-team-proxy` Helm release. Rotate the sessionKey via:
> ```bash
> helm upgrade claude-team-proxy ./src/backend/services/claude-team-proxy/helm \
>   --namespace insight --reuse-values \
>   --set-string sessionKey="<new-sessionKey-from-DevTools>"
> ```

### Automatically injected

These fields are added to every record by the connector — do **not** put them in the K8s Secret:

| Field | Source |
|-------|--------|
| `tenant_id` | `insight_tenant_id` from tenant YAML (`connections/<tenant>.yaml`) |
| `source_id` | `insight.cyberfabric.com/source-id` annotation on the K8s Secret |
| `unique_key` | Composite primary key (varies per stream — see Streams below) |
| `data_source` | Always `insight_claude_team` |
| `collected_at` | UTC ISO-8601 timestamp at extraction time |

### Local development

```bash
cp src/ingestion/secrets/connectors/claude-team.yaml.example src/ingestion/secrets/connectors/claude-team.yaml
# Edit with the real claude_org_id, then apply:
kubectl apply -f src/ingestion/secrets/connectors/claude-team.yaml
```

For local development without a running cluster, set `proxy_url` to a locally-running instance of the proxy (e.g. `http://localhost:3000`). Start the proxy locally with:

```bash
cd src/backend/services/claude-team-proxy
SESSION_KEY="<your-sessionKey>" node src/index.js
```

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

- **sessionKey expiry**: when the cookie expires, the proxy starts returning HTTP 401 for all `/api/*` requests. The connector sync fails (Airbyte marks it RED), alerting operators. Rotate the sessionKey via `helm upgrade --set-string sessionKey=...` against the proxy Helm release.
- **Cloudflare challenge**: the proxy boots a headless Chromium to solve the initial CF challenge. Boot time is up to 60 s; the Helm liveness probe uses `initialDelaySeconds: 60` to avoid false restarts. During high-traffic periods CF challenges can take longer — the proxy will exit and be restarted by K8s.
- **Single-replica proxy**: by design (DESIGN §2.2). The proxy holds a single browser session and is not horizontally scalable. Sync cadence is daily at 04:00 UTC to avoid overlap with other Insight connectors.
- **No CI/CD entry for proxy image**: the proxy Docker image is built and pushed manually for MVP. A follow-up issue tracks CI automation.

## Validation

```bash
./src/ingestion/tools/declarative-connector/source.sh validate-strict ai/claude-team
./src/ingestion/tools/declarative-connector/source.sh validate        ai/claude-team
```

## Related

- `claude-admin` — Anthropic Admin API connector for organization metadata, token usage, cost reports, Claude Code usage via the programmatic API. Complementary to this connector: `claude-admin` covers the API-facing side; `claude-team` covers the claude.ai web UI side (Team plan roster + Code metrics for web-UI users).
- `claude-enterprise` — Anthropic Enterprise Analytics API for DAU/WAU/MAU summaries and engagement analytics.

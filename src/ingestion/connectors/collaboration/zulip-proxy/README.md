# Zulip-Proxy Connector

Aggregated Zulip activity collected through a tenant-controlled proxy with Bearer-token auth.
Same Bronze shape as the direct-API [zulip](../../../../../docs/components/connectors/collaboration/zulip/zulip.md)
connector spec, but reached through a private proxy endpoint instead of `*.zulipchat.com`.

See the [PRD](../../../../../docs/components/connectors/collaboration/zulip-proxy/specs/PRD.md),
[DESIGN](../../../../../docs/components/connectors/collaboration/zulip-proxy/specs/DESIGN.md), and
[FEATURE](../../../../../docs/components/connectors/collaboration/zulip-proxy/specs/FEATURE.md)
for full context.

## Prerequisites

1. A reachable Zulip-proxy instance inside the tenant network exposing
   `GET {base_url}/users` and `GET {base_url}/messages`.
2. A Bearer token issued by the proxy operator. The token MUST grant read access to both
   endpoints; it MUST NOT be the upstream Zulip bot/API key — the proxy is the only component
   that holds Zulip primary credentials.

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-zulip-proxy-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: zulip-proxy
    insight.cyberfabric.com/source-id: zulip-proxy-main
type: Opaque
stringData:
  zulip_proxy_base_url: ""             # e.g. http://10.0.0.1:9999/api
  zulip_proxy_api_key: ""              # Bearer token
  zulip_proxy_start_date: "2024-01-01" # earliest created_at for first sync
  zulip_proxy_throttle_ms: "5000"      # optional, default 5000
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `zulip_proxy_base_url` | Yes | Root URL of the proxy (`scheme://host[:port]/api`, no trailing slash). |
| `zulip_proxy_api_key` | Yes | Opaque Bearer token. `airbyte_secret: true` in the manifest. |
| `zulip_proxy_start_date` | Yes | ISO date (`YYYY-MM-DD`) — backfill anchor for the first sync only. |
| `zulip_proxy_throttle_ms` | No (default 5000) | Server-side pacing hint forwarded as the `throttle` query param on `/messages`. |

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

```bash
cp src/ingestion/secrets/connectors/zulip-proxy.yaml.example \
   src/ingestion/secrets/connectors/zulip-proxy.yaml
# Fill in real values, then:
kubectl apply -f src/ingestion/secrets/connectors/zulip-proxy.yaml
```

## Streams

| Stream | Description | Sync Mode | Cursor |
|--------|-------------|-----------|--------|
| `users` | Zulip realm user directory (id, uuid, email, full_name, role, is_active, recipient_id). | full_refresh | — |
| `messages` | Aggregated per-sender message counts per bucket (uniq, sender_id, count, created_at). Message bodies are NEVER collected. | incremental | `created_at` |

## Silver Targets

- `staging.zulip_proxy__bronze_promoted` — RMT promotion bootstrap (one-shot per bronze table).
- `staging.zulip_proxy__users_snapshot` — SCD2 snapshot of the user directory.
- `staging.zulip_proxy__users_fields_history` — field-level history for SCD2 columns.
- `staging.zulip_proxy__identity_inputs` — feeds Identity Manager (email → person).
- `staging.zulip_proxy__collab_chat_activity` — `class_collab_chat_activity` daily roll-up
  per (`tenant_id`, `source_id`, `email`, `date`).

## Operational notes

- A 401 response from the proxy fails the run with an operator-actionable log line that names
  `connector=zulip-proxy` and the offending `source_id`. Action: rotate the Bearer token in the
  K8s Secret and re-run.
- 429 and 503 responses are honored with `Retry-After`; 5xx are retried with exponential backoff
  up to 5 attempts.
- The connector holds no Zulip primary credentials. The proxy is the trust boundary.

# Cursor Connector

Cursor IDE team usage data: members, audit logs, usage events, and daily usage metrics.

## Prerequisites

1. Have a Cursor Team plan
2. Generate a team API key from Cursor Settings > Team > API

## K8s Secret

Create a Kubernetes Secret with the connector credentials:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-cursor-main                          # convention: insight-{connector}-{source-id}
  namespace: insight                                  # reconcile-connectors.sh discovers secrets here (override with K8S_NAMESPACE)
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: cursor          # must match descriptor.yaml name
    insight.cyberfabric.com/source-id: cursor-main     # passed as insight_source_id
type: Opaque
stringData:
  cursor_api_key: ""       # Cursor team API key
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `cursor_api_key` | Yes | Cursor team API key (sensitive) |

### Automatically injected

These fields are set by `reconcile-connectors.sh` and should NOT be in the Secret:

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from ConfigMap `insight-config` (ns `data`) or `INSIGHT_TENANT_ID` env |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

Connector credentials are stored in the K8s Secret. Platform identifiers (`insight_tenant_id`, `insight_source_id`) are injected separately by reconcile from the cluster ConfigMap and Secret annotations.

## Streams

| Stream | Sync Mode | Description |
|--------|-----------|-------------|
| `cursor_members` | Full Refresh | Team members list (email, role, status) |
| `cursor_audit_logs` | Incremental | Team audit events (login, settings changes) |
| `cursor_usage_events` | Incremental | Per-request usage events (model, tokens, cost) |
| `cursor_usage_events_daily_resync` | Incremental | Same as usage_events but re-fetches yesterday for finalized costs |
| `cursor_daily_usage` | Incremental | Aggregated daily per-user metrics (requests, tabs, lines) |

## Multi-Instance

To sync multiple Cursor teams, create separate Secrets with different `source-id` annotations:

```yaml
# Secret 1: insight-cursor-main
annotations:
  insight.cyberfabric.com/source-id: cursor-main

# Secret 2: insight-cursor-secondary
annotations:
  insight.cyberfabric.com/source-id: cursor-secondary
```

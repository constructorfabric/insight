# Zoom Connector

Zoom meeting, webinar, and user activity data via Server-to-Server OAuth.

## Prerequisites

1. Create a Server-to-Server OAuth app at https://marketplace.zoom.us/
2. Grant scopes: `dashboard:read:chat:admin`, `dashboard:read:list_meetings:admin`, `dashboard:read:list_meeting_participants:admin`, `user:read:list_users:admin`


## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-zoom-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: zoom
    insight.cyberfabric.com/source-id: main
type: Opaque
stringData:
  zoom_account_id: ""       # Zoom account ID
  zoom_client_id: ""        # OAuth app client ID
  zoom_client_secret: ""    # OAuth app client secret
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `zoom_account_id` | Yes | Zoom Server-to-Server OAuth account ID |
| `zoom_client_id` | Yes | OAuth app client ID |
| `zoom_client_secret` | Yes | OAuth app client secret (sensitive) |

There is no start-date knob: the `meetings` stream reads the Zoom Dashboard API
(`/v2/metrics/*`), which only serves data for the last six months. The first
sync automatically backfills from `now - 150 days` (a safety margin inside that
window); later syncs continue incrementally from saved state.

The same applies to `participants`: its private `_meetings` parent cursor is
persisted in the substream state (`incremental_dependency` + a formal
`join_time` cursor that filters nothing), so each sync fans out one
`/metrics/meetings/{uuid}/participants` request per meeting **newer than the
saved cursor minus 7 days** — not per meeting of the whole 150-day window.
This keeps the sync well inside the Zoom Dashboard-API "Heavy" quota
(60k requests/day per account); the full fan-out is ~25k requests on a busy
account and exhausted the quota when run repeatedly (dev-vhc, 2026-07-14).

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

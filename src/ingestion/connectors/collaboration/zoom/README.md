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

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

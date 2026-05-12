# Slack Connector

Per-day user activity snapshots from Slack Analytics API (`admin.analytics.getFile?type=member`).

## Prerequisites

- Slack workspace on **Business+** or **Enterprise Grid** (Analytics API is not available on lower plans).
- Slack **user OAuth token** (`xoxp-*`) with `admin.analytics:read` scope, installed by an **Org Owner/Admin**. Slack restricts `admin.*` scopes to user tokens — bot tokens (`xoxb-*`) are not supported by `admin.analytics.getFile`.

### Creating the token

1. Go to https://api.slack.com/apps → **Create New App** → **From scratch**
2. Pick the workspace, name the app (e.g. `Insight Analytics`)
3. **OAuth & Permissions** → add `admin.analytics:read` under the appropriate scopes list
4. Install the app by an org admin → copy the resulting OAuth token
5. Store as `slack_bot_token` in the K8s Secret (field name kept for backward compatibility)

## Streams

| Stream | Sync Mode | Primary Key | Cursor | Description |
|--------|-----------|-------------|--------|-------------|
| `users_details` | Incremental | `unique_key` (`tenant-source-user_id-date`) | `date` (P1D) | Per-user per-day Slack activity snapshot |

The stream issues one HTTP request per date from `slack_start_date` to `today - 2 days`. Response body is gzip-compressed JSON Lines; each line is a single member record.

### Fields (from `admin.analytics.getFile?type=member`)

Identity: `team_id`, `user_id`, `email_address`
Lifecycle: `is_guest`, `is_billable_seat`, `date_claimed`
Activity flags: `is_active`, `is_active_ios`, `is_active_android`, `is_active_desktop`, `is_active_apps`, `is_active_workflows`, `is_active_slack_connect`
Counters: `messages_posted_count`, `channel_messages_posted_count`, `reactions_added_count`, `files_added_count`, `total_calls_count`, `slack_calls_count`, `slack_huddles_count`, `search_count`

Injected by the connector: `tenant_id`, `source_id`, `date`, `unique_key`.

### Not included

This connector does **not** fetch:

- Raw messages or threads (no `channels:history` scope used)
- Private channels, DMs, or group DMs
- Channel metadata (use a separate stream/scope if required)

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-slack-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: slack
    insight.cyberfabric.com/source-id: slack-main
type: Opaque
stringData:
  slack_bot_token: "xoxp-..."       # OAuth token with admin.analytics:read
  slack_start_date: "2026-04-03"    # Earliest analytics date to fetch
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `slack_bot_token` | Yes | User OAuth token (`xoxp-*`) with `admin.analytics:read` |
| `slack_start_date` | Yes | Earliest analytics date (YYYY-MM-DD). Recommended: 14 days ago. |

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

## Notes

- Slack publishes analytics with a ~2-day delay. The connector pins `end_date = today - 2 days` to avoid `file_not_yet_available`.
- Slack analytics data is available on a rolling 13-month basis; recently upgraded workspaces may have less history.
- Response content is `application/gzip` containing JSONL (one JSON record per line).

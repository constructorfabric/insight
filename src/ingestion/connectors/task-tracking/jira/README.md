# Jira Connector

Extracts projects, users, issues, issue history (changelog), comments, worklogs, sprints, and field definitions from Jira Cloud REST API v3 using Basic Auth (email + API token). Declarative Airbyte manifest — no custom code.

## Specification

- **PRD**: [../../../../../docs/components/connectors/task-tracking/jira/specs/PRD.md](../../../../../docs/components/connectors/task-tracking/jira/specs/PRD.md)
- **DESIGN**: [../../../../../docs/components/connectors/task-tracking/jira/specs/DESIGN.md](../../../../../docs/components/connectors/task-tracking/jira/specs/DESIGN.md)

## Prerequisites

1. Generate an Atlassian API token at [id.atlassian.com/manage-profile/security/api-tokens](https://id.atlassian.com/manage-profile/security/api-tokens).
2. Use the email address of the Atlassian account that has **Browse Projects** on every target project.
3. Identify the project keys to sync (e.g. `TC`, `TNG`) — visible in any issue URL as the prefix before the hyphen. Jira Cloud rejects unbounded JQL queries, so this is **required**.

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-jira-main
  namespace: insight
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: jira
    insight.cyberfabric.com/source-id: jira-main
type: Opaque
stringData:
  jira_instance_url: "https://myorg.atlassian.net"
  jira_email: "user@example.com"
  jira_api_token: "CHANGE_ME"
  jira_project_keys: "PROJ1,PROJ2"
  # jira_start_date: "2024-01-01"   # optional, default = 2020-01-01
  # jira_page_size: "50"             # optional, default = 50, max 100
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `jira_instance_url` | Yes | Jira Cloud URL, no trailing slash (e.g. `https://myorg.atlassian.net`) |
| `jira_email` | Yes | Atlassian account email for Basic Auth |
| `jira_api_token` | Yes | Atlassian API token. Marked `airbyte_secret: true` — never logged |
| `jira_project_keys` | Yes | Comma-separated project keys (e.g. `TC,TNG`). Jira Cloud rejects unbounded JQL queries |
| `jira_start_date` | No | Earliest date to sync issues from, `YYYY-MM-DD`. Default `2020-01-01` |
| `jira_page_size` | No | JQL page size, 1..100. Default `50` |

### Automatically injected

These fields are added to every record by the connector — do **not** put them in the K8s Secret:

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML (`connections/<tenant>.yaml`) |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation on the K8s Secret |
| `tenant_id` / `source_id` | Mirrored onto every Bronze row |
| `unique_key` | Composite PK — varies per stream (see DESIGN §3.7) |
| `collected_at` | UTC ISO-8601 timestamp at extraction time |

### Local development

```bash
cp src/ingestion/secrets/connectors/jira.yaml.example src/ingestion/secrets/connectors/jira.yaml
# Fill in real values, then apply:
kubectl apply -f src/ingestion/secrets/connectors/jira.yaml
```

## Streams

| Stream | Endpoint | Sync Mode | Cursor | Pagination |
|--------|----------|-----------|--------|-----------|
| `jira_fields` | `GET /rest/api/3/field` | Full refresh | — | None |
| `jira_projects` | `GET /rest/api/3/project/search` | Full refresh | — | Offset (`startAt` / `maxResults`) |
| `jira_user` | `GET /rest/api/3/users/search` | Full refresh | — | Offset |
| `jira_issue` | `GET /rest/api/3/search/jql` | Incremental | `updated` | Cursor (`nextPageToken`) |
| `jira_issue_history` | `GET /rest/api/3/issue/{key}/changelog` | Substream of `jira_issue` | — | Offset |
| `jira_comments` | `GET /rest/api/3/issue/{key}/comment` | Substream of `jira_issue` | — | Offset |
| `jira_worklogs` | `GET /rest/api/3/issue/{key}/worklog` | Substream of `jira_issue` | — | Offset |
| `jira_sprints` | `GET /rest/agile/1.0/board/{board_id}/sprint` | Substream of boards | — | Offset |

The `jira_boards` stream (`GET /rest/agile/1.0/board`) is the substream parent for `jira_sprints` and materializes its own Bronze table.

### Identity Key

- `jira_user.email_address` — primary identity key
- `jira_issue.reporter_id`, `jira_issue_history.author_account_id`, `jira_comments.author_account_id`, `jira_worklogs.author_account_id` — Atlassian `accountId` resolved to `email` downstream via `jira_user` JOIN in Silver.

## Silver Targets

- `class_task_tracker_*` — cross-source task-tracker unification is the responsibility of the Silver/dbt layer (DESIGN §1.1, §3.2).
- This connector package currently ships the Bronze source declaration (`dbt/schema.yml`) only; staging/Silver models are delivered by the task-tracking Silver layer package.

## Operational Constraints

- **Auth**: Basic Auth with email + API token. Missing/invalid token → HTTP 401; Jira project-level permission failures → 403. Both halt the run.
- **Rate limits**: Atlassian caps per-user and per-IP API calls. The connector honours `Retry-After` on HTTP 429 and 503 (both used by Atlassian for throttling) with backoff.
- **JQL scope**: `jira_project_keys` is required; Jira Cloud rejects unbounded queries (`project != EMPTY`) with an error.
- **Custom fields**: all custom fields are preserved in `jira_issue.custom_fields_json` for downstream dbt extraction.

## Related

- Silver layer (unified task-tracker schema): `docs/components/connectors/task-tracking/silver/`
- Sibling connector: YouTrack (planned, same Silver target)

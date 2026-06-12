# Figma Connector

Extracts the design workspace inventory and collaboration signals from the
Figma REST API v1 (projects, files, file metadata, version history, comments)
using a personal access token. Bronze-only: raw streams land in
`bronze_figma`, no Silver transformations yet.

## Prerequisites

1. **Personal access token** — any Figma user can create one: Figma →
   Settings → **Security** tab → Personal access tokens → Generate new token.
   Select read-only scopes covering projects, file metadata, file versions
   and file comments. The token is shown once — copy immediately.
2. **Token owner matters twice**:
   - The token sees exactly what the owner sees — the account must be a
     member of every team you want to collect.
   - API rate limits are tied to the owner's seat type. A Dev/Full seat gets
     50–150 req/min on the endpoints this connector uses; a viewer/collab
     seat gets 5–10 req/min. Use a Dev/Full seat account.
3. **Team IDs** — the API cannot enumerate teams. Open each team in the
   browser and copy the ID from the URL: `figma.com/files/team/<team_id>/...`

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-figma-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: figma
    insight.cyberfabric.com/source-id: figma-main
type: Opaque
stringData:
  figma_token: "CHANGE_ME"
  figma_team_ids: "CHANGE_ME"          # comma-separated team IDs
  figma_start_date: "2020-01-01"
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `figma_token` | Yes | Personal access token (`X-Figma-Token` header). |
| `figma_team_ids` | Yes | Comma-separated Figma team IDs. No default — must be set explicitly. |
| `figma_start_date` | No | Earliest date of interest (YYYY-MM-DD). Files not modified since, and versions created before, this date are skipped. Defaults to `2020-01-01`. |
| `figma_page_size` | No | Page size for the versions endpoint (default 50 = API max). |

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

Create `src/ingestion/secrets/connectors/figma.yaml` (gitignored) from the example:

```bash
cp src/ingestion/secrets/connectors/figma.yaml.example src/ingestion/secrets/connectors/figma.yaml
# Fill in real values, then apply:
kubectl apply -f src/ingestion/secrets/connectors/figma.yaml
```

## Streams

| Stream | Description | Sync Mode |
|--------|-------------|-----------|
| `design_projects` | Projects per configured team (`GET /v1/teams/{id}/projects`). | Full Refresh |
| `design_files` | Files per project: key, name, `last_modified`, denormalized project/team (`GET /v1/projects/{id}/files`). | Incremental (client-side cursor on `last_modified`) |
| `design_file_meta` | Per-file metadata: creator, last_touched_by, editor type (figma/figjam/slides/…), link access (`GET /v1/files/{key}/meta`). | Full Refresh (substream) |
| `design_file_versions` | Version history: author, timestamp, label/description (`GET /v1/files/{key}/versions`). Bounded by `figma_start_date` via record filter + pagination stop. | Full Refresh (substream) |
| `design_file_comments` | Comments incl. replies (`parent_comment_id`), resolution timestamps, reaction counts (`GET /v1/files/{key}/comments`). | Full Refresh (substream) |

API behaviors encoded in the manifest:

- **Rate limits** are tiered per endpoint and per token-owner seat. All five
  endpoints used are Tier 2/3. The full-document endpoint
  (`GET /v1/files/{key}`, Tier 1 — 6 req/month on viewer seats) is
  deliberately not used; design content never leaves Figma.
- **429** is retried honouring `Retry-After` (capped at 600 s, like
  confluence); 5xx retried with backoff.
- **403/404 on per-file endpoints are IGNOREd** — invite-only projects and
  deleted files are routine and must not kill the sync. On the team-level
  `design_projects` stream they FAIL, because there they mean a bad token or
  team ID.
- **No user directory**: the REST API has no team/org member enumeration
  (Enterprise-only SCIM does), and `User` objects carry only `id`/`handle` —
  never email. Identity resolution is deferred to dbt (handle-based matching
  via the Identity Manager, or an Enterprise SCIM directory later).

## Silver Targets

None yet — bronze-only delivery. `dbt_select: tag:figma+` runs
`figma__bronze_promoted` (ReplacingMergeTree promotion of the five bronze
tables). The planned Silver stream is `class_design_activity` per
`docs/components/connectors/ui-design/README.md`.

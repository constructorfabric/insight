# HubSpot Connector

CDK-based Python connector for HubSpot CRM. Pulls live data via CRM v3 Search API with v4 associations and archived data via list + batch_read; only an allowlisted subset of `hubspotDefined` standard properties (the curated `ALLOWED_PROPERTIES_BY_OBJECT`) surfaces as typed Bronze columns. Tenant-defined (`hubspotDefined=false`) properties are folded into a single `custom_fields` JSON column so Bronze stays stable across portals and bounded in width regardless of customization depth.

Streams sync sequentially. HubSpot's search endpoint is rate-limited to 4 rps portal-wide so a single thread saturates the cap; concurrency would only redistribute the same 4 rps across more 429 retries.

## Prerequisites

1. In HubSpot: **Settings → Integrations → Private Apps → Create private app**.
2. Grant read scopes for each enabled object plus the matching property-schema scope (HubSpot has no wildcard — list scopes individually):
   - Objects: `crm.objects.contacts.read`, `crm.objects.companies.read`, `crm.objects.deals.read`, `crm.objects.leads.read`, `tickets`, `crm.objects.owners.read`
   - Property schemas: `crm.schemas.contacts.read`, `crm.schemas.companies.read`, `crm.schemas.deals.read`, `crm.schemas.leads.read`
   - Engagements (calls/emails/meetings/tasks) read is covered by the per-object CRM scopes.
3. Copy the access token — it begins with `pat-`.

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-hubspot-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: hubspot
    insight.cyberfabric.com/source-id: hubspot-main
type: Opaque
stringData:
  hubspot_access_token: ""                           # Private App token (pat-...)
  hubspot_start_date: "2024-01-01T00:00:00Z"         # Optional
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `hubspot_access_token` | Yes | Private App access token (sensitive) |
| `hubspot_start_date` | No | Incremental sync start (ISO 8601). Defaults to two years before current date |

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Multi-instance

Deploy additional Secrets with distinct `source-id` annotations to ingest multiple HubSpot portals as separate sources.

## Streams

**19 streams** — 10 live + 9 archived siblings (one per live object whose archived listing HubSpot supports). Live streams feed Silver via search on `hs_lastmodifieddate`; archived siblings full-sweep `/crm/v3/objects/{type}?archived=true` and are merged into Silver with `_version = greatest(updatedAt, archivedAt)` so an archive event outranks the prior live update.

### Live (search)

| Stream | Silver target | Cursor | PK |
|---|---|---|---|
| `contacts` | `class_crm_contacts` | `updatedAt` | `id` |
| `companies` | `class_crm_accounts` | `updatedAt` | `id` |
| `deals` | `class_crm_deals` | `updatedAt` | `id` |
| `engagements_calls` | `class_crm_activities` | `updatedAt` | `id` |
| `engagements_emails` | `class_crm_activities` | `updatedAt` | `id` |
| `engagements_meetings` | `class_crm_activities` | `updatedAt` | `id` |
| `engagements_tasks` | `class_crm_activities` | `updatedAt` | `id` |
| `owners` | `class_crm_users` | `updatedAt` | `id` |
| `leads` | bronze-only | `updatedAt` | `id` |
| `tickets` | bronze-only | `updatedAt` | `id` |

Every stream is incremental. `hubspot_start_date` controls the initial backfill window; subsequent syncs only fetch records modified after the cursor's high-water mark.

CRM streams use the search endpoint (`POST /crm/v3/objects/{type}/search`) for live records — single window per sync filtered on `hs_lastmodifieddate`, sorted by `hs_object_id ASC`, paged via `after`; when HubSpot's hard cap of 10,000 results per query is hit, the loop restarts the same window with `hs_object_id > last_seen_id` keyset.

`owners` has no search endpoint and the list endpoint has no `updatedAt` filter, so the stream pages the full owner list every sync but filters records client-side on `updatedAt > state`; only changed owners are written to Bronze after the first sync.

### Archived (list + batch_read)

| Stream | Silver target | Cursor | PK |
|---|---|---|---|
| `contacts_archived` | `class_crm_contacts` | `archivedAt` | `id` |
| `companies_archived` | `class_crm_accounts` | `archivedAt` | `id` |
| `deals_archived` | `class_crm_deals` | `archivedAt` | `id` |
| `engagements_calls_archived` | `class_crm_activities` | `archivedAt` | `id` |
| `engagements_emails_archived` | `class_crm_activities` | `archivedAt` | `id` |
| `engagements_tasks_archived` | `class_crm_activities` | `archivedAt` | `id` |
| `owners_archived` | `class_crm_users` | `archivedAt` | `id` |
| `leads_archived` | bronze-only | `archivedAt` | `id` |
| `tickets_archived` | bronze-only | `archivedAt` | `id` |

Archived streams are **client-side incremental on `archivedAt`**: HubSpot has no server-side `archivedAt` filter, so the stream pages the entire archived set every sync but drops records whose `archivedAt` precedes the prior cursor state and only emits newly-archived rows downstream. First sync lands all archives; subsequent syncs typically write 0 records.

CRM-object archived streams (everything except owners) use a **two-pass list + batch_read** to fetch the full property set without HTTP 414:

1. `GET /crm/v3/objects/{type}?archived=true` — collect ids + `archivedAt` for archives newer than the cursor (no `properties=` query param so the URL stays short).
2. `POST /crm/v3/objects/{type}/batch/read?archived=true` — fetch full properties (standard + custom) for ≤100 ids per call; the property list rides in the JSON body, no URL cap.

`engagements_meetings_archived` is **deliberately absent**: HubSpot returns 400 *"Paging through deleted objects is not yet supported"* on `/crm/v3/objects/meetings?archived=true`. The runtime keeps a defensive 400 swallow on the Pass 1 list path so a future regression on any other object type degrades to an empty stream + warning rather than a sync failure.

## Robustness

### 10,000-result search cap
HubSpot's CRM Search endpoint caps at `after = 10,000`. The connector sorts every search by `hs_object_id ASC`; when a window crosses the cap, the loop restarts the same window with `hs_object_id > last_seen_id` keyset. Logs show `restarting from hs_object_id>...` when this kicks in.

### Rate limits
- Burst: 10 rps (standard portals), 100 rps (Enterprise).
- Search endpoint: 4 rps portal-wide, no `Retry-After` header. Connector uses a 1.2s fallback on 429 for search requests, 3s elsewhere. Streams sync sequentially; concurrency wouldn't lift throughput because a single thread already saturates 4 rps at typical search latency.
- Daily request limit: fails fast with a `transient_error` after 5 retries so orchestration can alert.

### Error surfacing
- `401` — Private App token invalid or revoked. Fail fast with config-error message.
- `403 MISSING_SCOPES` — fail with the missing scope list parroted back from HubSpot's response.
- `530` — Cloudflare origin-DNS; indicates a malformed token. Fail fast with token-format hint.
- `5xx`, chunked-encoding, connection resets — retried with exponential backoff.

### Property scope
Bronze advertises **only the curated `ALLOWED_PROPERTIES_BY_OBJECT` allowlist** of `hubspotDefined` standard properties as typed `properties_*` columns; standard properties outside the allowlist are skipped. Tenant-defined (`hubspotDefined=False`) properties land in the `custom_fields` JSON column with null/empty values dropped and per-value byte cap applied (see envelope). This keeps Bronze width bounded regardless of portal customization depth — typical width is 5–15 typed columns per object plus `custom_fields`, instead of the 50–250+ you'd get from projecting every standard property. To project a new standard column, add it to `ALLOWED_PROPERTIES_BY_OBJECT[object_type]` in `constants.py`.

### Deleted / archived records
Each archived stream runs as **client-side incremental on `archivedAt`** — page the full archived set, drop records at-or-below the prior cursor state, batch_read full properties for the survivors. After the first sync, only newly-archived rows write to Bronze. Silver UNIONs the live and archived sources and ranks rows by `greatest(updatedAt, archivedAt)` so an archive event outranks the prior live update. The `archived: true` flag is still surfaced on Silver rows via the `metadata` JSON column.


## Local development

```bash
cp src/ingestion/secrets/connectors/hubspot.yaml.example src/ingestion/secrets/connectors/hubspot.yaml
# fill in real values, then:
./src/ingestion/secrets/apply.sh
```

Build + test:

```bash
/connector build crm/hubspot
/connector test  crm/hubspot
/connector schema crm/hubspot
```

Run a sync:

```bash
./src/ingestion/run-sync.sh hubspot {tenant_id}
./src/ingestion/logs.sh hubspot {tenant_id}     # tail logs
```

## Troubleshooting

**401 immediately after token rotation.** Private App tokens propagate eventually but HubSpot can cache the previous value at the edge for a minute or two. Wait and retry; if it persists, regenerate.

**`MISSING_SCOPES` on a stream the operator didn't realize needed scopes.** Property discovery requires `crm.schemas.{object}.read` for custom properties — granting only `crm.objects.{object}.read` returns standard fields but may still fail property lookup. Grant both.

**Silver `class_crm_users.email NOT NULL` test failing.** Deactivated HubSpot Owners can have null email. The staging model `hubspot__crm_users.sql` filters these out; if a null-email owner surfaces anyway, it's usually a test row created with an empty email field. Filter at source or relax the Silver test — see the plan notes.

# HubSpot Connector

CDK-based Python connector for HubSpot CRM. Pulls live data via CRM v3 Search API with v4 associations and archived data via list + batch_read; the allowlist in `ALLOWED_PROPERTIES_BY_OBJECT` surfaces as dedicated Bronze columns and defines the advertised schema statically, so the table shape is identical across portals. Every other property still ships in the `raw_data` JSON column, keeping Bronze width bounded without losing values.

Streams sync sequentially. HubSpot's search endpoint is rate-limited to 4 rps portal-wide so a single thread saturates the cap; concurrency would only redistribute the same 4 rps across more 429 retries.

## Prerequisites

1. In HubSpot: **Settings → Integrations → Private Apps → Create private app**.
2. Grant read scopes for each enabled object plus the matching property-schema scope (HubSpot has no wildcard — list scopes individually):
   - Objects: `crm.objects.contacts.read`, `crm.objects.companies.read`, `crm.objects.deals.read`, `crm.objects.leads.read`, `tickets`, `crm.objects.owners.read`
   - Property schemas: `crm.schemas.contacts.read`, `crm.schemas.companies.read`, `crm.schemas.deals.read`, `crm.schemas.leads.read`
   - Engagements (calls/emails/meetings/tasks/notes/communications), pipelines and property history are covered by the per-object CRM scopes above.
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

**27 streams** — 12 live objects, 10 archived siblings (one per live object whose archived listing HubSpot supports) and 5 definition / history streams. Live streams search on `hs_lastmodifieddate`; archived siblings full-sweep `/crm/v3/objects/{type}?archived=true` and are merged into Silver with `_version = greatest(updatedAt, archivedAt)` so an archive event outranks the prior live update.

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
| `engagements_notes` | `class_crm_activities` | `updatedAt` | `id` |
| `engagements_communications` | `class_crm_activities` | `updatedAt` | `id` |
| `leads` | `class_crm_leads` | `updatedAt` | `id` |
| `tickets` | `class_crm_tickets` | `updatedAt` | `id` |
| `owners` | `class_crm_users`, `class_crm_teams` | `updatedAt` | `id` |

Every object stream except `owners` also feeds `class_crm_associations`.

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
| `engagements_notes_archived` | `class_crm_activities` | `archivedAt` | `id` |
| `leads_archived` | `class_crm_leads` | `archivedAt` | `id` |
| `tickets_archived` | `class_crm_tickets` | `archivedAt` | `id` |
| `owners_archived` | `class_crm_users` | `updatedAt` | `id` |

Archived owners carry no `archivedAt`, so `owners_archived` filters on `updatedAt` and, like `owners`, emits every archived owner on the first sync regardless of `hubspot_start_date` — deals keep referencing reps deactivated long before it.

The other archived streams are **client-side incremental on `archivedAt`**: HubSpot has no server-side `archivedAt` filter, so the stream pages the entire archived set every sync but drops records whose `archivedAt` precedes the prior cursor state and only emits newly-archived rows downstream. First sync lands all archives; subsequent syncs typically write 0 records.

CRM-object archived streams (everything except owners) use a **two-pass list + batch_read** to fetch the full property set without HTTP 414:

1. `GET /crm/v3/objects/{type}?archived=true` — collect ids + `archivedAt` for archives newer than the cursor (no `properties=` query param so the URL stays short).
2. `POST /crm/v3/objects/{type}/batch/read?archived=true` — fetch full properties (standard + custom) for ≤100 ids per call; the property list rides in the JSON body, no URL cap.

`engagements_meetings_archived` and `engagements_communications_archived` are **deliberately absent**: HubSpot returns 400 *"Paging through deleted objects is not yet supported"* for both object types. The runtime keeps a defensive 400 swallow on the Pass 1 list path so a future regression on any other object type degrades to an empty stream + warning rather than a sync failure.

### Definitions and history

| Stream | Endpoint | Silver target | Cursor | PK |
|---|---|---|---|---|
| `deal_pipelines` | `GET /crm/v3/pipelines/deals` | `class_crm_pipelines`, `class_crm_pipeline_stages` | full sweep | `id` |
| `ticket_pipelines` | `GET /crm/v3/pipelines/tickets` | `class_crm_pipelines`, `class_crm_pipeline_stages` | full sweep | `id` |
| `property_definitions` | `GET /crm/v3/properties/{type}` (live + archived) | `class_crm_property_definitions` | full sweep | `{objectType}:{name}` |
| `deals_property_history` | search + `POST /crm/v3/objects/deals/batch/read` with `propertiesWithHistory` | `class_crm_field_history` | `updatedAt` of the deal | `{objectId}:{property}:{digest}` |
| `tickets_property_history` | search + `POST /crm/v3/objects/tickets/batch/read` with `propertiesWithHistory` | `class_crm_field_history` | `updatedAt` of the ticket | `{objectId}:{property}:{digest}` |

Pipelines and property definitions are small, unpaged and carry no reliable modification time (a stage edit need not move its pipeline's `updatedAt`; HubSpot-defined properties often have none), so every sync re-reads them whole and Bronze dedups on `unique_key`.

The history streams search for objects modified since the cursor, requesting only `hs_object_id`, then resolve them through `batch/read` with `propertiesWithHistory` — HubSpot caps that call at 50 inputs. Each change becomes one record. A changed object re-emits its whole history, so a change's id is a digest of the change itself (timestamp, value, source, user) and a replay lands on the same `unique_key`.

| History stream | Properties |
|---|---|
| `deals_property_history` | `dealstage`, `pipeline`, `amount`, `amount_in_home_currency`, `deal_currency_code`, `closedate`, `hubspot_owner_id`, `hs_manual_forecast_category` |
| `tickets_property_history` | `hs_pipeline_stage`, `hs_pipeline`, `hubspot_owner_id`, `hs_ticket_priority` |

Teams come from the `teams` array on `owners` (`id`, `name`, `primary`): `/settings/v3/users/teams` needs a settings scope a CRM-only Private App does not carry.

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
Bronze advertises **exactly the `ALLOWED_PROPERTIES_BY_OBJECT` allowlist** as `properties_*` columns. The advertised schema is static — derived from `constants.py`, not from a portal describe — so every portal produces the same table shape and an allowlisted property the portal doesn't define is simply NULL. To project a new column, add it to `ALLOWED_PROPERTIES_BY_OBJECT[object_type]` in `constants.py`.

Free-text bodies (`hs_call_body`, `hs_email_text`, `hs_meeting_body`, `hs_internal_meeting_notes`, `hs_task_body`, `hs_note_body`, `hs_communication_body`, ticket `content`, deal `description`) keep up to 64 KiB in their `properties_*` column; every other value is capped at 2 KiB.

Syncs still request **every** property the portal defines. Nothing is discarded: the full record — nested `properties` object plus association ids — is serialized into the `raw_data` JSON column, so a property with no dedicated column stays recoverable downstream. The per-value byte cap (see envelope) applies before serializing, so the blob itself stays parseable. This keeps the typed width bounded regardless of portal customization depth.

### Deleted / archived records
Each archived stream runs as **client-side incremental on `archivedAt`** — page the full archived set, drop records at-or-below the prior cursor state, batch_read full properties for the survivors. After the first sync, only newly-archived rows write to Bronze. Silver UNIONs the live and archived sources and ranks rows by `greatest(updatedAt, archivedAt)` so an archive event outranks the prior live update. Silver rows carry it as `is_deleted` / `deleted_at`.


## Silver layer

The connector's staging models (`dbt/hubspot__crm_*.sql`, tag `hubspot`) feed the `silver.class_crm_*` classes through `union_by_tag`; nothing reads Bronze directly. Every class is ReplacingMergeTree on `unique_key` with `_version`; read it with `FINAL` (or `argMax(... , _version)`).

**Why multiple tables:** a CRM record links to many others (a deal has many contacts and activities), pipelines and properties are dictionaries the records only reference by id, and a property's history has one row per change — none of these fits a column of its parent without losing rows.

### Shared conventions

- `unique_key` is `{tenant}-{source}-{natural key}`; the natural key includes the object type wherever HubSpot ids are unique only per type (associations, pipelines, stages, history, property definitions).
- **Deletion is a row state, not a missing row.** Archived records stay with `is_deleted = 1` and `deleted_at`; filter `is_deleted = 0` for the live picture.
- **`properties`** (`Map(String, String)`) on accounts, contacts, deals, activities, leads and tickets holds every non-empty property of the record — standard and custom — as HubSpot returns it (`properties['my_custom_field']`). Labels, types and option labels are in `class_crm_property_definitions`. It is read from `raw_data`, so each value is capped at 2 KiB; dedicated columns carry the untruncated bodies.
- Staging keeps one row per `unique_key` per batch (`LIMIT 1 BY unique_key` over `_version DESC`), and silver is delete+insert on `unique_key`; `FINAL` still guards against parts that have not merged yet.
- Owner columns (`owner_id`) join `class_crm_users.user_id` (owners.id); creator columns (`created_by_user_id`, `changed_by_user_id`) join `class_crm_users.hs_user_id` (owners.userId).

### Classes

| Class | Grain | Bronze source | Key columns |
|---|---|---|---|
| `class_crm_accounts` | company | `companies` (+archived) | `account_id`, `name`, `domain`, `owner_id` |
| `class_crm_contacts` | contact | `contacts` (+archived) | `contact_id`, `email`, `owner_id`, `lifecycle_stage`, `account_id` (primary company) |
| `class_crm_deals` | deal | `deals` (+archived) | `deal_id`, `pipeline_id`, `stage`, `amount`, `amount_home`, `currency`, `close_date`, `is_closed`, `is_won`, `owner_id` |
| `class_crm_activities` | engagement | `engagements_{calls,emails,meetings,tasks,notes,communications}` (+archived) | `activity_type`, `timestamp`, `owner_id`, `created_by_user_id`, `subject`, `body`, `direction`, `status`, `channel`, `duration_seconds`, `ended_at` |
| `class_crm_leads` | lead | `leads` (+archived) | `lead_id`, `pipeline_id`, `stage_id`, `owner_id`, `contact_id`, `account_id` |
| `class_crm_tickets` | ticket | `tickets` (+archived) | layout of `silver.dim_support_ticket`, then `pipeline_id`, `stage_id`, `content`, `deal_id` |
| `class_crm_users` | owner | `owners` (+archived) | `user_id`, `hs_user_id`, `email`, `is_active`, `primary_team_id`, `team_ids` |
| `class_crm_teams` | team | `owners.teams` | `team_id`, `name` |
| `class_crm_associations` | record × associated object type | every object stream | `from_object_type`, `from_object_id`, `to_object_type`, `to_object_ids` |
| `class_crm_pipelines` | pipeline | `deal_pipelines`, `ticket_pipelines` | `pipeline_id`, `object_type`, `label`, `display_order` |
| `class_crm_pipeline_stages` | stage | `deal_pipelines`, `ticket_pipelines` | `stage_id`, `pipeline_id`, `object_type`, `label`, `display_order`, `probability`, `is_closed`, `is_won`, `ticket_state` |
| `class_crm_field_history` | property change | `deals_property_history`, `tickets_property_history` | `object_type`, `object_id`, `field`, `value`, `changed_at`, `change_source_type`, `changed_by_user_id` |
| `class_crm_property_definitions` | property of an object type | `property_definitions` | `object_type`, `name`, `label`, `type`, `field_type`, `is_custom`, `options` (value → label) |

### Notes per class

- **Activities.** `body` is the engagement's free text: call notes, email plain text, meeting body, task body, note body, message text. Meeting `timestamp` is the meeting start; `ended_at` is set for meetings only. `channel` is set for communications only (`SMS`, `WHATS_APP`, `LINKEDIN_MESSAGE`, …). `contact_id` / `deal_id` / `account_id` hold the first association; the full lists are in `class_crm_associations`.
- **Tickets.** Columns up to `_version` repeat `silver.dim_support_ticket` in name, order and type (including `insight_source_id`), so CRM and support tickets can be unioned positionally. `status_canonical` is `open` / `closed` from `hs_is_closed`; `priority_canonical` lowercases `hs_ticket_priority` with `MEDIUM` → `normal`; `type_canonical` is the lowercased category; `group_source_id` is the HubSpot team; requester and organisation are the first associated contact and company.
- **Associations.** One row per record and associated object type with the complete id list, replaced whenever the record changes — a removed association disappears with the record's next version. Use `ARRAY JOIN to_object_ids` for pairs.
- **Pipeline stages.** `is_won` is set for deal stages that close at probability 1 (HubSpot carries no explicit won flag); ticket stages carry `ticket_state` instead. Outcomes such as *refunded* are stage labels, read from `label`. A stage or pipeline the latest poll no longer returns stays with `is_deleted = 1` and its last label, so history still joins to it.
- **Field history.** One row per value change of the tracked properties, with `change_source_type` (`CRM_UI`, `IMPORT`, `API`, `CALCULATED`, …). Deal stage transitions are `field = 'dealstage'`; time in stage is the gap to the next change of the same object and field.
- **Teams.** Known only through owner memberships, so a team without members is absent.

### Identity

HubSpot owners feed `identity.identity_inputs` with `insight_source_type = 'hubspot'`, so deals and engagements resolve to the same `person_id` as the person's accounts in other sources.

`hubspot__owners_latest` → `hubspot__owners_snapshot` → `hubspot__owners_fields_history` → `hubspot__identity_inputs`

- `source_account_id` is the owner id (`owners.id`) — the value deals, engagements, leads and tickets carry in `owner_id`. Columns that hold an owner's `userId` (`created_by_user_id`, `changed_by_user_id`) go through `class_crm_users.hs_user_id` first.
- Emitted values: `email` (lower-cased) and `display_name` (first + last name).
- An archived owner keeps its identity values: archiving means the HubSpot user was deactivated, and their deals and engagements still have to resolve to the person.

### Example: open deals per owner at a point in time

The state of a deal at a moment `T` is its last change at or before `T` in `class_crm_field_history`, falling back to the current value when the deal has no change in that window. A deal is open when that stage is not closing. The sketch shows the idea; a real query also reads with `FINAL`, scopes joins by `tenant_id` / `source_id` and drops deals deleted before `T`.

```sql
SELECT owner_at_T, count() AS open_deals
FROM (
    SELECT
        d.deal_id,
        coalesce(argMaxIf(h.value, h.changed_at, h.field = 'dealstage'),        d.stage)    AS stage_at_T,
        coalesce(argMaxIf(h.value, h.changed_at, h.field = 'hubspot_owner_id'), d.owner_id) AS owner_at_T
    FROM silver.class_crm_deals AS d
    LEFT JOIN silver.class_crm_field_history AS h
           ON h.object_id = d.deal_id AND h.changed_at <= T
    WHERE d.created_at <= T
    GROUP BY d.deal_id, d.stage, d.owner_id
) AS s
JOIN silver.class_crm_pipeline_stages AS st ON st.stage_id = s.stage_at_T
WHERE st.is_closed = 0
GROUP BY owner_at_T
```

History exists only for deals modified since `hubspot_start_date`; for an older `T` the fallback to the current state is an approximation. `owner_at_T` is a HubSpot owner id; identity resolution maps it to a `person_id` (see Identity above).

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

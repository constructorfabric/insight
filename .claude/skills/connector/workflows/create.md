---
name: connector-create
description: "Create a new Insight Connector package"
---

# Create Connector

Creates a complete Insight Connector package with all required files.

## Phase 1: Gather Information

Ask the user (skip questions where context already provides the answer):

```
[Q1] Category? (collaboration / hr-directory / git / task-tracking / crm / support / ai / wiki)
[Q2] Connector name? (short, lowercase, e.g. m365, bamboohr, jira)
[Q3] API base URL? (e.g. https://graph.microsoft.com/v1.0)
[Q4] Auth type? (oauth2_client_credentials / api_key / bearer / basic)
[Q5] API documentation URL? (optional — will fetch and analyze)
[Q6] What data streams should the connector extract? (e.g. users, activities, tickets)
```

## Phase 2: Research API (if docs URL provided)

1. Fetch API documentation via WebFetch
2. Identify: endpoints, auth flow, pagination pattern, rate limits
3. Identify: available fields per stream, primary keys, cursor fields
4. Summarize findings for user confirmation

## Phase 3: Create Package

### For nocode (`CONNECTOR_TYPE=nocode`):

**⚠️ Pick the reference connector carefully.** Manifests that work at runtime can still be rejected by the Airbyte Builder UI. Read `src/ingestion/tools/declarative-connector/README.md` §"Builder-UI compatibility — hard rules" before copying anything.

Builder-UI-compatible references (OK to copy):
- `src/ingestion/connectors/collaboration/zoom/connector.yaml`
- `src/ingestion/connectors/collaboration/m365/connector.yaml`
- `src/ingestion/connectors/hr-directory/bamboohr/connector.yaml`

**Do NOT copy from**:
- `src/ingestion/connectors/task-tracking/jira/connector.yaml` — uses whole-object `$ref` (`#/definitions/auth`, `#/definitions/paginator`, `#/streams/N`) which the Builder strict validator rejects. It loads via the CDK runtime but cannot be opened in the Builder UI without full expansion.

Create files:

#### 3.1 `connector.yaml` — Airbyte declarative manifest

The manifest MUST be compatible with Airbyte Builder (import/export without manual fixes).

**Manifest version**: Use `version: 7.0.4` for new connectors. Existing connectors may use older versions (e.g. 6.44.0, 6.60.9) — do NOT change their version unless upgrading. The version refers to the Airbyte CDK declarative schema version. Breaking changes between 6.x and 7.x:
- v7 requires `type: DeclarativeSource` at top level
- v7 field definitions use `type: AddedFieldDefinition` explicitly
- v7 schemas use `http://json-schema.org/schema#` (not `draft-07`)

**Top-level structure** (order matters for Builder compatibility):

```yaml
version: 7.0.4
type: DeclarativeSource

check:
  type: CheckStream
  stream_names:
    - <lightest_stream>

definitions:
  linked:
    ...

streams:
  - type: DeclarativeStream
    ...

concurrency_level:
  type: ConcurrencyLevel
  default_concurrency: 1

spec:
  ...

metadata:
  autoImportSchema:
    <stream_name>: true
```

**`definitions.linked` pattern** — Builder uses granular `$ref` linking, NOT whole-object refs:

```yaml
definitions:
  linked:
    HttpRequester:
      url_base: https://api.example.com/v1
      authenticator:
        type: BasicHttpAuthenticator
        username: "{{ config['<prefix>_api_key'] }}"
        password: x
      request_headers:
        Accept: application/json
    SimpleRetriever:
      paginator:
        type: NoPagination
```

Each stream references individual properties from `definitions.linked`:

```yaml
requester:
  type: HttpRequester
  url_base:
    $ref: "#/definitions/linked/HttpRequester/url_base"
  authenticator:
    $ref: "#/definitions/linked/HttpRequester/authenticator"
  request_headers:
    $ref: "#/definitions/linked/HttpRequester/request_headers"
  path: <stream_specific_path>
```

Do NOT put `error_handler` in `definitions.linked` — Builder strips linked error handlers. Error handling is either per-stream in the requester or handled by the runtime.

**Streams go at root level** (`streams:`), NOT under `definitions`. They reference definitions via `$ref`.

**`check` block** goes BEFORE `definitions`, at the top of the manifest (after version/type). Use the lightest stream for the health check.

**`transformations` with AddFields** — each field item MUST have `type: AddedFieldDefinition`:

```yaml
transformations:
  - type: AddFields
    fields:
      - type: AddedFieldDefinition
        path:
          - tenant_id
        value: "{{ config['insight_tenant_id'] }}"
      - type: AddedFieldDefinition
        path:
          - source_id
        value: "{{ config['insight_source_id'] }}"
      - type: AddedFieldDefinition
        path:
          - unique_key
        value: >-
          {{ config['insight_tenant_id'] }}-{{ config['insight_source_id']
          }}-{{ record['<primary_field>'] }}
```

Only inject: `tenant_id`, `source_id`, `unique_key`, and optionally `raw_data` for configurable streams. Do NOT add `_source` or `_extracted_at` — dbt models handle source tagging, and Airbyte auto-generates `_airbyte_extracted_at`.

**Schema rules** — must match Builder output format:

```yaml
schema_loader:
  type: InlineSchemaLoader
  schema:
    type: object
    $schema: http://json-schema.org/schema#
    properties:
      unique_key:
        type: string
      tenant_id:
        type:
          - string
          - "null"
      source_id:
        type:
          - string
          - "null"
      # ... source fields with [type, "null"] order
    required:
      - unique_key
    additionalProperties: true
```

Schema specifics:
- Use `http://json-schema.org/schema#` (Builder output), NOT `http://json-schema.org/draft-07/schema#`
- Type arrays: `[type, "null"]` not `["null", type]`
- MUST include `required: [unique_key]`
- MUST include `additionalProperties: true`
- **Dynamic-key objects**: when an object uses data-driven keys (dates, IDs, locales) instead of fixed field names, define it as `type: object` with `additionalProperties: true` and do NOT list sample keys in `properties` -- Builder's `autoImportSchema` will hardcode sample keys, which must be removed.

**BasicHttpAuthenticator warning**: when using `BasicHttpAuthenticator`, Builder auto-adds `username` and `password` to `spec.connection_specification`. These are Builder artifacts — they map from the authenticator config fields and should NOT be added to K8s Secrets. The real credential fields use source-specific prefixes (e.g. `bamboohr_api_key`).

MUST include:
- `check` block at the top with the lightest stream
- `definitions.linked` block with reusable components (auth, paginator) using granular `$ref`
- `streams` at root level with `transformations` containing `AddFields` (with `AddedFieldDefinition` type on each item)
- `concurrency_level` section
- `metadata` section with `autoImportSchema`
- `spec.connection_specification` with `insight_tenant_id` and `insight_source_id` as required fields
- All config fields with source-specific prefixes (e.g. `azure_*`, `github_*`, `jira_*`)
- `InlineSchemaLoader` with schema following Builder conventions (see above)
- Incremental sync with computed dates (no config params for start/end)

MUST NOT:
- Use whole-object `$ref` (`#/definitions/auth`, `#/definitions/paginator`, `#/streams/N`, `#/definitions/add_fields`). Builder strict validator only accepts granular leaf-field `$ref` into `definitions.linked.<Component>/<field>`. For substream parents (`parent_stream_configs[0].stream`) and any object that cannot be leafed, inline the full definition or duplicate.
- Put template strings in request params that collide with API datetime dialects. YouTrack `updated: ` expects ISO 8601 with `T` separator, no braces, no spaces. Jira JQL expects `"YYYY-MM-DD HH:MM"` with space, no T. Always run `source.sh check <tenant>` against a real instance to confirm.
- Convert epoch-millisecond cursor fields via a transformation like `"{{ format_datetime(record['updated'] / 1000, '%Y-%m-%dT%H:%M:%S') }}"` in `AddedFieldDefinition.value`. The value does not reliably interpolate before `cursor.observe()` sees it, and you'll get a runtime error with the literal Jinja template as the cursor value. Use Airbyte's native `%ms` (or `%s`, `%s_as_float`, `%epoch_microseconds`) token in `DatetimeBasedCursor.cursor_datetime_formats` instead. See `src/ingestion/tools/declarative-connector/README.md` §"Epoch millisecond cursors" for the exact pattern.
- Route substreams through nullable parent fields (e.g. `parent_key: id_readable` / `record.get('idReadable')`). Use the parent's stable internal id (`record['id']`, surfaced as `youtrack_id` etc.). A `null` from the API silently routes to `.../None/<endpoint>` which 404s and drops the entire partition.
- Use a heavy stream (large per-record payloads, e.g. Jira `fields=*all` + `expand=names` ≈ 2 MB/response on ~1000-field instances) as a `SubstreamPartitionRouter` parent. The CDK **auto-caches every parent's HTTP responses in a SQLite requests-cache**; with multi-MB responses the cache balloons (observed: 226 MB after ~108 responses) and the read stalls silently — job stays "running", CPU busy, **0 records emitted, forever**. Instead, split roles: add a dedicated lightweight key-enumeration parent stream that requests a minimal field set (e.g. `fields: updated` — id/key arrive top-level for free) and point all children at it; keep the full-payload stream as a plain emitter (non-parent streams are not cached). This mirrors the official `source-jira` (`board_issues` parent uses `fields: ['key','created','updated']`; only the terminal `issues` emitter uses `*all`). Reference implementation: `jira_issue_keys` in `task-tracking/jira/connector.yaml`.

- Set `concurrency_level.default_concurrency: 1`. With a single worker the concurrent CDK **self-deadlocks** once one sync generates ≥ ~10k partitions (the CDK futures limit): the only worker thread runs `generate_partitions` and throttles on "futures limit reached" (`partition_enqueuer.py`), while the partition-read futures it waits on have no other worker to run them. Symptom: CPU-busy spin, zero I/O, zero records, forever — and the records counter freezing at exactly 10000 is the fingerprint. Hit in production by jira (`jira_issue_history` full-window fan-out) and confluence (`wiki_page_versions`). Use `default_concurrency: 4` (verified: the same full-window run that deadlocked at 1 emitted 4208 records in 75 s at 4).

Substream-parent rules (each one was a production incident):
- Reconcile auto-selects **every** discovered stream (ADR-0015) — there is no "helper, don't sync it" escape for top-level streams. A lightweight parent defined at top level WILL land as a bronze table, so it MUST carry the standard `tenant_id`/`source_id`/`unique_key` stamp and have a `promote_bronze_to_rmt` line like every other stream. Alternative: define the parent **inline inside `partition_router.parent_stream_configs[].stream`** (see `_scrum_boards` inside jira's `jira_sprints`) — inline parents are invisible to `discover`, so no table appears; they are still response-cached, so keep them lightweight too.
- The `cursor_field` of `DatetimeBasedCursor` is read from the **top level** of the emitted record. If the API nests it (Jira `/search/jql` returns `fields.updated`), hoist it via `AddFields` (`value: "{{ (record.get('fields') or {}).get('updated') }}"`) — otherwise the cursor never observes values and state never advances, silently re-reading the full window every sync.

NOTE on integer-typed slots: Airbyte declarative CDK accepts BOTH integers AND Jinja-interpolated strings for `OffsetIncrement.page_size`, `CursorPagination.page_size`, and similar slots — `page_size: "{{ config.get('x_page_size', 100) }}"` is valid and recommended for config-driven pagination. (Earlier guidance in this file was wrong; the strict validator's "literal integer" rejection in our YouTrack work was a downstream symptom of a different `$ref` issue, not of templated page_size.)

See `src/ingestion/tools/declarative-connector/README.md` for the full Builder-UI rules list and datetime pitfalls.

#### 3.2 `descriptor.yaml`

```yaml
name: <connector_name>
version: "1.0"

schedule: "0 2 * * *"
dbt_select: "tag:<slug>+"          # <slug> = this descriptor's `name`, hyphen and all (see §3.5 NAMING)
workflow: sync

connection:
  namespace: "bronze_<snake>"      # <snake> = <slug> with '-' → '_'
```

All streams from the manifest are synced. Sync mode is auto-detected by Airbyte discover (`incremental` if supported, otherwise `full_refresh`).

#### 3.3 K8s Secret example — `src/ingestion/secrets/connectors/<name>.yaml.example`

All connector credentials are stored as K8s Secrets, not inline in tenant YAML. Create the example file:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-<connector_name>-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: <connector_name>
    insight.cyberfabric.com/source-id: <connector_name>-main
type: Opaque
stringData:
  <prefix>_field1: "CHANGE_ME"
  <prefix>_field2: "CHANGE_ME"
```

Rules:
- File goes to `src/ingestion/secrets/connectors/<name>.yaml.example` (committed to git)
- Real secrets go to `src/ingestion/secrets/connectors/<name>.yaml` (gitignored)
- Secret name pattern: `insight-<connector_name>-<source_id_suffix>`
- Labels: `app.kubernetes.io/part-of: insight`
- Annotations: `insight.cyberfabric.com/connector: <name>`, `insight.cyberfabric.com/source-id: <name>-main`
- `stringData` keys MUST match `spec.connection_specification` property names (with source-specific prefixes)
- Do NOT include `insight_tenant_id` or `insight_source_id` — these are injected by `connect.sh`
- Do NOT include `username`/`password` if using `BasicHttpAuthenticator` — these are Builder artifacts

#### 3.4 `README.md` — Connector documentation

```markdown
# <Connector Name> Connector

<One-line description of what data this connector extracts and the auth method.>

## Prerequisites

1. <How to get credentials from the source system>

## K8s Secret

\`\`\`yaml
<Full K8s Secret YAML — same as the .yaml.example>
\`\`\`

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `<prefix>_field` | Yes/No | <description> |

> **Note on `username` / `password` spec fields.** (only if BasicHttpAuthenticator)
> <explanation that these are Builder artifacts>

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

Create `src/ingestion/secrets/connectors/<name>.yaml` (gitignored) from the example:

\`\`\`bash
cp src/ingestion/secrets/connectors/<name>.yaml.example src/ingestion/secrets/connectors/<name>.yaml
# Fill in real values, then apply:
kubectl apply -f src/ingestion/secrets/connectors/<name>.yaml
\`\`\`

## Streams

| Stream | Description | Sync Mode |
|--------|-------------|-----------|

## Silver Targets

- `class_<domain>` — <description>
```

#### 3.5 `dbt/<connector_name>__<domain>.sql`

> **NAMING — two forms, do not mix them up (this bites hyphenated connectors):**
> - **slug** = the descriptor `name` exactly, e.g. `zulip-proxy` (keep the hyphen). This is also the Argo `data_source`.
> - **snake** = slug with `-`→`_`, e.g. `zulip_proxy`.
>
> Use **snake** for: model file prefix (`<snake>__<domain>.sql`), `source('bronze_<snake>', …)`, the `bronze_<snake>` namespace, and `source_type='<snake>'`.
> Use **slug** for: the **first dbt tag** on every model (`tags=['<slug>', …]`) and the descriptor `dbt_select: tag:<slug>+`.
>
> WHY the tag must be the slug, not snake: the `ingestion-pipeline` workflow's silver step (`transform-legacy`) HARDCODES its selector as `tag:{{data_source}}+` (= `tag:<slug>+`) and **ignores** descriptor `dbt_select`. A snake-cased tag (`zulip_proxy`) is then never selected → dbt logs `Nothing to do`, silver tables silently never build. For a hyphen-free connector (e.g. `m365`) slug == snake so it doesn't matter; for `zulip-proxy`/`claude-team`/`ms-entra` it does.

```sql
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    schema='staging',
    tags=['<slug>', 'silver:class_<domain>']
) }}

SELECT
    tenant_id,
    source_id,
    unique_key,
    -- source-specific field mappings
    '<connector_name>' AS source
FROM {{ source('<connector_name>', '<stream_name>') }}
{% if is_incremental() %}
WHERE <cursor_field> > (SELECT max(<mapped_field>) FROM {{ this }})
{% endif %}
```

#### 3.6 `dbt/schema.yml`

Define source (bronze database) and model with tests:
- `tenant_id`: not_null
- `source_id`: not_null
- `unique_key`: not_null, unique

#### 3.6b Identity Resolution inputs (REQUIRED when the source exposes a user directory)

If the connector has a stream that enumerates users **with emails** (or another
person-identifying value like employee_id), it MUST feed Identity Resolution
via the standard three-macro chain. Reference implementations:
`collaboration/zoom`, `collaboration/zulip-proxy`, `hr-directory/bamboohr`,
`hr-directory/ms-entra`, `wiki/outline`.

```
<users bronze table>
  -> <snake>__users_snapshot        snapshot() macro — SCD2, appends a row when
                                    tracked columns change
    -> <snake>__users_fields_history fields_history() macro — one row per changed
                                    field per version transition
      -> <snake>__identity_inputs   identity_inputs_from_history() macro —
                                    UPSERT/DELETE observation rows, tagged
                                    silver:identity_inputs
        -> identity.identity_inputs silver/_shared/identity_inputs.sql unions
                                    all contributors via union_by_tag
```

Three models to create. File names / refs use **snake**; the FIRST tag on each
model uses the **slug** (hyphenated `name`) — see §3.5 NAMING. (`<snake>` and
`<slug>` differ only for hyphenated connectors.)

```sql
-- dbt/<snake>__users_snapshot.sql
-- depends_on: {{ ref('<snake>__bronze_promoted') }}
{{ config(materialized='incremental', incremental_strategy='append',
          schema='staging', tags=['<slug>']) }}
{{ snapshot(
    source_ref=source('bronze_<snake>', '<users_stream>'),
    unique_key_col='unique_key',
    check_cols=['<email_field>', '<display_name_field>', '<status_field>']
) }}
```

```sql
-- dbt/<snake>__users_fields_history.sql
{{ config(materialized='table', schema='staging', tags=['<slug>', 'silver']) }}
{{ fields_history(
    snapshot_ref=ref('<snake>__users_snapshot'),
    entity_id_col='toString(<source_user_id_col>)',   -- MUST yield String — see Rules below
    fields=['<email_field>', '<display_name_field>', '<status_field>']
) }}
```

```sql
-- dbt/<snake>__identity_inputs.sql
{{ config(materialized='incremental', incremental_strategy='append',
          schema='staging', tags=['<slug>', 'silver', 'silver:identity_inputs']) }}
{{ identity_inputs_from_history(
    fields_history_ref=ref('<snake>__users_fields_history'),
    source_type='<snake>',
    identity_fields=[
        {'field': '<email_field>', 'value_type': 'email',        'value_field_name': 'bronze_<snake>.<users_stream>.<email_field>'},
        {'field': '<name_field>',  'value_type': 'display_name', 'value_field_name': 'bronze_<snake>.<users_stream>.<name_field>'},
    ],
    deactivation_condition="field_name = '<status_field>' AND lower(new_value) = '<inactive_value>'"
) }}
```

Rules:
- `entity_id_col` is the SOURCE-side stable user id (e.g. `user_id`, `id`) —
  the macro emits the canonical `value_type='id'` binding row from it
  automatically (ADR-0002); do NOT list it in `identity_fields`.
- `entity_id_col` MUST evaluate to a **String**. The macro emits `entity_id`
  into the `value` column, which is `String` in every other branch — if the
  source id is numeric (ClickHouse `Decimal`/`Int`, common when the bronze
  field is a JSON number, e.g. zulip-proxy `id`), the final `UNION ALL` fails
  with `Code 386 NO_COMMON_TYPE: no supertype for String, Decimal(…)`. Wrap
  numeric ids: `entity_id_col='toString(id)'`. (A natively-string id like a
  GUID or `unique_key` needs no wrap — that's why zoom/ms-entra pass bare `id`.)
- `check_cols` (snapshot) and `fields` (fields_history) must be the same list —
  every field whose change should produce a history row. Booleans are fine:
  `fields_history` stringifies via `toString()`, so a ClickHouse Bool becomes
  `'true'`/`'false'` — compare with `lower(new_value) = 'true'` in
  `deactivation_condition`.
- The snapshot reads its source with `FINAL`, so the bronze table MUST already
  be RMT-promoted (`<snake>__bronze_promoted` + the `-- depends_on` header on
  the snapshot model).
- Add `-- depends_on: {{ ref('<snake>__identity_inputs') }}` to
  `src/ingestion/silver/_shared/identity_inputs.sql` so the first run
  materialises the staging model before the shared union.
- Document the chain and contributed value types in the connector README.
- If the source exposes NO user directory (e.g. Confluence — author ids only,
  no emails), skip this section and document in the README how identities
  resolve instead (cross-connector JOIN, Silver Step 2 direct mapping).

#### 3.6c Surfacing the connector in a dashboard metric (silver class → gold → query_ref → catalog)

> **A connector that only reaches silver does NOT appear in the UI.** Tagging a
> model `silver:class_<X>` makes `union_by_tag` fold it into the shared
> `silver.class_<X>` table — and that is ALL that is automatic. The gold views,
> the metric `query_ref`s, and the metric catalog each enumerate their inputs
> EXPLICITLY; a new source contributes nothing past silver until you wire four
> more layers by hand. Skip this whole section if the connector is bronze-only
> or its silver class is not consumed by any gold metric (then say so in the
> README). Reference implementation: PR constructorfabric/insight#1466 (added
> Zulip chat to the Collaboration bullet) — open it as a worked example.

The data path for a per-person dashboard metric:

```
bronze_<snake>.<stream>
  → <snake>__<class>.sql            (silver, tag silver:class_<X>)   ← §3.5
    → silver.class_<X>              (union_by_tag — AUTOMATIC)
      → insight.<section>_bullet_rows   (gold VIEW, CH migration)    ← layer A
        → metrics.query_ref (…IC/Team/member/dept)  (SeaORM)         ← layer B
          → metric_catalog row      (label/threshold, SeaORM)        ← layer C
            → analytics-api → person-profile <Section> card (UI)
```

What to add (use an EXISTING section like `collab` / `git` / `ai` / `support`
when the metric fits one; only create a new `<section>_bullet_rows` view +
metric ids for a genuinely new section):

- **Layer 0 — silver class build edge.** Add
  `-- depends_on: {{ ref('<snake>__<class>') }}` to
  `src/ingestion/silver/<domain>/class_<X>.sql`. `union_by_tag` already includes
  your model at SQL level once its table exists; this edge makes a
  `tag:<slug>+` prod run rebuild the class (and the e2e rig's `derive_selectors`
  build it from the `silver:` tag).
- **Layer A — gold view branch.** In the CH migration that defines
  `insight.<section>_bullet_rows`
  (`src/ingestion/scripts/migrations/*-bullet-rewrite.sql`; idempotent
  `DROP+CREATE`, no tracking table → edit the canonical definition in place),
  add a branch `... FROM silver.class_<X> WHERE data_source = 'insight_<snake>'`
  emitting your `metric_key`(s). The `data_source` literal is exactly what your
  silver model SELECTs (`'insight_<snake>'`). Join `insight.people` on
  `lower(email) = p.person_id` for `org_unit_id`.
- **Layer B — query_ref(s).** The bullet `query_ref`s materialize EVERY
  FE-visible `metric_key` as `sumIf(metric_value, metric_key='<k>') AS <k>_v`
  then unpivot via `ARRAY JOIN [('<k>', <k>_v), …]`. Add your key to BOTH lists.
  SeaORM migrations are append-only — write a NEW
  `m<date>_<n>_<section>_<conn>.rs` that `UPDATE metrics SET query_ref=… WHERE
  id = UNHEX('<hex>')`, basing the SQL on the LATEST migration that set that
  query_ref (grep the metric id; a section has SEVERAL copies — IC bullet,
  Team bullet, member-values `…0041`, dept-distribution `…0045` — update each
  surface you need). Register the migration in
  `src/backend/services/analytics-api/src/migration/mod.rs`.
- **Layer C — catalog row.** New append-only migration
  `m<date>_<n>_seed_<conn>_<section>_catalog.rs` inserting a `metric_catalog`
  row (`<section>_bullet_rows.<metric_key>`, label, sublabel, unit,
  `source_tags: ["<slug>"]`) + a product-default `metric_threshold`. Model it on
  `m20260620_000002_seed_wiki_catalog.rs`. Register in `mod.rs`.

Then prove the whole chain with a `/metric-test` fixture (see that skill) —
seed bronze, query the bullet metric, assert your `metric_key`'s value. Adding a
key to a shared section raises its `size(items)`, so bump any sibling test that
asserts the old count.

### For CDK (`CONNECTOR_TYPE=cdk`):

Create Python scaffold:

#### 3.1 `src/source_<name>/__init__.py`
#### 3.2 `src/source_<name>/source.py`

```python
from airbyte_cdk.sources import AbstractSource

class Source<Name>(AbstractSource):
    def check_connection(self, logger, config):
        # Validate credentials
        ...

    def streams(self, config):
        tenant_id = config["insight_tenant_id"]
        source_id = config["insight_source_id"]
        return [
            Stream1(tenant_id=tenant_id, source_id=source_id, ...),
        ]
```

Each stream MUST inject `tenant_id`, `source_id`, `unique_key` in `parse_response()`:

```python
def parse_response(self, response, **kwargs):
    for record in response.json()["data"]:
        record["tenant_id"] = self.tenant_id
        record["source_id"] = self.source_id
        record["unique_key"] = f"{self.tenant_id}-{self.source_id}-{record['id']}"
        yield record
```

#### 3.3 `src/source_<name>/schemas/<stream>.json`
#### 3.4 `setup.py`
#### 3.5 `Dockerfile`
#### 3.6 Same descriptor.yaml, K8s Secret example, README.md, dbt/ as nocode

## Phase 3.7: Image-bearing connector requirements (CRITICAL when Dockerfile present)

If your connector ships at least one `Dockerfile` under its connector directory (CDK source, enrich sidecar, future bootstrap or migrator container), you MUST declare every such image in your `descriptor.yaml.images:` block. CI uses **dynamic discovery** — it scans every descriptor on every run and builds whatever is declared. Adding a new connector with images is a descriptor edit; CI follows automatically.

See [ADR-0016 — Descriptor `images:` Block](../../../../docs/components/airbyte-toolkit/specs/ADR/0016-descriptor-images-block.md) for the rationale.

### 1. Descriptor `images:` block (REQUIRED for every Dockerfile)

In `descriptor.yaml`, after `workflow:` and before `dbt_select:`, add a map-style `images:` block. The map's keys are free-form identifiers; the **reserved** keys `cdk` and `enrich` have runtime semantics (reconcile and the enrich workflow read them respectively).

```yaml
images:
  cdk:                                              # reserved key for CDK source images
    name: source-<connector>-insight                # GHCR image short name (no registry prefix, no tag)
    dockerfile: ./Dockerfile                        # leading "./" mandatory
    context: .                                      # build context relative to connector dir
    image: ""                                       # full ref; empty until first CI build
  enrich:                                           # reserved key for enrich sidecars
    name: insight-<connector>-enrich
    dockerfile: ./enrich/Dockerfile
    context: ./enrich
    image: ""
```

**Field semantics:**
- `name` — pushed to `${IMAGE_PREFIX}/<name>`.
- `dockerfile` — path RELATIVE to connector directory, with leading `./`.
- `context` — build context path, RELATIVE to connector directory, with leading `./` (`./` alone means connector root).
- `image` — full image reference. CI patches this field on every successful push. Empty string `""` is allowed for not-yet-published images.

**No top-level `cdk_image:` or `enrich_image:` fields.** ADR-0011 and ADR-0014 are SUPERSEDED; CI rejects any descriptor that carries these legacy fields.

### 2. CI build wiring (paths-trigger only — NO per-image job)

Build identity (`name`, `dockerfile`, `context`) is read from your descriptor at job time. You do NOT add a per-image job to `.github/workflows/build-images.yml` — the workflow's `discover` step finds your connector automatically.

What you DO need to add:

- In the workflow's `on.push.paths` and `on.pull_request.paths` lists, ensure `src/ingestion/connectors/<category>/<name>/**` is covered (typically already covered by `src/ingestion/**`).
- In the `changes` job's paths-filter, add a per-connector flag that ALSO excludes `descriptor.yaml` (recursion prevention — the bump-descriptors commit patches descriptor.yaml and the flag must NOT re-fire on it):

  ```yaml
  <slug>:
    - 'src/ingestion/connectors/<category>/<name>/**'
    - '!src/ingestion/connectors/<category>/<name>/descriptor.yaml'
  ```

  The `<slug>` is the connector's snake_case identifier.

- In the `discover` step's filter logic, ensure your connector's `images[].context` paths are evaluated against the changed-file list — `discover` builds the matrix only for entries whose `context` directory had changes.

If a connector has both `images.cdk` (`context: .`) and `images.enrich` (`context: ./enrich`), a change inside `enrich/**` rebuilds only the enrich image; a change elsewhere in the connector dir rebuilds only the cdk image; a change touching both areas rebuilds both. `discover` handles this with no per-connector code.

### 3. bump-descriptors invocation (typically no skill action needed)

The `bump-descriptors` job uses the same `discover` output to know which descriptors to patch and which keys to update. Patching is universal: for every entry that built this run, it (1) patches `images.<key>.image` with the new full ref AND (2) bumps `descriptor.version` by one minor increment (X.Y.Z → X.(Y+1).0).

```bash
# pseudo-code; the actual implementation lives in build-images.yml
for entry in discover.matrix:
  if entry.built_in_this_run:
    yq -i ".images.${entry.key}.image = \"${IMAGE_PREFIX}/${entry.name}:${BUILD_TAG}\"" \
      "${entry.connector_dir}/descriptor.yaml"

# then dedupe by connector_dir and bump version once per descriptor:
for connector_dir in unique(discover.matrix[].connector_dir):
  python3 .github/workflows/scripts/bump-descriptor-version.py \
    --descriptor "${connector_dir}/descriptor.yaml"
```

The minor version bump makes reconcile classify the diff as `bump_kind: minor` per ADR-0015 → catalog re-discovery on the next deploy, no `dbt --full-refresh`.

**Strict-semver gate (FATAL)**: `bump-descriptor-version.py` rejects values that aren't strict semver `MAJOR.MINOR.PATCH` — no leading zeros, no `v` prefix, no pre-release suffix, no two-segment forms. Date-style legacy values like `2026.05.04` and two-segment like `1.0` fail loud, halting the CI job. Your descriptor's `version:` MUST be on-spec before the first CI run; the `cfs validate` rule `connector-images-triad` covers this. If it slips through, the operator fixes the version manually and re-pushes.

No per-connector wiring in `bump-descriptors` itself. Your descriptor declaring `images:` plus a strict-semver `version:` IS your wiring.

### Doneness validation

After your descriptor edit lands, run:

```bash
cfs --json validate --artifact src/ingestion/connectors/<category>/<name>/descriptor.yaml
yq '.images' src/ingestion/connectors/<category>/<name>/descriptor.yaml   # confirm block present
yq '.cdk_image, .enrich_image' src/ingestion/connectors/<category>/<name>/descriptor.yaml   # MUST return null (no top-level legacy fields)
```

The deterministic rule `connector-images-triad` (see `workflows/validate.md`) checks the descriptor and the paths-filter together.

## Phase 4: Validate Package Structure

After creating all files, run:
```
/connector validate <name>
```

## Phase 5: Local Testing (MANDATORY before Airbyte)

All testing MUST happen locally first via `source.sh` before uploading to Airbyte.

**Airbyte Builder note**: after importing/exporting via Builder, expect these changes to the manifest:
- `username` and `password` fields added to `spec.connection_specification` (Builder artifact from `BasicHttpAuthenticator` — expected and harmless)
- Schema `$schema` normalized to `http://json-schema.org/schema#`
- Field types reordered to `[type, "null"]`
- `metadata.testedStreams` section added with stream hashes

These are normal Builder behaviors, not errors.

### 5.1 Create K8s Secret with credentials

```bash
cp src/ingestion/secrets/connectors/<name>.yaml.example src/ingestion/secrets/connectors/<name>.yaml
# Edit with real credential values
kubectl apply -f src/ingestion/secrets/connectors/<name>.yaml
```

### 5.2 Validate manifest structure (no API call)

Run **both** validators, in order — never skip `validate-strict`. **Working directory** for every command in §5.2–§5.5: `src/ingestion/` (path arguments like `<category>/<name>` are resolved relative to `src/ingestion/connectors/`):

```bash
# cd src/ingestion
./tools/declarative-connector/source.sh validate-strict <category>/<name>   # Builder-UI compat
./tools/declarative-connector/source.sh validate        <category>/<name>   # CDK runtime
```

If `validate-strict` fails, do NOT proceed. Fix per-path errors first — the Builder UI will reject the manifest otherwise. Common `validate-strict` errors and fixes are listed in `src/ingestion/tools/declarative-connector/README.md` §"Debugging strict-validation errors".

If `validate-strict` passes but `validate` fails, there is a runtime problem — usually a bad Jinja expression, a template reference to an undefined config key, or a `$ref` pointing at a path that does not exist.

### 5.3 Check credentials against API

```bash
./tools/declarative-connector/source.sh check <category>/<name> <tenant>
```

### 5.4 Discover streams and generate schema from real data

```bash
./tools/declarative-connector/source.sh discover <category>/<name> <tenant>
./airbyte-toolkit/generate-schema.sh <name>
```

This saves real JSON schemas to `connectors/<category>/<name>/schemas/`.

### 5.5 Update manifest with real schema

Replace InlineSchemaLoader schemas in `connector.yaml` with the generated ones from `schemas/`.
Verify that all cursor fields exist in the schema (this prevents ClickHouse destination NPE).

### 5.6 Read data locally — per-stream smoke test (MANDATORY)

Working directory: `src/ingestion/`.

```bash
# cd src/ingestion
./tools/declarative-connector/generate-catalog.sh <category>/<name> <tenant>
```

Then read **each stream in isolation** — not just one combined read. `validate` and `validate-strict` are purely structural; **only a real `read` against the real API catches runtime pitfalls** that the Builder-UI (or Airbyte production) would otherwise hit. Known runtime-only landmines:

| Landmine | Symptom at `read` time | Fix |
|---|---|---|
| `step` without `cursor_granularity` in `DatetimeBasedCursor` | `ValueError: If step is defined, cursor_granularity should be as well and vice-versa` | Add `cursor_granularity: PT1S` (or appropriate ISO duration) alongside `step`. |
| `format_datetime(...)` inside `AddedFieldDefinition.value` for a cursor transformation | `ValueError: No format in [...] matching {{ format_datetime(record['x']/1000, ...) }}` — literal Jinja template stored as record value | Do not transform the cursor field. Use the native `%ms` / `%s` / `%s_as_float` / `%epoch_microseconds` tokens in `cursor_datetime_formats` to parse epoch values directly. |
| `record.get('X', {}).get('Y')` when `record['X']` is present but `null` | `jinja2.exceptions.UndefinedError: 'None' has no attribute 'get'` — defaults on `.get()` only apply to **missing** keys, not `None` values | Replace with `(record.get('X') or {}).get('Y')`. Use the same pattern for every chain that may hit a nullable parent object. |
| Source API query syntax (e.g. YouTrack `updated:`, Jira JQL, Salesforce SOQL) does not match your template | HTTP 400 `invalid_query` from the source | Never trust documentation alone — run `check` against a live tenant and inspect the generated URL. Each API has its own datetime dialect. See `src/ingestion/tools/declarative-connector/README.md` §"Datetime syntax pitfalls". |
| Heavy `SubstreamPartitionRouter` parent (multi-MB responses, e.g. `fields=*all`) | `read` stalls **silently**: CPU stays busy, 0 records emitted for minutes/hours, no error. The parent's SQLite requests-cache (`/tmp/tmp*/<parent_stream>.sqlite` inside the runner) grows to hundreds of MB, then freezes | Split roles: lightweight key-enumeration parent (minimal `fields` request param) + full-payload emitter that is not a parent. Diagnose by watching the cache file size and records-emitted count during `read` — healthy parents stay in the KB range. |
| `cursor_field` nested in the API response (e.g. Jira `fields.updated`) and not hoisted to the top level | No crash; first read looks fine, but no `STATE` cursor progress — every sync re-reads the full window (resume read returns the same count as the first read) | Add an `AddFields` hoist for the cursor field; include it in the inline schema (`required` if always present). |

**Per-stream `read` pattern** (for thorough testing — saves the full catalog, swaps in single-stream catalog, resets state, runs `read`, captures the emitted `STATE`, then runs `read` a second time to verify cursor advancement). **Working directory: repo root** — the `INGESTION` variable below makes the script independent of `cd` location:

```bash
# cd <repo root>
INGESTION=src/ingestion
CONNECTOR_PATH=<category>/<name>            # e.g. task-tracking/youtrack
CONN=$INGESTION/connectors/$CONNECTOR_PATH
CONNECTOR_NAME=$(basename "$CONN")
TENANT=<tenant>                              # e.g. example-tenant

cp "$CONN/configured_catalog.json" "$CONN/configured_catalog.json.bak"
for stream in $(jq -r '.streams[].stream.name' "$CONN/configured_catalog.json.bak"); do
  # Build single-stream catalog
  jq --arg s "$stream" '.streams |= map(select(.stream.name == $s))' \
     "$CONN/configured_catalog.json.bak" > "$CONN/configured_catalog.json"
  echo '[]' > "$CONN/state.json"

  echo "=== $stream ==="
  log=/tmp/${CONNECTOR_NAME}_${stream}.log

  # First read: from empty state.
  ( bash $INGESTION/tools/declarative-connector/source.sh read "$CONNECTOR_PATH" "$TENANT" > "$log" 2>&1 ) &
  pid=$!; ( sleep 120; kill -TERM $pid 2>/dev/null ) & killer=$!
  wait $pid 2>/dev/null; kill -TERM $killer 2>/dev/null

  # Capture last emitted STATE message and write to state.json so the next read resumes from it.
  python3 - "$log" "$CONN/state.json" <<'PY'
import json, sys
states = []
with open(sys.argv[1]) as f:
    for line in f:
        try: msg = json.loads(line)
        except json.JSONDecodeError: continue
        if msg.get("type") == "STATE":
            states.append(msg.get("state", msg))
if states:
    with open(sys.argv[2], "w") as out:
        json.dump([states[-1]], out)
PY

  # Count records + errors from first read.
  python3 -c "
import json
recs = 0; errs = []
for line in open('$log'):
    try: o = json.loads(line)
    except: continue
    if o.get('type') == 'RECORD': recs += 1
    elif o.get('log',{}).get('level') in ('ERROR','FATAL'): errs.append(o['log']['message'][:300])
print(f'  first read:  records={recs}, errors={len(errs)}')
for e in errs[:2]: print(f'    {e}')
"

  # Second read: with persisted state — for incremental streams this must produce a strict subset
  # (often zero records). For full-refresh streams the count typically stays the same.
  log2=/tmp/${CONNECTOR_NAME}_${stream}_resume.log
  ( bash $INGESTION/tools/declarative-connector/source.sh read "$CONNECTOR_PATH" "$TENANT" > "$log2" 2>&1 ) &
  pid=$!; ( sleep 60; kill -TERM $pid 2>/dev/null ) & killer=$!
  wait $pid 2>/dev/null; kill -TERM $killer 2>/dev/null
  python3 -c "
import json
recs = 0; errs = 0
for line in open('$log2'):
    try: o = json.loads(line)
    except: continue
    if o.get('type') == 'RECORD': recs += 1
    elif o.get('log',{}).get('level') in ('ERROR','FATAL'): errs += 1
print(f'  resume read: records={recs}, errors={errs}')
"
done
cp "$CONN/configured_catalog.json.bak" "$CONN/configured_catalog.json"
rm "$CONN/configured_catalog.json.bak"
```

Acceptance criteria for each stream:
- [ ] First-read record count > 0 (unless source genuinely has no data — rare).
- [ ] Error count = 0 in both runs (any `ERROR` / `FATAL` log message is a runtime bug — fix before deploy).
- [ ] Every emitted record has `tenant_id`, `source_id`, `unique_key`.
- [ ] For substreams, parent records are enumerated first and child records reference valid parent ids (use the parent's stable internal id field — e.g. `youtrack_id` from `record['id']` — for routing, NOT a nullable `record.get('idReadable')`-style field, since YouTrack/Jira can return `null` for human-readable IDs in some payloads).
- [ ] Every `SubstreamPartitionRouter` parent requests a minimal field set; during a substream's `read`, the parent's SQLite requests-cache stays in the KB range (a cache growing to tens/hundreds of MB means a heavy parent — restructure before deploy, see MUST NOT list above).
- [ ] For incremental streams, the **resume read** (second run, with the captured STATE persisted in `state.json`) returns a strict subset of the first-read records — usually zero. If the resume read returns the same count as the first read, the cursor is not advancing and the manifest is broken.

If any stream fails, do NOT deploy. Fix the manifest and re-run both `validate-strict` and the per-stream `read`.

### 5.7 Only then deploy to Airbyte

```bash
/connector deploy <name>
```

## Phase 6: Summary

```
Connector package created and tested: src/ingestion/connectors/<category>/<name>/

Completed:
  ✓ Package structure validated
  ✓ K8s Secret created and applied
  ✓ Credentials checked against API
  ✓ Streams discovered, schema generated from real data
  ✓ Data read locally — all mandatory fields present

If your connector ships a Dockerfile (CDK or enrich sidecar), also verify
the descriptor + CI wiring (per Phase 3.7 and ADR-0016):
  ✓ `images:` map-style block added to descriptor.yaml (key per Dockerfile,
       4 fields each: name/dockerfile/context/image)
  ✓ NO top-level `cdk_image:` or `enrich_image:` fields in descriptor.yaml
       (these were removed by ADR-0016 superseding ADR-0011 and ADR-0014)
  ✓ paths-filter for the connector slug in .github/workflows/build-images.yml
       excludes `descriptor.yaml` (recursion prevention)
  ✓ Build, push, bump, chart-publish all done by the workflow's shared
       discover/build/bump-descriptors jobs — NO per-connector job edits
       in the workflow YAML

Next: /connector deploy <name>
```

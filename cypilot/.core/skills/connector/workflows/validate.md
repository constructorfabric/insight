---
name: connector-validate
description: "Validate an Insight Connector package against spec"
---

# Validate Connector

Checks that a connector package meets all requirements from the connector spec.

## Step 1: Automated structural validation (MANDATORY)

Before the checklist review, always run the automated validators:

```bash
./src/ingestion/tools/declarative-connector/source.sh validate-strict <category>/<name>
./src/ingestion/tools/declarative-connector/source.sh validate        <category>/<name>
```

- `validate-strict` — runs the Airbyte Builder UI JSON-schema check (no `$ref` resolution). This is the definitive compat test for the Builder UI. Must exit 0.
- `validate` — runs the CDK loader check (resolves `$ref` first). Lenient; must also exit 0.

If either fails, fix the reported per-path errors before proceeding with the checklist. See `src/ingestion/tools/declarative-connector/README.md` §"Debugging strict-validation errors".

## Step 2: Builder-UI compatibility checklist (manifest-only)

If `validate-strict` passed, these are already satisfied automatically — but eyeball them when reviewing a PR to catch intent mistakes:

- [ ] No whole-object `$ref` to `#/definitions/<X>` or `#/streams/<N>`. Only leaf-field `$ref` into `#/definitions/linked/<Component>/<field>` is allowed.
- [ ] Every `AddFields.fields[]` item has `type: AddedFieldDefinition`.
- [ ] `concurrency_level.default_concurrency` is a literal integer **≥ 2** (with 1 worker the concurrent CDK self-deadlocks at ≥ ~10k partitions — futures-limit throttle with no free worker; fingerprint: records counter frozen at exactly 10000, CPU-busy, zero I/O).
- [ ] `page_size` in `OffsetIncrement` / `CursorPagination` is either a literal integer OR a Jinja template like `"{{ config.get('x_page_size', 100) }}"` (both forms are accepted by the CDK and the Builder UI). Wire declared `*_page_size` config keys via the templated form so operator overrides take effect.
- [ ] Schema `$schema` is `http://json-schema.org/schema#` (not draft-07).
- [ ] Schema type arrays are `[type, "null"]`, not `["null", type]`.
- [ ] `check` block is present and placed BEFORE `definitions`.
- [ ] `version`, `type: DeclarativeSource`, `concurrency_level`, `metadata.autoImportSchema` present.
- [ ] Did NOT copy from `task-tracking/jira` (jira uses whole-object `$ref`; it is a known anti-template).

## Step 2b: Runtime-only pitfalls (checked by per-stream `read`, MANDATORY)

`validate-strict` does not catch these — only a live `read` against a real tenant does. Fail the connector review if any of these are present:

- [ ] `DatetimeBasedCursor` with `step` also has matching `cursor_granularity`. Missing `cursor_granularity` → CDK raises `ValueError: If step is defined, cursor_granularity should be as well`.
- [ ] No `format_datetime(...)` call inside an `AddedFieldDefinition.value` used as a cursor source. That Jinja expression may not render, leaving the literal template as the cursor value. Use native `%ms` / `%s` / `%s_as_float` / `%epoch_microseconds` in `cursor_datetime_formats` to parse epoch values directly from the source field.
- [ ] Every `record.get('X', {}).get('Y')` chain is replaced with `(record.get('X') or {}).get('Y')`. The `.get(key, default)` default only applies when the key is **missing**; it does NOT apply when the key is present with `null` value, and `None.get(...)` crashes the whole slice.
- [ ] Source API query syntax has been verified against a real tenant via `source.sh check`. YouTrack, Jira JQL, Salesforce SOQL each have distinct datetime and operator dialects — template substitution can produce syntactically valid but semantically wrong queries that `validate-strict` cannot detect.
- [ ] No `SubstreamPartitionRouter` parent is a heavy stream (e.g. `fields=*all` / multi-MB responses). The CDK auto-caches parent HTTP responses in SQLite; a heavy parent balloons the cache (observed: 226 MB → silent permanent stall, job "running" with 0 records). Parents must request a minimal field set (pattern: dedicated key-enumeration stream like jira's `jira_issue_keys`, or an inline parent like `_scrum_boards`).
- [ ] Every `DatetimeBasedCursor.cursor_field` exists at the **top level** of the emitted record. If the API nests it (Jira `fields.updated`), an `AddFields` hoist is present — otherwise state never advances and every sync silently re-reads the full window (detectable: resume read returns the same count as the first read).

## Step 2c: Per-stream `read` smoke test (MANDATORY)

Run the per-stream `read` loop from `connector-create.md` §5.6 and verify, for every stream:

- [ ] First-read record count > 0 (unless the source truly has no data).
- [ ] Error count = 0 in both first read and resume read (any `ERROR` / `FATAL` in the log is a blocker).
- [ ] Every emitted record contains `tenant_id`, `source_id`, `unique_key`.
- [ ] For substreams, the parent_key/partition_field uses the parent's stable internal id (e.g. `youtrack_id` from `record['id']`), NOT a nullable human-readable field like `id_readable` from `record.get('idReadable')` — a `null` value silently routes to `.../None/<endpoint>` which 404s and drops the slice.
- [ ] For incremental streams, a **resume read** (second run after capturing the emitted `STATE` message from stdout and writing it to `state.json`) returns a strict subset of the first-read records — usually zero. The skill's smoke-test script in `connector-create.md` §5.6 does this capture + persist + resume automatically. A naive "second consecutive read" without persisting state cannot validate cursor advancement (`source.sh read` writes Airbyte Protocol JSON to stdout but does not update `state.json` itself).

## Step 3: Spec-level checklist

Read connector package files and verify each item:

### Structure
- [ ] `connector.yaml` exists (nocode) or `Dockerfile` + `source_<name>/source.py` exists (CDK)
- [ ] `descriptor.yaml` exists with required fields (name, version, type, schedule, workflow, dbt_select, connection.namespace)
- [ ] `README.md` exists with prerequisites, K8s Secret fields, streams table, and multi-instance example
- [ ] K8s Secret example in `secrets/connectors/<name>.yaml.example` with `insight_source_id` annotation
- [ ] `dbt/` directory with at least one .sql model and schema.yml

### Manifest (nocode)
- [ ] `version: 7.0.4` or compatible
- [ ] `type: DeclarativeSource`
- [ ] `spec.connection_specification` has `insight_tenant_id` as required
- [ ] `spec.connection_specification` has `insight_source_id` as required
- [ ] All config fields use prefixes (insight_*, azure_*, github_*, etc.)
- [ ] No bare `tenant_id` or `client_id` in config fields
- [ ] AddFields includes `tenant_id` from `config['insight_tenant_id']`
- [ ] AddFields includes `source_id` from `config['insight_source_id']`
- [ ] AddFields includes `unique_key` with pattern: `{tenant_id}-{source_id}-{natural_key}`
- [ ] InlineSchemaLoader has `additionalProperties: true`
- [ ] Schema includes `tenant_id`, `source_id`, `unique_key` as string fields
- [ ] Nullable types used only where API actually returns null (not all fields)
- [ ] EVERY top-level stream — including lightweight substream parents added for cache hygiene — carries the full identity stamp and a `promote_bronze_to_rmt` line. Reconcile (ADR-0015) auto-selects every discovered stream, so "helper" top-level streams land as real bronze tables; without the stamp + RMT promotion they accumulate unbounded duplicates. Parent streams that must NOT become tables go inline inside `partition_router.parent_stream_configs[].stream` instead (invisible to discover).

### CDK (Python)
- [ ] `parse_response()` injects `tenant_id`, `source_id`, `unique_key`
- [ ] `unique_key` includes `tenant_id` and `source_id`
- [ ] `spec.json` has `insight_tenant_id` and `insight_source_id` as required
- [ ] All config fields in `spec.json` use source-specific prefixes (`insight_*`, `github_*`, `jira_*`, etc.)
- [ ] No bare field names (`token`, `client_id`, `tenant_id`, `start_date`, etc.) in `connectionSpecification.properties`

### Descriptor
- [ ] `name` matches directory name
- [ ] `version` is bumped in the SAME PR as any `connector.yaml` change — reconcile republishes the nocode manifest only on descriptor-version drift (equal versions → noop), so a manifest edit without a bump silently never reaches Airbyte
- [ ] `connection.namespace` = `bronze_<name>`
- [ ] `dbt_select` includes connector tag with `+` suffix (e.g., `tag:m365+`)
- [ ] `schedule` is valid cron expression
- [ ] `workflow` field is present
- [ ] No `streams` block (streams are owned by Airbyte connector, discovered via `airbyte discover`)
- [ ] No `silver_targets` block (Silver targets are determined by dbt model tags via `dbt_select`)

### Rule: `connector-images-block` (FATAL — when Dockerfile present)

**Applies to**: every connector directory under `src/ingestion/connectors/**/` that contains at least one `Dockerfile`. Nocode connectors (no Dockerfile) are exempt.

**Severity**: FATAL — failing this rule means the connector image will silently never be rebuilt in CI, OR reconcile will fail to register the connector. Validate exits 2 on failure.

**Check 1 — `descriptor.yaml.images:` is well-formed (map-style per ADR-0016)**

- `descriptor.yaml` MUST contain an `images:` block that is a YAML map (NOT a list).
- The map MUST have at least one key. Keys are free-form identifiers; the reserved keys `cdk` and `enrich` have runtime semantics.
- Each entry MUST have all four fields: `name`, `dockerfile`, `context`, `image`.
- `dockerfile` and `context` paths, joined to the connector directory, MUST resolve to a real file (`dockerfile`) and a real directory (`context`) on disk.
- `image` MUST be a string; empty string `""` is allowed for not-yet-published images. If non-empty, MUST be a full image reference (`registry/repo:tag` or `registry/repo@sha256:...`).

**Check 2 — No top-level legacy fields**

- `descriptor.yaml` MUST NOT contain top-level `cdk_image:` or `enrich_image:` keys. These were SUPERSEDED by ADR-0016. `yq -r '.cdk_image, .enrich_image' <descriptor>` MUST return `null` on both.

**Check 3 — paths-filter exclusion in CI**

- `.github/workflows/build-images.yml` `changes` job's paths-filter MUST contain an entry for the connector's snake_case slug whose includes match the connector dir AND explicitly exclude `descriptor.yaml`:

  ```yaml
  <slug>:
    - 'src/ingestion/connectors/<category>/<name>/**'
    - '!src/ingestion/connectors/<category>/<name>/descriptor.yaml'
  ```

  Without the exclusion, the descriptor-bump commit re-triggers the image build and the workflow loops forever.

**Check 4 — Reserved-key runtime requirements**

- If `images.cdk` is present and `image` is non-empty: reconcile reads it; no further action required.
- If `images.cdk` is present but `image` is empty: reconcile WARN+skips registration (acceptable for not-yet-built connectors).
- If `images.enrich` is present: at least one workflow template under `charts/insight/templates/ingestion/` MUST reference `<connector>_enrich_image` as a parameter (so reconcile's render step can propagate it).

**Check 5 — Strict semver `version:` (CI bump precondition)**

- Because the CI `bump-descriptors` job bumps `descriptor.version` by one minor every time an image rebuilds (per ADR-0016 + ADR-0015), the field MUST be on strict-semver form `MAJOR.MINOR.PATCH` from day one. The matcher is `python3 .github/workflows/scripts/bump-descriptor-version.py --descriptor <path> --print-only` succeeding (exit 0).
- Each of MAJOR, MINOR, PATCH MUST be `0` or a non-zero digit followed by more digits (no leading zeros — semver.org §2).
- NO `v` prefix, NO pre-release suffix, NO build metadata.
- Examples that PASS: `1.0.0`, `0.1.0`, `10.20.30`, `100.0.0`.
- Examples that FAIL: `2026.05.04` (leading zeros), `1.0` (two segments), `v1.0.0` (prefix), `1.0.0-rc1` (pre-release), `1.0` (any non-three-segment form).

**Output on failure** (one bullet per missing check):

- `Connector <name>: missing descriptor.images: block (must be a map with at least one key) — see ADR-0016.`
- `Connector <name>: top-level cdk_image:/enrich_image: still present — must be removed (ADR-0011/0014 SUPERSEDED).`
- `Connector <name>: images.<key> missing required field (name|dockerfile|context|image).`
- `Connector <name>: images.<key>.dockerfile/<context> does not resolve to an existing file/directory.`
- `Connector <name>: paths-filter for <slug> does not exclude descriptor.yaml; descriptor-bump commit will infinite-loop.`
- `Connector <name>: images.enrich present but no chart workflow template references <connector>_enrich_image parameter.`
- `Connector <name>: descriptor.version is not strict semver MAJOR.MINOR.PATCH — CI bump-descriptors will fail loud on next image rebuild. Got: '<value>'. Fix to e.g. "1.0.0".`

### dbt Models
- [ ] Model name follows `<connector>__<domain>.sql` pattern
- [ ] `materialized='incremental'`
- [ ] `schema='staging'`
- [ ] Tags include connector name and `silver:class_<domain>`
- [ ] SELECT includes `tenant_id`, `source_id`, `unique_key`
- [ ] Uses `{{ source('bronze_<name>', '<stream>') }}`
- [ ] Has `{% if is_incremental() %}` block

### Identity Resolution inputs

- [ ] If the connector ingests a user-directory stream with emails (or another
  person-identifying value), the three-macro chain is present:
  `<name>__users_snapshot` (snapshot) → `<name>__users_fields_history`
  (fields_history) → `<name>__identity_inputs` (identity_inputs_from_history,
  tagged `silver:identity_inputs`). See `connector-create.md` §3.6b.
- [ ] `src/ingestion/silver/_shared/identity_inputs.sql` carries a
  `-- depends_on: {{ ref('<name>__identity_inputs') }}` line for the connector.
- [ ] If the source has NO user directory, the README documents the
  alternative resolution path instead (e.g. Confluence → jira_user JOIN).

### dbt schema.yml
- [ ] Source defined with `schema: bronze_<name>`
- [ ] Model has `tenant_id` with not_null test
- [ ] Model has `source_id` with not_null test
- [ ] Model has `unique_key` with not_null and unique tests

### Bronze Promotion (`promote_bronze_to_rmt`)

Airbyte writes bronze tables as plain `MergeTree`, so full-refresh streams accumulate duplicates across syncs. Every connector with a `dbt/` directory MUST migrate its bronze tables to `ReplacingMergeTree` via the `promote_bronze_to_rmt` macro (see `dbt/macros/promote_bronze_to_rmt.sql`). The bootstrap view MUST run BEFORE all other transformations.

- [ ] `dbt/<connector_snake>__bronze_promoted.sql` exists (where `<connector_snake>` is the connector name with hyphens converted to underscores, e.g. `bitbucket_cloud`, `ms_entra`, `claude_enterprise`)
- [ ] The bootstrap model is `materialized='view'`, uses `schema='staging'`, and tags the connector name (`tags=['<name>']`)
- [ ] For every Airbyte stream the connector emits, there is a `{% do promote_bronze_to_rmt(table='bronze_<name_snake>.<stream>', order_by='unique_key') %}` line
- [ ] No spurious `promote_bronze_to_rmt` calls reference streams that don't exist in the manifest / source
- [ ] Every other dbt model in `dbt/` that reads from `source('bronze_<name_snake>', '...')` declares the bronze_promoted dependency as the FIRST non-blank line above the `config` block:
  ```jinja
  -- depends_on: {{ ref('<connector_snake>__bronze_promoted') }}
  ```
  This makes dbt's DAG materialise the bootstrap view before any model that reads bronze.

Run the deterministic checker:

```bash
./airbyte-toolkit/validate-bronze-promoted.py <category>/<connector>
# or for a CI-friendly summary across the whole repo:
./airbyte-toolkit/validate-bronze-promoted.py --all --json
```

Exit 0 = PASS for the targeted connector(s); exit 2 = at least one FAIL. Rule IDs:

| Rule | What it checks |
|------|----------------|
| `BP-1` | `<name>__bronze_promoted.sql` file exists |
| `BP-2` | bootstrap is `materialized='view'` |
| `BP-3` | bootstrap uses `schema='staging'` |
| `BP-4` | bootstrap tags include connector name |
| `BP-5` | every stream has a `promote_bronze_to_rmt(table='bronze_<name>.<stream>')` call |
| `BP-6` | no `promote_bronze_to_rmt` call references an unknown stream (WARN) |
| `BP-7` | `promote_bronze_to_rmt` calls target the connector's own bronze namespace |
| `BP-8` | every `promote_bronze_to_rmt` call passes `order_by` |
| `BP-9` | every other model that reads bronze depends on `<name>__bronze_promoted` |

### Credentials Template
- [ ] `credentials.yaml.example` lists all required fields
- [ ] `insight_source_id` is included
- [ ] No real credentials in any tracked file

## Output

```
=== Connector Validation: <name> ===

  Structure:    PASS (5/5)
  Manifest:     PASS (12/12)  or  CDK: PASS (5/5)
  Descriptor:   PASS (7/7)
  dbt Models:   PASS (7/7)
  dbt Schema:   PASS (4/4)
  Credentials:  PASS (3/3)

  Status: PASS
```

If any FAIL, show specific issue with file:line and fix suggestion.

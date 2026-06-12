# declarative-connector — local manifest runner

Runs Airbyte declarative-manifest connectors in Docker without the full Airbyte platform. Used for rapid manifest development, validation, and local end-to-end smoke tests before uploading to Airbyte.

## Commands

| Command | Needs creds? | When to use |
|---|---|---|
| `./source.sh validate <class>/<connector>` | no | CDK-runtime validation. Resolves `$ref` AND normalizes (auto-fills `type:` etc.) before checking. **Lenient** — passes manifests the Builder UI rejects. |
| `./source.sh validate-strict <class>/<connector>` | no | **Strict** Builder-UI validation — resolves `$ref`s with the CDK resolver, then validates against `declarative_component_schema.yaml` **without normalization**, emitting per-path errors. Run this **before** opening the manifest in the Airbyte Builder UI. |
| `./source.sh check <class>/<connector> <tenant>` | yes | Manifest + credentials smoke test against the source API. |
| `./source.sh discover <class>/<connector> <tenant>` | yes | List available streams and their schemas. |
| `./source.sh read <class>/<connector> <tenant>` | yes | Extract data (Airbyte Protocol JSON on stdout). |

`<class>/<connector>` is relative to `src/ingestion/connectors/`, e.g. `collaboration/m365` or `task-tracking/youtrack`.

## Validator image pin

All commands run inside `airbyte/source-declarative-manifest` pinned in `source.sh` (`BASE_IMAGE`) to the `airbyte-cdk` version bundled with the **deployed** Airbyte platform — the version in `airbyte-connector-builder-server/requirements.in` of the `airbyte-platform` release matching `AIRBYTE_VERSION` in `deploy/scripts/install-airbyte.sh`. Never switch the default to `:latest`: upstream tightens `declarative_component_schema.yaml` over time, so a newer schema rejects manifests the deployed Builder UI happily accepts (false `validate-strict` failures). When bumping `AIRBYTE_VERSION`, bump the pin in `source.sh` + `Dockerfile` in the same change; the local wrapper image tag derives from the pin, so the rebuild happens automatically.

## Validation ladder — always in this order

1. **`validate-strict`** — first. Catches Builder UI blockers (unresolvable `$ref`, missing `type: AddedFieldDefinition`, integer-only slots templated by mistake, bad `$schema` URL, etc.) early, before runtime wastes a round trip.
2. **`validate`** — second. Smoke-checks that the CDK loader accepts the manifest at runtime, after `$ref` resolution and normalization.
3. **`check <tenant>`** — third. Real credentials against the real API. Catches query-syntax errors and auth problems.
4. **`discover` / `read`** — fourth. Produces real records locally; feeds `generate-schema.sh`.

If any step in the ladder fails, fix the issue and restart from step 1. **Do not skip ahead** — a manifest that fails `validate-strict` may still pass `validate` but cannot be edited in the Builder UI.

## Builder-UI compatibility — hard rules

The Builder UI resolves `$ref`s with the CDK's `ManifestReferenceResolver` and then validates the resolved manifest against `declarative_component_schema.yaml` — but **without** the runtime's `ManifestNormalizer` pass that auto-fills missing `type:` fields and similar. `validate-strict` reproduces exactly that: resolve refs, then raw schema check. A manifest can load fine via the runtime CDK (`validate`) and still be rejected by the Builder. Keep these rules in mind when authoring manifests or when copying from another connector package:

### Rule 1 — Prefer field-level `$ref` into `definitions.linked`

Any resolvable `$ref` passes `validate-strict` (the resolver inlines it before the schema check, including `$ref`-with-sibling-keys merge). For **project style**, keep shared values in `definitions.linked.<Component>/<field>` — the Builder's own shared-components store, round-tripped losslessly by the UI:

```yaml
definitions:
  linked:
    HttpRequester:
      url_base: https://api.example.com/v1
      authenticator:
        type: BasicHttpAuthenticator
        username: "{{ config['example_api_key'] }}"
        password: x
      request_headers:
        Accept: application/json

streams:
  - type: DeclarativeStream
    retriever:
      requester:
        type: HttpRequester
        url_base:
          $ref: "#/definitions/linked/HttpRequester/url_base"
        authenticator:
          $ref: "#/definitions/linked/HttpRequester/authenticator"
        request_headers:
          $ref: "#/definitions/linked/HttpRequester/request_headers"
        path: /widgets
```

Whole-object `$ref`s (full requester, paginator, stream — the `task-tracking/jira` style, also what the Builder export itself emits as `streams: [{$ref: "#/definitions/streams/<name>"}]`) resolve fine and pass the gate; for new in-house manifests still prefer linked field refs or inlining — resolved whole-object refs lose their sharing on a Builder round-trip.

### Rule 2 — `type: AddedFieldDefinition` on every `AddFields.fields[]` item

```yaml
transformations:
  - type: AddFields
    fields:
      - type: AddedFieldDefinition          # MANDATORY — Builder will reject without it
        path: [tenant_id]
        value: "{{ config['insight_tenant_id'] }}"
      - type: AddedFieldDefinition
        path: [source_id]
        value: "{{ config['insight_source_id'] }}"
```

### Rule 3 — Integer-typed fields accept literal integers OR Jinja templates

`OffsetIncrement.page_size`, `CursorPagination.page_size` and similar slots are typed as `integer | string-with-interpolation` in the schema. Both forms are accepted by the CDK and by the Builder UI strict validator:

✅ `page_size: 50` (literal)
✅ `page_size: "{{ config.get('my_page_size', 100) }}"` (config-driven)

Use the templated form to wire declared `*_page_size` config keys — otherwise operator overrides in the K8s Secret are silently ignored. `concurrency_level.default_concurrency` is integer-only and does NOT accept a template.

### Rule 4 — Schema URL

Use `http://json-schema.org/schema#`, not `http://json-schema.org/draft-07/schema#`. This is what the Builder emits on export.

### Rule 5 — Schema type arrays ordered `[type, "null"]`

✅ `type: [string, "null"]`
❌ `type: ["null", string]`

### Rule 6 — Required top-level shape

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
  type: Spec
  connection_specification:
    required:
      - insight_tenant_id
      - insight_source_id
      - <source_api_fields>
    properties: ...

metadata:
  autoImportSchema: {...}
```

The `check` block goes **before** `definitions`. Use the cheapest stream (e.g. the directory list) so the health-check has minimal side effects.

## Anti-template: `task-tracking/jira`

The jira connector works at runtime but **fails the Builder UI strict validator** because it uses whole-object `$ref` for `auth`, `base_requester`, `paginator`, substream parents, and `add_fields`. Do not copy from it. Use `collaboration/zoom`, `collaboration/m365`, or `hr-directory/bamboohr` as structural references when authoring a Builder-compatible manifest.

Existing connectors that open cleanly in the Builder UI:

- `collaboration/zoom`
- `collaboration/m365`
- `hr-directory/bamboohr`

Connectors that **do not** open cleanly:

- `task-tracking/jira` — pre-Builder-compat; migrate to granular `$ref` when touching the file.

## Datetime syntax pitfalls

### YouTrack `updated` query

- Format MUST be ISO 8601 with `T` separator: `2026-01-01T00:00:00`
- No braces, no spaces: `updated: 2026-01-01T00:00:00 .. 2026-04-23T00:00:00 sort by: updated asc`
- Braces around datetimes (`updated: {2026-01-01T00:00:00} ..`) are rejected by YouTrack Cloud with `invalid_query`. They worked in legacy v1 because the server was older.

### Jira JQL

- Format: `YYYY-MM-DD HH:MM` (space separator, no seconds, no T)
- `updated >= "2024-01-01 00:00" AND updated <= "2024-02-01 00:00" ORDER BY updated ASC`

Each API has its own datetime dialect. Always confirm with `source.sh check <tenant>` against a real instance before trusting the manifest.

### Epoch millisecond cursors (e.g. YouTrack `updated`)

Some APIs return the cursor field as epoch milliseconds (YouTrack `updated` is an integer ms). **Do not try to convert via a transformation** — `format_datetime(record['x'] / 1000, ...)` inside an `AddedFieldDefinition.value` does not reliably render before the cursor observes the record, and you will see runtime errors like:

```
ValueError: No format in ['%Y-%m-%dT%H:%M:%S'] matching {{ format_datetime(record['updated'] / 1000, '%Y-%m-%dT%H:%M:%S') }}
```

(The value stays as the literal Jinja template.)

**Use Airbyte's native epoch formats** in `DatetimeBasedCursor.cursor_datetime_formats`:

| Token | Meaning |
|---|---|
| `%s` | epoch seconds |
| `%s_as_float` | epoch seconds (float, sub-second precision) |
| `%ms` | epoch **milliseconds** |
| `%epoch_microseconds` | epoch microseconds |

For YouTrack `updated` (millis), the cursor block is:

```yaml
incremental_sync:
  type: DatetimeBasedCursor
  cursor_field: updated                      # raw record field, no transform
  cursor_datetime_formats:                   # parse record value as %ms
    - '%ms'
    - '%Y-%m-%dT%H:%M:%S'                    # also accept ISO for persisted state
  datetime_format: '%Y-%m-%dT%H:%M:%S'       # format used for state + request params
  start_datetime:
    type: MinMaxDatetime
    datetime: "{{ config.get('x_start_date', '2020-01-01') }}T00:00:00"
    datetime_format: '%Y-%m-%dT%H:%M:%S'
  end_datetime:
    type: MinMaxDatetime
    datetime: "{{ now_utc().strftime('%Y-%m-%dT%H:%M:%S') }}"
    datetime_format: '%Y-%m-%dT%H:%M:%S'
  cursor_granularity: PT1S                   # MUST be present whenever `step` is set
  step: P30D
  lookback_window: PT1H
```

Keep both `%ms` (for live record values) and `%Y-%m-%dT%H:%M:%S` (for persisted state values re-parsed on resume) in `cursor_datetime_formats`. `cursor_granularity` MUST accompany `step` — if it is omitted, the CDK raises `ValueError: If step is defined, cursor_granularity should be as well and vice-versa`.

## Debugging strict-validation errors

`validate-strict` prints the deepest-matching JSON-schema path for each error. Interpret like this:

```
[1] streams/0/transformations/0/fields/3: 'type' is a required property
```

→ `streams[0].transformations[0].fields[3]` is missing `type: AddedFieldDefinition`.

```
STRICT VALIDATION FAILED — unresolvable $ref: Undefined reference #/definitions/base_requester
```

→ A `$ref` points at a path that does not exist in the manifest. Fix the path or define the target. (Resolvable `$ref`s are inlined before validation — error paths always refer to the **resolved** manifest.)

```
[1] concurrency_level/default_concurrency: "{{ config.get('x_concurrency', 1) }}" is not of type 'integer'
```

→ `concurrency_level.default_concurrency` is integer-only — it does NOT accept a Jinja template. Replace with a literal integer.

If you need raw validator output with all alternative branches (e.g. while iterating on a `oneOf` union), bypass the leaf-picker and dump every error directly:

```bash
docker run --rm \
  -v "$PWD/src/ingestion/connectors/<class>/<connector>:/input:ro" \
  -v "$PWD/src/ingestion/tools/declarative-connector/validate_strict.py:/scripts/validate_strict.py:ro" \
  --entrypoint=/bin/sh \
  airbyte/source-declarative-manifest:local-6.60.9 \
  -c "python3 /scripts/validate_strict.py /input/connector.yaml"
```

For lower-level inspection (every error including the noisy `oneOf` branches), edit `validate_strict.py` locally — keeping the logic in a `.py` file is the project's no-inline-Python rule (`cypilot/config/rules/code-conventions.md` §"No inline scripts").

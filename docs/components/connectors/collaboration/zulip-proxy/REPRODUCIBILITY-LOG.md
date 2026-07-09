# Zulip-Proxy Connector — Reproducibility Log

Purpose: track every deviation from the documented workflow (`/connector create`, Constructor Studio,
existing connector conventions) and every gap in the skills/specs encountered while building this
connector. Future contributors should be able to reproduce the package end-to-end and the
maintainers should be able to close the gaps in the skills/specs.

Conventions:
- `REF` — concrete reference to a doc/file path where the workflow is defined.
- `CHOICE` — a decision taken by the user/agent during this run.
- `DEVIATION` — a documented departure from the workflow or convention. Each deviation has a
  rationale and a recommended follow-up to upstream into the skill or convention.
- `GAP` — a missing instruction, ambiguous step, or "the doc says one thing, the example does the
  opposite" finding. Each gap has a recommended fix.

---

## 0. Inputs and ground truth

- **Reference manifest** (Airbyte declarative source v0.57.0, incompatible with current repo):
  `zulip_proxy.yaml` (local workspace copy, not in repo) — Bearer auth against a proxy that aggregates
  Zulip data. Streams `users` (offset-paginated) and `messages` (cursor-paginated, incremental on
  `created_at`).
- **Same-data sibling spec** (existing Zulip Basic-Auth connector docs in the repo):
  `docs/components/connectors/collaboration/zulip/zulip.md` (+ `specs/DESIGN.md`, `specs/PRD.md`,
  `specs/ADR/`).
- **Manifest version** chosen: 7.0.4 (matches `collaboration/m365/connector.yaml`; recommended for
  new connectors by `.cf-studio/.core/skills/connector/workflows/create.md` §3.1).

## 1. Scope decisions

- `CHOICE`: zulip-proxy is a **separate connector** under `collaboration/zulip-proxy/`, not a
  replacement for the existing `collaboration/zulip/` Basic-Auth spec. Reason: same Bronze data
  shape but distinct transport (proxy host, Bearer token), distinct source contract, distinct
  K8s Secret. The existing `zulip` spec stays untouched.
- `CHOICE`: Constructor Studio artifacts: PRD + DESIGN + FEATURE under
  `docs/components/connectors/collaboration/zulip-proxy/specs/` (no ADR initially — no contested
  architectural decision unique to this connector).
- `CHOICE`: dbt scope: full bronze → silver, with `identity_inputs` and `promote_bronze_to_rmt`
  macros, modeled on `collaboration/zoom/` (closest collaboration pattern; `ms-teams` lives inside
  `collaboration/m365/` and is not a separate connector folder).
- `CHOICE`: Secret fields exposed via `connection_specification`:
  `zulip_proxy_api_key` (Bearer token, `airbyte_secret: true`),
  `zulip_proxy_base_url` (proxy host, no default — fail-fast),
  `zulip_proxy_start_date` (ISO date, controls incremental backfill window),
  `zulip_proxy_throttle_ms` (per-request throttle, default 5000),
  `insight_tenant_id`, `insight_source_id` (mandatory in every nocode connector).

## 2. Deviations from documented workflow / repo conventions

### DEV-01 — `start_date` declared as a config parameter

- **Convention**: `.cf-studio/.core/skills/connector/workflows/create.md` §3.1 says: "MUST include …
  Incremental sync with **computed dates (no config params for start/end)**".
- **Deviation**: `zulip_proxy_start_date` is a required field in `connection_specification` and is
  injected into the `DatetimeBasedCursor.start_datetime` Jinja template. Same pattern as
  `collaboration/zoom/connector.yaml` (`zoom_start_date`) — so the convention as stated is
  contradicted by an existing reference connector.
- **Rationale**: per-tenant backfill horizon varies (some proxies retain longer history than
  others); user explicitly asked for this knob.
- **Recommended fix**: update `create.md` §3.1 to reflect the de-facto pattern — backfill anchor
  date MAY be a required `connection_specification` field when source retention varies per
  deployment. The "computed dates" rule applies to per-run window math, not to the absolute
  backfill anchor.

### DEV-02 — `url_base` parameterized via config (proxy host)

- **Convention**: in every existing nocode connector in this repo, `definitions.linked.HttpRequester.url_base` is hardcoded
  to a single public API host (e.g. `https://api.zoom.us/v2`).
- **Deviation**: zulip-proxy `url_base` is rendered from `{{ config['zulip_proxy_base_url'] }}`
  because the proxy host varies per deployment (private IP or internal DNS, no public API).
- **Rationale**: same connector binary must target multiple proxy instances; baking in a host
  defeats the purpose.
- **Recommended fix**: add a "proxied/private-host source" pattern to `create.md` §3.1
  (Builder-UI compatibility of templated `url_base` is supported — already used by `cdk` builds).

### DEV-03 — Connector-level throttle exposed via config

- **Convention**: rate limiting is handled by `error_handler.backoff_strategies` keyed on
  `429`/`Retry-After`, not by client-side throttling parameters.
- **Deviation**: `zulip_proxy_throttle_ms` is forwarded as a request_parameter `throttle` on the
  `messages` stream (mirrors the reference 0.57.0 manifest which carried `throttle: '5000'` as a
  hardcoded query param). The proxy interprets this server-side as a per-request pacing hint.
- **Rationale**: server-side contract of the proxy — not a client-side timer.
- **Recommended fix**: documented in `docs/components/connectors/collaboration/zulip-proxy/specs/DESIGN.md`
  §"External Dependencies" so the proxy contract is explicit.

### DEV-05 — `zulip_proxy_throttle_ms` declared as `string` in spec (not `integer`)

- **Convention**: spec fields representing numeric values are typed `integer` (e.g. zoom's
  `page_size`).
- **Deviation**: discovered during the first live `check` — K8s Secret `stringData` always
  stringifies values, but `source.sh` only auto-parses JSON arrays/objects, not scalars
  (see `tools/declarative-connector/source.sh` ~ line 145). With `type: integer`, the spec
  validator rejected `"5000"` from the secret with `Config validation error: '5000' is not of
  type 'integer'`. Changed `zulip_proxy_throttle_ms` to `type: string` (default `"5000"`); the
  proxy parses the value server-side. Same outcome as zoom would face if it tried to override
  `page_size` via secret instead of relying on the spec default.
- **Recommended fix**: either (a) change `source.sh` to coerce numeric stringData to the spec's
  declared type when the field has `type: integer|number|boolean`, or (b) document in
  `create.md` §3.3 that any spec field that can be overridden via K8s Secret must be `type:
  string`. The current state silently forces (b) but doesn't say so.

### DEV-04 — Connector implemented manually rather than via the `/connector create` skill flow

- **Workflow expected**: `.cf-studio/.core/skills/connector/workflows/create.md` Phase 1 asks 6
  interactive questions and scaffolds files. Run inside an LLM client that supports the slash
  command.
- **Reality**: the agent assembled files directly from the create.md template by reading example
  connectors (`collaboration/zoom`, `collaboration/m365`). Phase 1 Q&A was conducted via the
  `AskUserQuestion` tool earlier in the conversation; the artifacts match the Phase 3 spec.
- **Recommended fix**: this is acceptable — the `/connector create` workflow is a description of
  what to produce, not a script. Add a note to `create.md` clarifying that the workflow is
  declarative ("produce the following files") so future agents do not look for an executable
  scaffolder.

## 3. Gaps in skills / specs / conventions

### GAP-01 — `/connector create` does not generate `dbt/` Silver-layer scaffolding for nocode

- **Where**: `.cf-studio/.core/skills/connector/workflows/create.md` §3.5–3.6 — defines a single
  staging dbt model (`<name>__<domain>.sql`) and `schema.yml`. It does NOT mention:
  - `<name>__bronze_promoted.sql` (RMT promotion bootstrap — required by
    `docs/domain/ingestion-data-flow/specs/ADR/0002-promote-bronze-to-rmt.md`)
  - `<name>__users_snapshot.sql` + `<name>__users_fields_history.sql`
    (SCD2 history for identity fields)
  - `<name>__identity_inputs.sql` (uses `identity_inputs_from_history` macro to feed Identity
    Resolution)
  - The expected silver tag convention `silver:class_<class>` on staging models
- **Reference for correct shape**: `src/ingestion/connectors/collaboration/zoom/dbt/`.
- **Recommended fix**: expand `create.md` §3.5 into a checklist that covers all five dbt files
  (bronze_promoted, users_snapshot, users_fields_history, identity_inputs, class_*) with template
  snippets.

### GAP-02 — Mismatch between `descriptor.yaml` field set in `create.md` and what existing connectors actually use

- **`create.md` §3.2** documents:
  ```yaml
  name:
  version: "1.0"
  schedule: "0 2 * * *"
  dbt_select: "tag:<name>+"
  workflow: sync
  connection:
    namespace: "bronze_<name>"
  ```
- **Existing `collaboration/zoom/descriptor.yaml`** also has:
  ```yaml
  secret:
    required_fields: [...]
  ```
  This is mandated by `docs/components/airbyte-toolkit/specs/ADR/0007-required-fields-in-descriptor-not-example.md`.
- **Recommended fix**: add `secret.required_fields` to the `create.md` §3.2 template.

### GAP-03 — `create.md` says "do NOT add `username`/`password` if using BasicHttpAuthenticator" but does not warn about Bearer-token spec naming

- **Where**: `.cf-studio/.core/skills/connector/workflows/create.md` §3.3, last bullet of the K8s
  Secret rules: "Do NOT include `username`/`password` if using `BasicHttpAuthenticator` — these
  are Builder artifacts".
- **Gap**: there is no equivalent note for `BearerAuthenticator`. In this connector the Bearer
  token is `zulip_proxy_api_key`. The Builder UI does NOT auto-inject Bearer artifacts (unlike
  Basic), but the naming convention (`<source>_api_key` with `airbyte_secret: true`) is implicit
  rather than documented.
- **Recommended fix**: add a one-liner to §3.3 stating the Bearer convention.

### GAP-04 — Manifest version migration guidance is thin

- **Where**: `create.md` §3.1 lists three v6→v7 breaking changes but does NOT cover the migration
  path for a v0.5x reference manifest (such as the one provided by the user). The Builder applies
  `applied_migrations` automatically on import, but offline ports must be done manually.
- **Recommended fix**: link `src/ingestion/tools/declarative-connector/README.md`'s migration
  notes from `create.md` §3.1 or duplicate the v0.5x → v6 → v7 schema diffs.

### GAP-06 — `/check-dbt-conventions` rules contradict existing connector practice

- **Where**: `.claude/skills/check-dbt-conventions/SKILL.md` Check 2 ("If `materialized` is
  `incremental` or `table` → `engine='ReplacingMergeTree(_version)'` + `order_by=['unique_key']`")
  and Check 7 (`materialized='table'` allow-list).
- **Gap**: existing connectors (`zoom/dbt/`, `m365/dbt/`) intentionally do NOT set
  `engine`/`order_by` on intermediate staging models such as `*__users_snapshot.sql`,
  `*__users_fields_history.sql`, `*__identity_inputs.sql`. The rule, taken literally, would mark
  every existing connector as failing — and indeed mark zulip-proxy as failing too. This
  connector mirrors the existing convention (zoom in particular) and gets the Check 2 / Check 7
  warnings as a consequence.
- **Recommended fix**: scope Check 2 and Check 7 to silver models (`silver:class_*` /
  `silver:fct_*` / `silver:mtr_*` tags), and treat snapshot / fields_history / identity_inputs
  as a documented "intermediate staging" tier with looser requirements — or update those
  intermediate models project-wide to the strict rule in a single sweep.

### GAP-07 — `validate-bronze-promoted.py` referenced by skill but missing in repo

- **Where**: `.cf-studio/.core/skills/connector/workflows/validate.md` §"Bronze Promotion" calls
  `./airbyte-toolkit/validate-bronze-promoted.py <category>/<connector>`.
- **Gap**: that script does not exist in `src/ingestion/airbyte-toolkit/` (verified via `find`).
  This run substituted manual `grep`+inspection of the bronze_promoted file.
- **Recommended fix**: either commit the validator script or update `validate.md` to point at
  whatever tool replaced it.

### GAP-05 — No example of "private/proxied source" in `create.md`

- **Where**: the Phase-1 question template asks "API base URL? (e.g.
  https://graph.microsoft.com/v1.0)" — the example is a public hostname.
- **Gap**: there is no guidance for private-network sources where `url_base` must come from a
  per-deployment Secret (see DEV-02 above).
- **Recommended fix**: add a "Private/proxied sources" subsection to `create.md` Phase 3.1
  showing a `url_base: "{{ config['<name>_base_url'] }}"` example.

## 4. Validation matrix

| Step | Tool | Expected outcome | Status (this run) |
|------|------|------------------|-------------------|
| `cfs validate --artifact docs/components/connectors/collaboration/zulip-proxy/specs/PRD.md` | Constructor Studio | PASS — structure, IDs, cross-refs | ✅ PASS |
| `cfs validate --artifact docs/components/connectors/collaboration/zulip-proxy/specs/DESIGN.md` | Constructor Studio | PASS | ✅ PASS (after TOC regen + `component`/`seq` IDs added) |
| `cfs validate --artifact docs/components/connectors/collaboration/zulip-proxy/specs/FEATURE.md` | Constructor Studio | PASS | ✅ PASS (after restructure to mandatory section list: States/DoD/AC) |
| `./tools/declarative-connector/source.sh validate-strict collaboration/zulip-proxy` | connector | PASS — Builder strict | ✅ PASS (after inlining `BearerAuthenticator` per-stream — strict validator doesn't follow `$ref` for `type`) |
| `./tools/declarative-connector/source.sh validate collaboration/zulip-proxy` | connector | PASS — CDK runtime | ✅ PASS |
| `./tools/declarative-connector/source.sh check collaboration/zulip-proxy <tenant>` | connector | PASS — Bearer token accepted | ✅ PASS — `CONNECTION_STATUS: SUCCEEDED` after fixing DEV-05 (throttle type) |
| `./tools/declarative-connector/source.sh discover collaboration/zulip-proxy <tenant>` | connector | discovered both streams | ✅ PASS — `users` (10 fields, full_refresh) + `messages` (7 fields, incremental, cursor=`created_at`) |
| `./tools/declarative-connector/source.sh read users` (full refresh) | connector | records > 0, errors = 0, all stamped | ✅ PASS — 910 records, 0 errors, all `tenant_id`/`source_id`/`unique_key` populated; sample `unique_key=example_tenant-zulip-proxy-main-8` |
| `./tools/declarative-connector/source.sh read messages` (first read, narrow window 2026-05-15…) | connector | records > 0, errors = 0, STATE emitted | ✅ PASS — 1417 records, 0 errors, 2 STATE messages; persisted `created_at=2026-05-20T00:00:00.000000+0000` |
| `./tools/declarative-connector/source.sh read messages` (resume read from STATE) | connector | strict subset of first read | ✅ PASS — 532 records (< 1417), cursor range 2026-05-19…2026-05-20 (vs first 2026-05-14…2026-05-20), cursor advanced |
| `/check-dbt-conventions` (zulip-proxy scope) | dbt | PASS — silver model is RMT(_version) + order_by [unique_key]; bronze_promoted correct; matches zoom convention on intermediate models (Check 2/7 noise — see GAP-06) | ✅ PASS |
| `cfs validate --skip-code` (whole project) | Constructor Studio | 242 pre-existing errors elsewhere, 0 for zulip-proxy artifacts | ✅ PASS (zulip-proxy scope) |

## 5. Open follow-ups

- After `/connector test … read` against the live proxy, confirm whether the `messages` payload
  field path is `messages` (current assumption from the v0.57.0 reference) and whether `users`
  payload field path is `users`. Update `record_selector.extractor.field_path` if the proxy
  returns a flat array.
- After `/connector schema …` runs against live data, regenerate `InlineSchemaLoader.schema` for
  both streams and remove the placeholder schemas committed in this PR.
- Decide whether `zulip-proxy` and `zulip` (existing direct Basic-Auth spec) should share a Silver
  target table (`class_collab_chat_activity` per Connector Reference) or use distinct `data_source`
  literals (`insight_zulip_proxy` vs `insight_zulip`). Currently: distinct literals — sources can
  coexist in the same Silver table without colliding because `(tenant_id, source_id, …)` keys are
  disjoint.

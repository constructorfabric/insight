---
status: proposed
date: 2026-06-19
---

# Feature: Declarative YAML Test Rig (full replacement of the CSV rig)

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-featstatus-yaml-rig`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Author and Run a Test](#author-and-run-a-test)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Resolve Records (refs + schema padding + validation)](#resolve-records-refs--schema-padding--validation)
  - [Execute Test (per-test loop)](#execute-test-per-test-loop)
  - [Evaluate an Expect Rule](#evaluate-an-expect-rule)
- [4. States (CDSL)](#4-states-cdsl)
  - [Test Lifecycle](#test-lifecycle)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Suffix-based Discovery](#suffix-based-discovery)
  - [Reference Resolution](#reference-resolution)
  - [Schema Resolution, Padding and Validation](#schema-resolution-padding-and-validation)
  - [Typed Bronze Seed from Records](#typed-bronze-seed-from-records)
  - [Batch API Roundtrip](#batch-api-roundtrip)
  - [Expect Engine](#expect-engine)
  - [Reference Test (collab_emails_sent)](#reference-test-collabemailssent)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

## 1. Feature Context

- [ ] `p2` - `cpt-bronze-to-api-e2e-feature-yaml-rig`

### 1.1 Overview

Replace the folder-based CSV rig (`feature-csv-rig`: `bronze/*.csv` + `spec.yaml` + `expected/response.csv`) with a single declarative YAML test file (`<name>.test.yaml`). One file describes the whole test: what raw rows to seed (`bronze`), what to call (`cases[].request`, batched), and what must hold (`cases[].expect`).

The readability goals are:

- **Small surface, full data on demand.** A bronze row is written as a `$ref` to a reusable record template plus only the fields the test actually exercises. After resolution the row is padded to every column of the table schema, so the seeded record is complete without the author spelling out 29 columns.
- **Reusable building blocks in separate files.** Per-table JSON schemas live in `metrics/schemas/<db>.<table>.yaml` (e.g. `bronze_m365.email_activity.yaml`); record templates live in `metrics/templates/*.yaml`. A test references them with the standard `$ref: "<file>#/<json-pointer>"` form.
- **Assert what matters, not the whole body.** `expect` is a list of rules; each selects a row with an exact-equality `find` and either compares a subset of fields (`equal`) or evaluates a CEL boolean (`assert`). Anything richer than equality lives in the CEL `assert`, so there is no second selector language.

This feature supersedes `feature-csv-rig` end to end: the `spec.yaml`/CSV loader, the `csv-asserter`, and the per-folder fixture layout are removed. The bronze→silver→gold→API path itself is unchanged — only the authoring format and the assertion engine change.

### 1.2 Purpose

The CSV rig proved the path but was verbose (one CSV per table, full column rows, a separate `expected/response.csv`) and tied to the single-metric endpoint. The product moved to a **batch** metric endpoint (`POST /v1/metrics/queries`), and dashboard metrics (e.g. the collaboration bullets) return many `metric_key` rows where the author cares about two or three. The YAML format keeps tests short and intention-revealing while still seeding production-shaped bronze.

**Requirements**:

- `cpt-bronze-to-api-e2e-fr-bronze-seed-from-csv` *(re-interpreted: seed from resolved YAML records, not CSV)*
- `cpt-bronze-to-api-e2e-fr-bronze-truncate`
- `cpt-bronze-to-api-e2e-fr-dbt-run-scoped`
- `cpt-bronze-to-api-e2e-fr-gold-view-queried`
- `cpt-bronze-to-api-e2e-fr-api-roundtrip` *(re-interpreted: batch endpoint)*
- `cpt-bronze-to-api-e2e-fr-csv-assert` *(re-interpreted: expect-rule engine, not CSV diff)*

**Principles**:

- `cpt-bronze-to-api-e2e-principle-shared-session`
- `cpt-bronze-to-api-e2e-principle-fixtures-are-truth`
- `cpt-bronze-to-api-e2e-principle-record-composition`
- `cpt-bronze-to-api-e2e-principle-schema-is-truth`

**Constraints**:

- `cpt-bronze-to-api-e2e-constraint-no-ddl-mutation`
- `cpt-bronze-to-api-e2e-constraint-loopback-only`

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-bronze-to-api-e2e-actor-test-author` | Authors the `<name>.test.yaml`, templates, and schemas; runs `pytest` |
| `cpt-bronze-to-api-e2e-actor-data-engineer` | Changes a dbt model / gold view and reruns to catch regressions |
| `cpt-bronze-to-api-e2e-actor-dbt-cli` | Subprocess invoked per test with a selector |
| `cpt-bronze-to-api-e2e-actor-analytics-api` | Service under test, spawned once per session |

### 1.4 References

- **PRD**: [../PRD.md](../PRD.md)
- **DESIGN**: [../DESIGN.md](../DESIGN.md) (v1.1 — adds `ref-resolver`, `schema-validator`, `expect-engine`; retires `csv-asserter`)
- **DECOMPOSITION**: [../DECOMPOSITION.md](../DECOMPOSITION.md)
- **Supersedes**: `cpt-bronze-to-api-e2e-feature-csv-rig`

## 2. Actor Flows (CDSL)

### Author and Run a Test

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-flow-yaml-author-and-run`

**Actor**: `cpt-bronze-to-api-e2e-actor-test-author`

**Success Scenarios**:

- Author writes `<name>.test.yaml` that `$ref`s shared people/source templates, overrides only the fields under test, and the test passes
- Author changes a dbt model / gold view and the relevant `equal`/`assert` rule fails with a precise message

**Error Scenarios**:

- A `$ref` points at a missing file or pointer → resolution fails at collect time with the offending ref
- A resolved record carries a field absent from the table schema (`additionalProperties:false`) → validation fails at collect time
- A `case` references a batch result id (`in:`) that the request does not declare → fails before the API call
- A metric query inside the batch returns `status: "error"` → the rule `result.status == 'ok'` fails with the embedded problem detail

**Steps**:

1. [ ] - `p1` - Author creates `src/ingestion/tests/e2e/metrics/<name>.test.yaml` - `inst-yaml-author-file`
2. [ ] - `p1` - Author writes `bronze:` keyed by table name; each row is a `$ref` to a template plus the overridden fields under test (duplicates allowed) - `inst-yaml-author-bronze`
3. [ ] - `p1` - Author writes `cases:` with `request.body.queries[]` (batch) and an `expect` list of rules - `inst-yaml-author-cases`
4. [ ] - `p1` - Author runs `pytest -k <name>` - `inst-yaml-author-run`
5. [ ] - `p1` - Algorithm: framework runs `cpt-bronze-to-api-e2e-algo-yaml-execute-test` - `inst-yaml-author-invoke-exec`
6. [ ] - `p1` - **IF** all rules of all cases hold **RETURN** "ok" - `inst-yaml-author-return-pass`
7. [ ] - `p1` - **ELSE** runner reports the failing rule (`case`, rule index, selector, expected vs actual) - `inst-yaml-author-emit-fail`

## 3. Processes / Business Logic (CDSL)

### Resolve Records (refs + schema padding + validation)

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-algo-yaml-resolve-refs`

**Input**: a YAML node `node`, the file it was written in `ctx_file`, a cycle-guard `stack`

**Output**: a fully-merged plain record (no `$ref`), padded and validated when it is a bronze row

**Steps**:

1. [ ] - `p1` - **IF** `node` is not a mapping **RETURN** `node` (scalars/lists pass through) - `inst-resolve-scalar`
2. [ ] - `p1` - **IF** `node` has no `$ref` key **RETURN** `{ k: resolve(v, ctx_file, stack) for k,v in node }` - `inst-resolve-plain`
3. [ ] - `p1` - Split `$ref` into `file_part` and `pointer` on `#` - `inst-resolve-split`
4. [ ] - `p1` - `target_file` = `ctx_file` **IF** `file_part` empty **ELSE** `dir(ctx_file) / file_part` (normalized) - `inst-resolve-target-file`
5. [ ] - `p1` - **IF** `(target_file, pointer)` in `stack` **RETURN** error("cycle: " + chain) - `inst-resolve-cycle`
6. [ ] - `p1` - Load `target_file` (cached); navigate JSON-pointer `pointer` → `target` (**IF** missing → error) - `inst-resolve-load-target`
7. [ ] - `p1` - `base` = `resolve(target, target_file, stack + [(target_file, pointer)])` — **the base resolves in its OWN file's context** - `inst-resolve-base`
8. [ ] - `p1` - `overrides` = `resolve(node without $ref, ctx_file, stack)` — siblings resolve in the current file's context - `inst-resolve-overrides`
9. [ ] - `p1` - **RETURN** `deep_merge(base, overrides)` — overrides win; map+map merges recursively; otherwise the override replaces - `inst-resolve-merge`

**Post-step (bronze rows only)**: for a resolved row of table `T`, look up `schemas[T]`, add every missing schema property with value `null`, then validate the row against the JSON schema; any violation fails the test at collect time.

### Execute Test (per-test loop)

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-algo-yaml-execute-test`

**Input**: a resolved `TestYaml` (`bronze`, `cases`), `WorkerContext`

**Output**: `Pass` | `Fail(failing rule report)`

**Steps**:

1. [ ] - `p1` - Algorithm: `cpt-bronze-to-api-e2e-algo-csv-rig-truncate-touched` clears the prior test's seeded + dbt-built tables (ledger) - `inst-yexec-truncate`
2. [ ] - `p1` - **FOR EACH** `(table, rows)` in `bronze` - `inst-yexec-foreach-table`
   1. [ ] - `p1` - DB: read column types from `system.columns` for `<table>` - `inst-yexec-types`
   2. [ ] - `p1` - Coerce each resolved record's fields to typed values (null, Date, Decimal, Bool, Array) - `inst-yexec-coerce`
   3. [ ] - `p1` - DB: INSERT all rows (including duplicates) into the bronze table - `inst-yexec-insert`
   4. [ ] - `p1` - Record `(schema, table)` in the per-test ledger - `inst-yexec-ledger`
3. [ ] - `p1` - Subprocess: two-pass `dbt build` (staging/promotion, then `class_*` silver) for the touched tables' models - `inst-yexec-dbt`
4. [ ] - `p1` - **IF** dbt exit != 0 → surface failing model + compiled SQL, **RETURN** Fail - `inst-yexec-dbt-fail`
5. [ ] - `p1` - Re-apply the gold-view migrations so the views match the rebuilt (real, Nullable) silver schema — the `Code 80` structure mismatch is a rig artifact, verified clean on dev - `inst-yexec-recreate-views`
6. [ ] - `p1` - Refresh materialized intermediates so silver writes are visible to gold views - `inst-yexec-refresh-mv`
7. [ ] - `p1` - **FOR EACH** `case` in `cases` - `inst-yexec-foreach-case`
   1. [ ] - `p1` - HTTP: POST `case.request.body` to `/v1/metrics/queries` → `BatchResponse { results[] }` - `inst-yexec-batch-call`
   2. [ ] - `p1` - **FOR EACH** `rule` in `case.expect`: Algorithm `cpt-bronze-to-api-e2e-algo-yaml-eval-expect` - `inst-yexec-foreach-rule`
   3. [ ] - `p1` - **IF** any rule fails **RETURN** Fail(rule report) - `inst-yexec-rule-fail`
8. [ ] - `p1` - **RETURN** Pass - `inst-yexec-return-pass`

### Evaluate an Expect Rule

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-algo-yaml-eval-expect`

**Input**: `rule` (`{ in?, find?, equal? | assert? }`), `BatchResponse`, the HTTP `status`

**Output**: `Pass` | `Fail(reason)`

**Steps**:

1. [ ] - `p1` - **IF** `rule.in` present → `result` = the element of `results[]` whose `id == rule.in` (**IF** absent → Fail) - `inst-eval-in`
2. [ ] - `p1` - **ELSE IF** exactly one result exists → `result` = that result **ELSE** `result` = none (rule operates on the whole response) - `inst-eval-sole`
3. [ ] - `p1` - **IF** `rule.find` present → filter `result.items` to rows whose every selected field exactly equals the given value; **IF** not exactly one match → Fail; `it` = the match - `inst-eval-find`
4. [ ] - `p1` - **IF** `rule.equal` present → for each `(field, exp)` assert `it[field] == exp` (subset; `null` compared explicitly); collect mismatches - `inst-eval-equal`
5. [ ] - `p1` - **ELSE IF** `rule.assert` present → evaluate the CEL expression with bindings `it`, `items`, `result`, `results`, `status`; **IF** not `true` → Fail - `inst-eval-assert`
6. [ ] - `p1` - **RETURN** Pass | Fail(field/expr, expected, actual) - `inst-eval-return`

**CEL `assert` bindings** — the variables a `assert` expression may reference.
Assembled in `lib/expect_engine.py::evaluate_case` (the `bindings` dict) and
converted to CEL in `_eval_cel`:

| Binding | Value | Present when |
|---|---|---|
| `it` | the single row matched by `find` | only with `find` (else `null`) |
| `items` | the selected result's `items` array | a result is selected (`in` or sole query) |
| `result` | the selected batch result `{id, status, metric_id, items, page_info}` | a result is selected |
| `results` | the full `results[]` of the batch response | always |
| `status` | the batch HTTP status code (int) | always |

CEL is strictly typed and will not compare an `int` to a `double`. Bindings are
passed through unchanged, so cast a possibly-integral metric value when comparing
against a fractional literal: `double(it.value) > 39.5`. `status` and `size(...)`
are integers (compare with integer literals). Exact / `null` checks belong in
`equal` (Python `==`), not `assert`.

## 4. States (CDSL)

### Test Lifecycle

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-state-yaml-lifecycle`

**States**: `PENDING`, `RESOLVING`, `SEEDING`, `DBT_BUILDING`, `VIEW_REFRESH`, `QUERYING`, `ASSERTING`, `PASSED`, `FAILED`

**Initial State**: `PENDING`

**Transitions**:

1. [ ] - `p1` - **FROM** `PENDING` **TO** `RESOLVING` **WHEN** the `.test.yaml` is parsed - `inst-ystate-pending-resolving`
2. [ ] - `p1` - **FROM** `RESOLVING` **TO** `SEEDING` **WHEN** all records resolve, pad and validate - `inst-ystate-resolving-seeding`
3. [ ] - `p1` - **FROM** `RESOLVING` **TO** `FAILED` **WHEN** a `$ref` is unresolvable, a cycle is found, or schema validation fails - `inst-ystate-resolving-failed`
4. [ ] - `p1` - **FROM** `SEEDING` **TO** `DBT_BUILDING` **WHEN** all bronze rows insert - `inst-ystate-seeding-dbt`
5. [ ] - `p1` - **FROM** `DBT_BUILDING` **TO** `VIEW_REFRESH` **WHEN** dbt exits 0 - `inst-ystate-dbt-view`
6. [ ] - `p1` - **FROM** `VIEW_REFRESH` **TO** `QUERYING` **WHEN** gold views recreated and MVs refreshed - `inst-ystate-view-query`
7. [ ] - `p1` - **FROM** `QUERYING` **TO** `ASSERTING` **WHEN** the batch response is received and deserialized - `inst-ystate-query-assert`
8. [ ] - `p1` - **FROM** `ASSERTING` **TO** `PASSED` **WHEN** every rule of every case holds - `inst-ystate-assert-passed`
9. [ ] - `p1` - **FROM** any state **TO** `FAILED` **WHEN** its guard fails; the failing step is reported - `inst-ystate-any-failed`

## 5. Definitions of Done

### Suffix-based Discovery

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-yaml-discovery`

The system **MUST** discover tests as `src/ingestion/tests/e2e/metrics/**/*.test.yaml`. Files under `metrics/schemas/` and `metrics/templates/` (and any `*.yaml` without the `.test.yaml` suffix) **MUST NOT** be collected as tests. A malformed `.test.yaml` **MUST** fail the collection of only that test.

**Implements**: `cpt-bronze-to-api-e2e-flow-yaml-author-and-run`

**Touches**: `src/ingestion/tests/e2e/conftest.py`, `src/ingestion/tests/e2e/lib/fixture_loader.py`; Components: `cpt-bronze-to-api-e2e-component-fixture-loader`

### Reference Resolution

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-yaml-ref-resolution`

The resolver (`cpt-bronze-to-api-e2e-algo-yaml-resolve-refs`) **MUST** satisfy, as pure unit tests (no ClickHouse / dbt):

1. local pointer `#/templates/x`;
2. cross-file `templates/people.yaml#/templates/alice`;
3. a sibling scalar overrides the base value;
4. a sibling adds a field absent from the base;
5. a sibling `null` overrides a non-null base value;
6. a chain `A $ref B $ref C` merges with the closest layer winning;
7. a nested `$ref` resolves relative to **its own** file (a `#/...` ref inside `people.yaml` stays in `people.yaml` when referenced from a test);
8. nested maps deep-merge;
9. a missing file raises a clear error naming the ref;
10. a missing pointer raises a clear error naming the ref;
11. a cycle `A→B→A` raises a clear error with the chain;
12. a two-layer override (`$ref` to a record that itself has `$ref` + siblings) merges both layers.

**Implements**: `cpt-bronze-to-api-e2e-algo-yaml-resolve-refs`

**Touches**: `src/ingestion/tests/e2e/lib/ref_resolver.py`, `src/ingestion/tests/e2e/meta/test_ref_resolver.py`; Components: `cpt-bronze-to-api-e2e-component-ref-resolver`

### Schema Resolution, Padding and Validation

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-yaml-schema-resolution`

For each `bronze.<table>`, the system **MUST** resolve the schema by table name from `metrics/schemas/<db>.<table>.yaml`, pad every resolved record with the schema's missing properties as `null`, and validate the record against the JSON schema. `additionalProperties:false` **MUST** reject an unknown field name. The `_airbyte_*` columns **MUST** be carried from the record (not auto-stamped), since transforms depend on them.

**Implements**: `cpt-bronze-to-api-e2e-algo-yaml-resolve-refs`

**Touches**: `src/ingestion/tests/e2e/lib/schema_validator.py`; Components: `cpt-bronze-to-api-e2e-component-schema-validator`

### Typed Bronze Seed from Records

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-yaml-bronze-seed`

The system **MUST** INSERT each resolved record into its bronze table with column types resolved via `system.columns` (null, Date, Decimal, Bool, Array coercions). Duplicate records (two identical resolved rows) **MUST** be inserted physically so dedup is exercised at the bronze/silver layer, not hidden by the rig.

**Implements**: `cpt-bronze-to-api-e2e-algo-yaml-execute-test`

**Touches**: `src/ingestion/tests/e2e/lib/ch_seeder.py`; Components: `cpt-bronze-to-api-e2e-component-ch-seeder`

### Batch API Roundtrip

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-yaml-batch-roundtrip`

The system **MUST** POST `case.request.body` (`{ queries: [...] }`) to `POST /v1/metrics/queries` and deserialize `{ results: [...] }`, where each result carries `id`, `status` (`ok`|`error`), and either `items`/`page_info` or `error`. A per-query `error` **MUST NOT** be masked by the batch HTTP 200.

**Implements**: `cpt-bronze-to-api-e2e-algo-yaml-execute-test`

**Constraints**: `cpt-bronze-to-api-e2e-constraint-loopback-only`

**Touches**: `src/ingestion/tests/e2e/lib/api_client.py`; API: `POST /v1/metrics/queries`; Components: `cpt-bronze-to-api-e2e-component-api-client`

### Expect Engine

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-yaml-expect-engine`

The system **MUST** evaluate `expect` rules per `cpt-bronze-to-api-e2e-algo-yaml-eval-expect`: `in` selects a batch result by id (optional when one query); `find` selects exactly one row via exact field equality; `equal` compares a subset of fields (explicit `null` supported); `assert` evaluates a CEL boolean with bindings `it` / `items` / `result` / `results` / `status`. A failing rule **MUST** report the case, rule, selector, and expected-vs-actual.

**Implements**: `cpt-bronze-to-api-e2e-algo-yaml-eval-expect`

**Touches**: `src/ingestion/tests/e2e/lib/expect_engine.py`, `src/ingestion/tests/e2e/meta/test_expect_engine.py`; Components: `cpt-bronze-to-api-e2e-component-expect-engine`

### Reference Test (collab_emails_sent)

- [ ] `p1` - **ID**: `cpt-bronze-to-api-e2e-dod-yaml-reference-test`

The system **MUST** ship one working test `metrics/collab_emails_sent.test.yaml` plus shared `metrics/schemas/{bronze_bamboohr.employees,bronze_m365.email_activity}.yaml` and `metrics/templates/{people,m365_email}.yaml`. It **MUST** exercise the IC Bullet Collaboration metric (`…0012`) over `bronze_m365.email_activity` + `bronze_bamboohr.employees`, assert the team median of `m365_emails_sent` (`[40,20,10] → median 20`, value 40 for the requested person, range `[10,40]`), and prove the Airbyte re-sync duplicate of `alice` does not inflate her sum.

**Implements**: `cpt-bronze-to-api-e2e-flow-yaml-author-and-run`

**Touches**: `src/ingestion/tests/e2e/metrics/collab_emails_sent.test.yaml`, `metrics/schemas/*`, `metrics/templates/*`; integrates all components

## 6. Acceptance Criteria

- [ ] **Given** a fresh checkout, **When** a developer runs `pytest src/ingestion/tests/e2e/ -k collab_emails_sent`, **Then** the reference test passes on a warm session
- [ ] **Given** the reference test passes, **When** the `alice` duplicate stops deduping (regress `union_by_tag` or bronze RMT), **Then** the `m365_emails_sent` rule fails because `value`/`median`/`range` drift
- [ ] **Given** a test references a missing template pointer, **When** the suite is collected, **Then** that test fails at collect time naming the unresolved `$ref`
- [ ] **Given** a resolved bronze record contains a misspelled field, **When** the suite is collected, **Then** schema validation fails naming the unknown field (`additionalProperties:false`)
- [ ] **Given** a batch with one query that errors server-side, **When** the test runs, **Then** the HTTP status is 200 but the `result.status == 'ok'` rule fails with the embedded problem detail
- [ ] **Given** the resolver unit tests, **When** they run without ClickHouse/dbt, **Then** all 12 invariants of `cpt-bronze-to-api-e2e-dod-yaml-ref-resolution` pass

---
name: metric-e2e-test
description: "Author and validate declarative YAML e2e tests for analytics metrics (src/ingestion/tests/e2e/specs/*.test.yaml). Use when asked to write/scaffold/validate an e2e test for a metric, seed bronze data for a test, add a fixture for a dashboard metric, or check a *.test.yaml. Covers schemas/, templates/, $ref+sibling composition, bronze records with duplicates, the batch endpoint POST /v1/metrics/queries, and expect rules (in / mongo-style find / equal subset / CEL assert)."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep
---

# Author a metric e2e test (declarative YAML)

This skill writes and validates `*.test.yaml` fixtures that drive the full
`bronze → dbt silver → gold view → analytics-api` path and assert the result.

## Source of truth (reference — open only if you need the detail)

This skill is self-contained for authoring. Consult these only when you need the
precise algorithm/DoD, or when this file and the spec disagree (the spec wins) —
no need to load them every time:

- FEATURE: [docs/domain/bronze-to-api-e2e/specs/feature-yaml-rig/FEATURE.md](../../../docs/domain/bronze-to-api-e2e/specs/feature-yaml-rig/FEATURE.md) — flows, the `resolve` algorithm, the expect engine, DoD.
- DESIGN: [docs/domain/bronze-to-api-e2e/specs/DESIGN.md](../../../docs/domain/bronze-to-api-e2e/specs/DESIGN.md) — principles `record-composition`, `schema-is-truth`; components `ref-resolver`, `schema-validator`, `expect-engine`.

## Commands

- `/metric-e2e-test create <name> --metric <uuid> --tables <t1,t2>` — scaffold a new `<name>.test.yaml` (+ any missing `schemas/` and `templates/`).
- `/metric-e2e-test validate <path>` — resolve refs, schema-validate records, lint `cases`/`expect` without running ClickHouse.

(Plain prose like "write an e2e test for the emails-sent metric" triggers the same flow.)

## File layout

```
src/ingestion/tests/e2e/specs/
  schemas/<db>.<table>.yaml      # one JSON schema per bronze table (all real columns)
  templates/<group>.yaml         # reusable records (people, m365_email, …)
  <name>.test.yaml               # ONE metric per file (discovered by the *.test.yaml suffix)
```

Files under `schemas/` and `templates/` are NOT tests (no `cases`) and are skipped by discovery.

**One metric per file.** Each `<name>.test.yaml` asserts exactly ONE output `metric_key`
(e.g. `collab_emails_sent.test.yaml` asserts only `m365_emails_sent`). Several files MAY
target the same `metric_id` — the collab email specs (`collab_emails_read`/`received`/`sent`)
all query metric_id `…0012` but each asserts a different `metric_key`. A file may hold many
`cases` (e.g. one per date window), but they all `find` the same single target metric.

## The format

### Records, `$ref`, and overrides

A record is a field map. It may carry `$ref: "<file>#/<json-pointer>"` to inherit
from another record; **sibling keys override the base** (closest wins). Paths are
relative to the file the `$ref` is written in; a `$ref` resolves in the context of
its own file (a `#/...` ref inside `templates/people.yaml` stays local to it).

```yaml
# templates/m365_email.yaml
templates:
  m365_email:            # base — carries EVERY schema column (unused = null)
    _airbyte_raw_id: "00000000-0000-0000-0000-000000000000"
    _airbyte_extracted_at: "2026-01-05T00:00:00"
    _airbyte_meta: "{}"
    _airbyte_generation_id: 0
    tenant_id: "00000000-0000-0000-0000-000000000000"
    source_id: m365-test
    sendCount: null
    # … every other column …
  alice_email:
    $ref: "#/templates/m365_email"
    userPrincipalName: alice@example.com
```

### `bronze` — what to seed

Keyed by table name (the key IS the table + which schema validates it). Each row =
`$ref` to a record + the fields under test. After resolution the row is **padded to
the full schema** (missing columns → null) and validated (`additionalProperties:false`
catches typos). Two identical rows = a real Airbyte re-sync duplicate (must dedup).

```yaml
bronze:
  bronze_m365.email_activity:
    - $ref: templates/m365_email.yaml#/templates/alice_email
      reportRefreshDate: "2026-01-05"
      unique_key: m365-alice-20260105
      sendCount: 40
    - $ref: templates/m365_email.yaml#/templates/alice_email   # duplicate → must NOT double
      reportRefreshDate: "2026-01-05"
      unique_key: m365-alice-20260105
      sendCount: 40
```

### `cases` — batch request + expectations

```yaml
cases:
  - name: <what this proves>
    request:
      url: /v1/metrics/queries
      method: POST
      body:
        queries:
          - id: collaboration            # echoed back as results[].id
            metric_id: <uuid>
            $filter: "person_id eq 'alice@example.com' and metric_date ge '2026-01-01' and metric_date le '2026-01-31'"
    expect:
      - assert: "status == 200"                       # HTTP code of the batch
      - in: collaboration
        assert: "result.status == 'ok'"                # this query's own status (batch HTTP stays 200 on per-query error)
      - in: collaboration
        find: { metric_key: m365_emails_sent }         # mongo-style selector → exactly one row (`it`)
        equal: { value: 40, median: 20, range_min: 10, range_max: 40 }   # subset; unlisted fields ignored
```

- `in` — select the batch result by request `id` (omit when there is one query).
- `find` — exact field equality: `{field: value}` (selects one row). Anything richer (inequalities, counts, predicates) goes in a CEL `assert` — there is no second selector language.
- `equal` — subset equality; use for exact ints / `null`.
- `assert` — CEL boolean; use for inequalities / floats / counts.

The request carries only `id` + `metric_id` + `$filter` (person_id + metric_date `ge`/`le`):
the live FE sends **no** `$top`/`$orderby`/`org_unit`, and the backend computes the team
(org_unit) cohort itself. **Assert only the metric under test** — one `metric_key` per file
via `find`+`equal`; do NOT assert a fixed positive `size(items)` count or an unrelated
metric_key. (The team shown in the UI comes from a *separate* identity service and can
disagree with the analytics cohort — irrelevant to these assertions.)

### `assert` (CEL) bindings

Assembled in `e2e_lib/expect_engine.py::evaluate_case` (the `bindings` dict),
converted to CEL in `_eval_cel`:

| Binding | Value | Present when |
|---|---|---|
| `it` | the single row matched by `find` | only with `find` (else `null` → `it.x` errors) |
| `items` | the selected result's `items` array | a result is selected (`in` or sole query) |
| `result` | the selected result `{id, status, metric_id, items, page_info}` | a result is selected |
| `results` | the full `results[]` of the batch | always |
| `status` | the batch HTTP status code (int) | always |

CEL is strictly typed and won't compare an `int` to a `double` — when a metric
value may be integral (`40`) and you compare against a fractional literal, cast it:
`double(it.value) > 39.5`. `status`/`size(...)` are ints (compare with int literals).
For exact / `null`, use `equal` (Python `==`), not `assert`. CEL macros available:
`size()`, `has()`, `.exists()`, `.all()`, `.map()`, `.filter()`.

## Date-window test design (metric_date)

`metric_date` bounds are **inclusive on both ends**: `metric_date ge '<lo>' and metric_date le '<hi>'`
includes rows ON `<lo>` and ON `<hi>`. When a metric is date-windowed, prove the bounds
with a dedicated spec (see `specs/collab_emails_read.test.yaml`):

- **Boundary-value (BVA):** seed rows one-day-BEFORE the lower bound (must be excluded),
  AT the lower bound (included), AT the upper bound (included), and one-day-AFTER the upper
  bound (excluded). Choose seed dates so each window's period SUM is unique, so a wrong /
  off-by-one bound fails the `equal`.
- **Single-day (degenerate):** `ge == le` — a one-day window still matches.
- **Cross-year:** a window spanning the year boundary (e.g. `ge '2025-12-31' le '2026-01-01'`), both bounds inclusive.
- **Empty window:** a valid range with no rows → `assert: "size(items) == 0"` (a bare CEL assert, not `equal`).
- **Equivalence partitions:** one case per dashboard window kind — week / month / quarter /
  custom — the FE issues these as distinct `ge`/`le` ranges.

## Scaffolding a new test

1. **Resolve the metric_id and its shape.** Find it in the seed catalog
   (`grep -rn "<label>" src/backend/services/analytics-api/src/migration/*.rs`) and
   the live `query_ref` rewrite for that metric. Note whether it returns a bullet
   (`metric_key`/`value`/`median`/`range_*`) or per-person rows. For the collaboration
   bullets the median/range cohort is **DEPARTMENT/org_unit-scoped for BOTH** the Team
   bullet (`…0005`) and the IC bullet (`…0012`) — `median`/`range_*` come from
   `quantileExact`/min/max over the person's own `org_unit_id` cohort (live query
   `m20260604_000002_collab_bullet_distribution.rs`, `GROUP BY metric_key, org_unit_id`
   joined `ON c.org_unit_id = p.org_unit_id`). The two bullets differ only in `value`
   (Team = team average `avg(p.v_period)`/`avg(c.team_*)`; IC = the requested member
   `any(c.team_*)`), NOT in median scope. Ignore the seed catalog's `range=company`
   description — it is stale (the older `20260518` company-wide query was replaced; that
   shape now lives only in `down()`/`old_*_query` for rollback). The `metric_key` strings
   you put in `find: { metric_key: … }` are the **seeded** query-output keys the
   `query_ref` emits (e.g. `m365_emails_sent`); copy the exact literal VERBATIM and never
   invent ids/keys/names — an unseeded key makes `find` match 0 rows and the case fails.
2. **Ensure a schema file per table.** If `schemas/<db>.<table>.yaml` is missing,
   generate it from the REAL table (do not invent columns):
   ```bash
   # Pod/ns/user vary per stand — find yours: `kubectl get pods -A | grep clickhouse`.
   # Current dev stand: pod clickhouse-shard0-0, ns insight-infra, user `insight`
   # (password in the `clickhouse-creds` secret). Bare `clickhouse-client` now fails
   # AUTHENTICATION_FAILED, so pass --user/--password.
   export KUBECONFIG=<path to your dev cluster kubeconfig>
   kubectl exec -n <ch-ns> <ch-pod> -- clickhouse-client --user insight --password '<pwd>' \
     --query "SELECT name, type FROM system.columns WHERE database='<db>' AND table='<table>' ORDER BY position FORMAT TSV"
   ```
   Map CH types → JSON-schema: `Nullable(String)`→`[string,"null"]`, `Decimal/Float/Int`→`[number,"null"]` (`UInt*` non-null →`integer`), `Bool`→`[boolean,"null"]`, `DateTime*`→`{string, format: date-time}`, `JSON`→`[object,"null"]`. Set `additionalProperties: false` and list **every** column (incl. `_airbyte_*`).
3. **Ensure base + variant templates.** The base record must contain every schema
   column (incl. `_airbyte_*` — transforms depend on them); variants `$ref` the base
   and override identity only.

   **Seed the department, not a UUID.** In the GOLD/served layer `org_unit_id` is the
   BambooHR **department STRING** — `insight.people.org_unit_id = argMax(department, …)`,
   keyed `person_id = lower(workEmail)`; it is a UUID only in silver/`person.persons`. So
   for a team/org_unit-cohort metric set `department: "Engineering"` on the bamboohr
   `employees` base record (people sharing a department form one cohort), and if you scope
   a team-view request use the string (`org_unit_id eq 'Engineering'`), never a UUID.

   **Identity match is load-bearing (silent NULL trap).** Cohort attribution is a LEFT
   JOIN: `collab_bullet_rows` joins `insight.people` ON `lower(silver.email) = person_id`,
   where `person_id = lower(workEmail)`. There is no `email` column on bronze — bamboohr
   carries `workEmail`, M365 carries `userPrincipalName`, and silver `email` derives from
   `userPrincipalName`. So a seeded person's `userPrincipalName` must equal their bamboohr
   `workEmail` **case-insensitively**; any mismatch → `org_unit_id` resolves NULL, the
   person silently drops out of the cohort (no error), and the median/range is computed
   over the wrong roster. Set the SAME email on both `workEmail` and `userPrincipalName`.
4. **Write `bronze`** with `$ref`+overrides; include a duplicate row when the metric
   should dedup.
5. **Write `cases`**: one batch `query` per metric under test (and one `metric_key` per
   file — see File layout); assert ONLY the target metric's few fields via `find`+`equal`,
   and counts/inequalities via `assert`.
6. **Pick numbers that distinguish behaviors** — e.g. for a median test use values
   where median ≠ mean (`[40,20,10]` → median 20, mean 23.33) so the test actually
   pins the aggregation. Use an **odd-size** cohort: ClickHouse `quantileExact(0.5)`
   (which both collab bullets use) is NOT the average of the two middle values on an
   EVEN cohort — it returns the UPPER middle element (index `floor(n/2)`): `{100,200}` →
   200, not 150. An even cohort whose median you compute as the mean of the middles will
   produce a wrong `equal:` (this bit the live specs twice), so prefer odd cohorts.

## Validating a test (no ClickHouse needed)

- Every `$ref` resolves (file + pointer exist); no cycles.
- Each resolved+padded bronze record validates against `schemas/<table>.yaml`
  (`additionalProperties:false`).
- Base templates cover **all** schema columns (quick check):
  ```bash
  python3 - <<'PY'
  import yaml
  s=set(yaml.safe_load(open("schemas/<db>.<table>.yaml"))["schemas"]["<db>.<table>"]["properties"])
  t=set(yaml.safe_load(open("templates/<group>.yaml"))["templates"]["<base>"]); t.discard("$ref")
  print("missing", sorted(s-t), "extra", sorted(t-s))
  PY
  ```
- Each `expect` rule has `find`+(`equal`|`assert`) or a bare `assert`; `in` matches a
  declared query `id`; CEL expressions parse.

## Running

```bash
cd src/ingestion/tests/e2e
ls specs/*.test.yaml                       # list existing tests
./e2e.sh test                              # run all tests (specs/ + meta/)
./e2e.sh test -k <name>                    # run one test by name
./e2e.sh test -k <name> -v                 # verbose (per-step log)
./e2e.sh down                              # tear down the e2e compose stack + volumes (full reset)
```

`<name>` is the file stem (e.g. `collab_emails_sent` for `specs/collab_emails_sent.test.yaml`).

Warm re-runs are fine. Isolation is **per-test**: each test first `TRUNCATE`s only the
tables the PRIOR test recorded in a per-test ledger (`CHSeeder.TouchedLedger`;
`specs/test_fixtures.py`, `e2e_lib/ch_seeder.py`), then seeds its own. The rig auto-records
the built **staging** and **silver** models too (not just bronze) — staging especially,
because silver reads staging via the `union_by_tag` macro, so an un-truncated prior staging
row would contaminate the silver rebuild. On TOP of that, a one-time session-start truncate
(`conftest.py`) clears the `class_collab_*` / `m365__collab_*` tables that
`insight.collab_bullet_rows` reads but a single collab fixture does not seed — this only
makes WARM re-runs deterministic (CI starts fresh). `./e2e.sh down` is the e2e compose
teardown (not a deploy), for when you want a fully clean ClickHouse.

To create a new test, use `/metric-e2e-test create` or hand-author `specs/<name>.test.yaml`
as above. There is no `./e2e.sh new` — the old CSV-rig scaffolder was removed when the
declarative `*.test.yaml` rig replaced the CSV rig.

## New bronze table for a not-yet-seeded connector

The seeder INSERTs into a table that MUST already exist (it reads
`system.columns` and fails otherwise — it does NOT create from the schema YAML).
Bronze tables come from `src/ingestion/scripts/create-bronze-placeholders.sh`
(the rig parses the `run_ch <<'SQL' … SQL` heredocs out of it). So to seed a
connector that isn't there yet:

1. Add `CREATE DATABASE IF NOT EXISTS bronze_<snake>;` to the database heredoc.
2. Add a `CREATE TABLE IF NOT EXISTS bronze_<snake>.<stream> (…)` block (inside a
   `run_ch <<'SQL' … SQL` heredoc) with the columns your dbt model reads + the 4
   `_airbyte_*` CDK columns. Real Airbyte overwrites it on first sync.
3. Add a matching `schemas/bronze_<snake>.<stream>.yaml` (every column;
   `additionalProperties: false`) and a base template covering all of them.

## Gotchas (rig operations + cross-test impact)

- **Stale binary / your migration didn't run.** Historically the biggest trap:
  `./e2e.sh` builds analytics-api into the `cargo-target` Docker volume, and on
  Docker Desktop (macOS) the mtimes cargo reads through the bind mount don't
  reliably advance, so cargo relinked a stale object and the binary silently
  lacked new SeaORM migrations (symptoms: `query_ref`/catalog changes have no
  effect, a `find` matches 0 rows, `size(items)` off by your new key). FIXED in
  `e2e_lib/analytics_api.py::build` — it now `touch`es the analytics-api crate
  sources before `cargo build`, forcing a recompile every run (~1-2 min, only
  that crate). So a plain `./e2e.sh test` picks up new migrations now; you should
  NOT need `down -v` for this. If you still suspect a stale binary, confirm by
  querying `seaql_migrations` (below) — your migration version must be present.
- **`1045 Access denied for user 'insight'` at API startup.** Stale
  `compose/.env` creds vs a persisted MariaDB volume. Same `down -v` fixes it.
- **Inspect the live DB after a run.** CH + MariaDB stay UP after `./e2e.sh test`
  (only the runner is `--rm`). Query directly:
  `docker exec insight-e2e-mariadb mariadb -uroot -p"$(grep ^MARIADB_ROOT_PASSWORD compose/.env|cut -d= -f2)" analytics -e "SELECT version FROM seaql_migrations"`
  and `docker exec insight-e2e-clickhouse clickhouse-client -q "SELECT … FROM silver.class_<X>"`.
- **Cross-test impact.** Adding a `metric_key` to a shared bullet section raises
  that section's `size(items)` for EVERY test that asserts a fixed count on it —
  bump those count assertions in the same change (e.g. the Zulip add moved the
  Collaboration bullet 20 → 21). The per-metric specs here deliberately assert
  ONLY their target `metric_key` (not `size(items)` — see the `cases` guidance
  above), which immunises them from this coupling; only a spec that pins a
  positive `size(items)` needs the lockstep bump.

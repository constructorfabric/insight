---
name: metric-test
description: "Author and validate a metric spec for the data-path suite (tests/datapath/metrics/<class>/<name>.test.yaml fixture + test_<name>.py module, run against a compose test-stand instance). Use when asked to write/scaffold/validate a test for a metric, seed bronze data for a test, add a fixture for a dashboard metric, or check a spec. Covers schemas/, templates/, $ref+sibling composition, bronze records with duplicates, account bindings, POST /v1/metric-results, and the assertion helpers (row/equals/contains, one/some over a view, approx)."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep
---

# Author a metric test (YAML fixture + pytest module)

This skill writes and validates metric specs: a `<name>.test.yaml` fixture that
drives the full `bronze → dbt silver → gold view → analytics` path on a compose
test-stand instance, and the `test_<name>.py` module beside it that asserts the
result as a seeded persona.

## Source of truth (reference — open only if you need the detail)

This skill is self-contained for authoring. For the precise algorithm, read the
library itself: `tests/lib/insight_datapath/` — `fixture_loader`, `ref_resolver`,
`schema_validator`, `metric_expect` and `spec_runner` live there, and the committed
specs are the worked examples (`tests/datapath/metrics/ai/test_ai_cost.py` with
`ai_cost.test.yaml` beside it is the reference pair). The stand client the suite
authenticates through is `tests/lib/insight_stand/`.

## Commands

- `/metric-test create <name> --metric <key> --tables <t1,t2>` — scaffold a new `<class>/<name>.test.yaml` (+ any missing `schemas/` and `templates/`).
- `/metric-test validate <path>` — resolve refs, schema-validate records, lint the module; no stand needed.

(Plain prose like "write a test for the emails-sent metric" triggers the same flow.)

## File layout

```
tests/datapath/
  metrics/
    schemas/<db>.<table>.yaml     # one JSON schema per bronze table (all real columns)
    templates/<group>.yaml        # reusable records (people, m365_email, …)
    conftest.py                   # the module-scoped `spec` fixture + the completeness rule
    <class>/<name>.test.yaml      # what to seed: description + bronze
    <class>/test_<name>.py        # what it proves: SPEC = "<name>", one test per case
  identity/                       # the identity lane (a connector's bronze → persons-seed); not this skill
  meta/                           # pure unit tests of the library; no stand
  conftest.py                     # the instance, the caller persona, dbt, the session floor
tests/lib/insight_datapath/       # the library every spec runs through
```

`<class>` is one of `ai`, `ci`, `collab`, `git`, `tasks`, `wiki` — the family the
metric belongs to, and the CI shard it runs in. A spec is the pair; files under
`schemas/` and `templates/` are neither. A spec sits one directory below them, so a
`$ref` from a spec reads `../templates/people.yaml#/templates/alice`.

**Name the spec for the metric under test.** `collab/collab_emails_sent.test.yaml`
is about `collab.emails_sent`. A module may read several keys off one seed when
they share the bronze under test (`git/test_git_metrics.py` reads the git keys off
one seeded set of commits and pull requests; `ai/test_ai_cost.py` pairs `ai.cost` with
`ai.extra_usage_cost` to prove a subset is served beside it, never summed), but
it asserts the keys it requests and never a fixed positive count of unrelated
entries. A module may hold many tests (one per date window, say) over the one seed.

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
    tenant_id: "{{ tenant }}"
    source_id: m365-test
    sendCount: null
    # … every other column …
  alice_email:
    $ref: "#/templates/m365_email"
    userPrincipalName: alice@example.com
```

**Placeholders.** `{{ tenant }}`, `{{ supervisor_email }}` and `{{ supervisor_name }}`
are filled by the run from the instance — the manifest's tenant, and the seeded
lead the caller signs in as — after `$ref` resolution, so they work inside a shared
template. An unsupplied placeholder fails the load; a spec never hard-codes a tenant
or a supervisor.

### `description` — metric + bronze→silver→gold formula

A folded `>` block stating WHAT the metric is and HOW it's computed, in plain
language — **not** dbt model / silver-column names. Keep it short. Shape:

```yaml
description: >
  Metric: <metric_key> — <bullet name> (…0012), #<issue>.
  How it's computed (bronze → silver → gold):
    • bronze: <the raw source report(s) that arrive>
    • silver: <how they're deduped / normalized to per-person/day counts>
    • gold:   <the metric rule — the aggregation, exclusions, cross-source sums>

  Team (median/range = the person's department):
    <one-line member distribution> → median <m>, range [<lo>, <hi>].
  Cases: <one-line list of what each case proves>.
```

- The **gold** line carries the metric-specific logic — e.g. "passive emails
  (received/read) excluded", "Teams + Zoom additive", "longest modality, not the
  sum", "Teams-only — Zoom excluded". This is where a reader learns the real rule.
- Describe the *transformation* in human terms; do NOT name staging models or silver
  columns (`m365__collab_*`, `*_count`) — the **layer flow** is the point, not the
  artifacts. (To trace the real artifacts, read the staging dbt models + the gold
  migration; see "Source of truth".)
- Keep the **Team** line concrete (seeded member values → the resulting
  median/range) so a reviewer can verify the `equals(...)` numbers without reading
  every case. For date-windowed metrics (no single Team), drop the Team line and let
  the Cases line enumerate the window kinds (see `collab/collab_emails_read.test.yaml`).
- Canonical example: `collab/collab_activity.test.yaml`.

### `bronze` — what to seed

Keyed by table name (the key IS the table + which schema validates it). Each row =
`$ref` to a record + the fields under test. After resolution the row is **padded to
the full schema** (missing columns → null) and validated (`additionalProperties:false`
catches typos). Two identical rows = a real Airbyte re-sync duplicate (must dedup).

```yaml
bronze:
  bronze_bamboohr.employees:
    - $ref: ../templates/people.yaml#/templates/alice
  bronze_m365.email_activity:
    - $ref: ../templates/m365_email.yaml#/templates/alice_email
      reportRefreshDate: "2026-01-05"
      unique_key: m365-alice-20260105
      sendCount: 40
    - $ref: ../templates/m365_email.yaml#/templates/alice_email   # duplicate → must NOT double
      reportRefreshDate: "2026-01-05"
      unique_key: m365-alice-20260105
      sendCount: 40
```

**Every person a spec asserts on needs a `bronze_bamboohr.employees` row.** People
are minted by persons-seed from the HR records the spec seeds — never stubbed. A
person with activity rows and no employment record is an address the product is
right to leave unresolved; a spec that seeds an employment record for someone the
product then fails to mint is an error at run time, not a null in the response.

A source whose models read the whole report row (`raw_data`) gets that payload
derived from the record's own fields; state `raw_data: null` only when a
payload-less row is what the case is about.

A top-level `skip: <reason>` makes the module skip instead of run — for a metric
blocked on an external fix.

### Account bindings — what the operator decides

Resolution by email cannot bind a second address to a person, nor an account whose
profile email was never collected. A spec states either as an operator decision, and
the run applies it through the operator API as the admin persona:

```yaml
identity_aliases:                       # every listed email → the canonical email's person
  alice@example.com: [alice.alpha@example.org]

identity_accounts:                      # a source account → a named person, whatever its email
  - {source_type: github, source_id: git-test, account_id: "9001", person: alice@example.com}
  - {source_type: github, source_id: git-test, account_id: "9002", person: excluded}
```

`source_id` is the RAW connector source id (the warehouse hashes it); `person:
excluded` is the reserved bot person. Worked examples:
`ai/identity_alias_collapse.test.yaml`, `git/git_pr_account_attribution.test.yaml`.

### The module — what the fixture proves

```python
"""<What the spec proves — the description block's gold line, in prose.>"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import approx, one, some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "collab_emails_sent"

ALICE = "alice@example.com"


def test_emails_sent_over_january(spec: SpecRun) -> None:
    """The month window takes both rows; the re-synced duplicate does not double the count."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-01-01", "to": "2026-01-31"},
                "metrics": [{"metric_key": "collab.emails_sent", "views": [{"view": "period"}, {"view": "peer"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("collab.emails_sent", "period", entity_id=ALICE).equals(value=40)
    r.row("collab.emails_sent", "peer", entity_id=ALICE).equals(
        target_value=40, p25=10, median=20, p75=40, min=10, max=40, n=3
    )
```

- `SPEC` names the fixture; the module-scoped `spec` fixture (`metrics/conftest.py`) seeds it,
  builds its models, mints its people and builds gold once per module.
- `spec.call(request)` posts the request as the seeded lead persona through the gateway and
  returns a `MetricResponse`. Entity ids in requests and rows are the fixture's emails — the
  runner translates person ids both ways.
- `r.row(metric_key, view, **find)` — exactly one row must match. Then `equals(**fields)` (exact;
  `None` for a served null), `contains(field={...})` (one element of a list field),
  `nonempty(*fields)`, `check(field, predicate, describe)`. By the end of the test every selected
  row must have its view's required fields asserted: period `value`; peer `target_value p25 median
  p75 min max n`; timeseries `points`; breakdown `value`; rollup `value contributing_entity_count`;
  histogram `bins`.
- Whole views: `r.series(key)`, `r.breakdown(key)`, `r.histogram(key)`, `r.rows(key, view)`.
  Select inside them with `one(entries, **selector)` / `some(entries, **selector)`; a dict
  selector matches any element of a list field:
  `one(r.breakdown(k), entity_id=ALICE, dimensions={"key": "tool", "value": "cursor"})`.
- An empty timeseries bucket carries `value: null` — coerce only the point you selected:
  `float(one(points, bucket_start="2026-01-05")["value"]) == approx(2.0)`. `approx()` is the
  module's tolerance (rel 1e-9 / abs 1e-6) for a served number read outside `equals`.
- Every `row` and whole-view call is recorded in `.artifacts/metric_assertions.json`, and every
  requested key in the same ledger, for the coverage gate.

The request carries the entity, the period and the metric under test with its views; the
backend computes the peer (org_unit / department) distribution itself. **Assert what you
request** and never a fixed positive count of unrelated entries. (The team shown in the UI
comes from a *separate* identity service and can disagree with the analytics team
(org_unit) — irrelevant to these assertions.)

## Date-window test design

`metric_date` bounds are **inclusive on both ends**: `metric_date ge '<lo>' and metric_date le '<hi>'`
includes rows ON `<lo>` and ON `<hi>`. When a metric is date-windowed, prove the bounds
with a dedicated spec (see `collab/collab_emails_read.test.yaml`):

- **Boundary-value (BVA):** seed rows one-day-BEFORE the lower bound (must be excluded),
  AT the lower bound (included), AT the upper bound (included), and one-day-AFTER the upper
  bound (excluded). Choose seed dates so each window's period SUM is unique, so a wrong /
  off-by-one bound fails the `equals`.
- **Single-day (degenerate):** `ge == le` — a one-day window still matches.
- **Cross-year:** a window spanning the year boundary (e.g. `ge '2025-12-31' le '2026-01-01'`), both bounds inclusive.
- **Empty window:** a valid range with no rows is served as a null, not a zero →
  `r.row(key, "period", entity_id=ALICE).equals(value=None)`.
- **Equivalence partitions:** one case per dashboard window kind — week / month / quarter /
  custom — the FE issues these as distinct `ge`/`le` ranges.

## Scaffolding a new test

1. **Resolve the metric key and its shape.** The key you request and select rows by is
   the registry key (`ai.cost`, `collab.emails_sent` —
   `src/backend/services/analytics/src/domain/metric_definitions/registry.yaml`); copy
   the literal VERBATIM. An unknown key is a 400, and a requested key the registry lacks
   fails the coverage gate. Note which views the metric serves (period, peer, timeseries,
   breakdown, histogram) and what its distribution is scoped to. For the collaboration
   bullets the median/range is **DEPARTMENT/org_unit-scoped for BOTH** the Team bullet
   (`…0005`) and the IC bullet (`…0012`) — `median`/`range_*` come from
   `quantileExact`/min/max over the person's own `org_unit_id` (department) team (live query
   `m20260604_000002_collab_bullet_distribution.rs`, `GROUP BY metric_key, org_unit_id`
   joined `ON c.org_unit_id = p.org_unit_id`). The two bullets differ only in `value`
   (Team = team average `avg(p.v_period)`/`avg(c.team_*)`; IC = the requested member
   `any(c.team_*)`), NOT in median scope. Ignore the seed catalog's `range=company`
   description — it is stale (the older `20260518` company-wide query was replaced; that
   shape now lives only in `down()`/`old_*_query` for rollback).
2. **Ensure a schema file per table.** If `schemas/<db>.<table>.yaml` is missing, write it
   from the table's `CREATE TABLE` in `src/ingestion/scripts/connectors-ddl/<connector>.sql`
   — the snapshot the suite applies to the instance, so the two cannot disagree. Do not
   invent columns. Map CH types → JSON-schema: `Nullable(String)`→`[string,"null"]`,
   `Decimal/Float/Int`→`[number,"null"]` (`UInt*` non-null →`integer`), `Bool`→`[boolean,"null"]`,
   `DateTime*`→`{string, format: date-time}`, `JSON`→`[object,"null"]`. Set
   `additionalProperties: false` and list **every** column (incl. `_airbyte_*`). An
   existing file such as `schemas/bronze_claude_team.claude_team_code_metrics.yaml` is the
   shape to copy.
3. **Ensure base + variant templates.** The base record must contain every schema
   column (incl. `_airbyte_*` — transforms depend on them) with `tenant_id: "{{ tenant }}"`;
   variants `$ref` the base and override identity only. People come from
   `templates/people.yaml`: its base employee record reports to `{{ supervisor_email }}`,
   which is how a spec's cast lands beneath the caller.

   **Seed the department, not a UUID.** In the GOLD/served layer `org_unit_id` is the
   BambooHR **department STRING** — `insight.people.org_unit_id = argMax(department, …)`,
   keyed `person_id = lower(workEmail)`; it is a UUID only in silver/`person.persons`. So
   for a team/department (org_unit) metric set `department: "Engineering"` on the bamboohr
   `employees` base record (people sharing a department form one team), and if you scope
   a team-view request use the string (`org_unit_id eq 'Engineering'`), never a UUID.

   **Identity match is load-bearing (silent NULL trap).** Team/department attribution is a LEFT
   JOIN: `collab_bullet_rows` joins `insight.people` ON `lower(silver.email) = person_id`,
   where `person_id = lower(workEmail)`. There is no `email` column on bronze — bamboohr
   carries `workEmail`, M365 carries `userPrincipalName`, and silver `email` derives from
   `userPrincipalName`. So a seeded person's `userPrincipalName` must equal their bamboohr
   `workEmail` **case-insensitively**; any mismatch → `org_unit_id` resolves NULL, the
   person silently drops out of the team/department (no error), and the median/range is computed
   over the wrong roster. Set the SAME email on both `workEmail` and `userPrincipalName`.
4. **Write the `description`** (metric + bronze→silver→gold formula + Team/Cases —
   see § `description`; a spec implementing a tracked feature scenario cites it
   here — see § Feature-scenario traceability), then **`bronze`** with `$ref`+overrides; include a duplicate
   row when the metric should dedup.
5. **Write `test_<name>.py`**: one test per window or behaviour, each calling
   `/v1/metric-results` for the metric under test; assert the target metric's rows via
   `r.row(...).equals(...)`, and counts or inequalities over a whole view through `one`/`some`.
6. **Pick numbers that distinguish behaviors** — e.g. for a median test use values
   where median ≠ mean (`[40,20,10]` → median 20, mean 23.33) so the test actually
   pins the aggregation. Use an **odd-size** team (an odd number of members with data): ClickHouse `quantileExact(0.5)`
   (which both collab bullets use) is NOT the average of the two middle values on an
   EVEN team — it returns the UPPER middle element (index `floor(n/2)`): `{100,200}` →
   200, not 150. An even team whose median you compute as the mean of the middles will
   produce a wrong `equals(...)` (this bit the live specs twice), so prefer odd teams.

## Validating a test (no stand needed)

- Every `$ref` resolves (file + pointer exist); no cycles.
- Each resolved+padded bronze record validates against `schemas/<table>.yaml`
  (`additionalProperties:false`).
- Every placeholder is one the run supplies (`tenant`, `supervisor_email`, `supervisor_name`).

All three are what `load` checks, so run it with the run's own substitutions:

```bash
uv run --project tests --frozen --group datapath --no-group dev python -c "from pathlib import Path; from insight_datapath.fixture_loader import load; load(Path('tests/datapath/metrics/<class>/<name>.test.yaml'), substitutions={'tenant': 't', 'supervisor_email': 'lead@example.com', 'supervisor_name': 'Lead'})"
```

- Base templates cover **all** schema columns (quick check, from the repo root):
  ```bash
  uv run --project tests --frozen --group datapath --no-group dev python - <<'PY'
  import yaml
  s=set(yaml.safe_load(open("tests/datapath/metrics/schemas/<db>.<table>.yaml"))["schemas"]["<db>.<table>"]["properties"])
  t=set(yaml.safe_load(open("tests/datapath/metrics/templates/<group>.yaml"))["templates"]["<base>"]); t.discard("$ref")
  print("missing", sorted(s-t), "extra", sorted(t-s))
  PY
  ```
- The module sets `SPEC` to the fixture's stem and every test takes `spec: SpecRun`.
  Lint it with `uv run --project tests --frozen ruff check tests/datapath/metrics/<class>/test_<name>.py`
  — ruff is in the default dependency group, so run this only while no data-path suite is
  running (see Gotchas: shared venv).
- The library's own tests: `uv run --project tests --frozen --group datapath --no-group dev pytest tests/datapath/meta -q`.

## Running

The suite seeds and clears a warehouse, so it needs an instance of its own, raised with
`minimal` (identity and the realm seeded, the warehouse empty):

```bash
./dev-compose.sh test-stand minimal --instance=datapath                                 # once
./dev-compose.sh test-stand test --tree=tests/datapath/metrics/<class> --instance=datapath -q
./dev-compose.sh test-stand test --tree=tests/datapath/metrics/<class> --instance=datapath -q -k <name>
./dev-compose.sh test-stand down --instance=datapath                                    # removes volumes: full reset
```

Trees go ONLY through `--tree=` (repeatable); everything after the leading options is
pytest's (`-q`, `-k`, `--tb=short`). `<name>` is the spec's stem (`collab_emails_sent` for
`collab/collab_emails_sent.test.yaml` + `collab/test_collab_emails_sent.py`). The verb picks
the suite's own dependency group for a `tests/datapath` tree and aims the run at the instance
through three environment variables; running pytest directly means setting them yourself:

```bash
INSIGHT_STAND_ENV_FILE=.env.compose.test-stand-datapath \
INSIGHT_STAND_MANIFEST=src/ingestion/tools/seed/manifest-datapath.json \
INSIGHT_STAND_REALM_EXPORT=deploy/compose/keycloak/realm-insight.generated-datapath.json \
uv run --project tests --frozen --group datapath --no-group dev pytest tests/datapath/metrics/<class>/test_<name>.py -q
```

Warm re-runs are fine. Isolation is **per spec**: before a module seeds, the run empties
every relation the previous spec wrote — its bronze tables and every **staging** and
**silver** model the build materialized (the ledger is filled as dbt is invoked, so a build
that failed partway still leaves its targets registered). Staging matters: silver reads
staging through the `union_by_tag` macro, so an un-cleared staging row would contaminate the
silver rebuild. On top of that, a session-start floor empties every `bronze_*`, `staging`
and `silver` relation that holds rows, because those models are incremental behind a
watermark and rows from an earlier session would keep the fixture's own out. The `identity`
database is never cleared — it holds the persona the suite signs in as.

Per module the run goes: seed bronze → staging (with ancestors) → enrich steps → silver →
identity inputs (full refresh) → persons-seed → account bindings → identity map → gold →
the spec's requests as the seeded lead. The coverage inputs are written at session end:
`.artifacts/metric_definitions.json` (the builtin catalogue, read from the instance's
analytics MariaDB) and `.artifacts/metric_assertions.json` (the ledger). The gate over them:

```bash
python3 tests/lib/insight_datapath/metric_coverage.py --universe-file .artifacts/metric_definitions.json --ledger .artifacts/metric_assertions.json
```

Every builtin metric owes `period` and `timeseries`; `peer` when it has a peer cohort,
`breakdown` when it has dimensions, `histogram` when its computation is `median`. A builtin
metric with no spec fails the required check. In CI (`.github/workflows/e2e-bronze-to-api.yml`)
the job `e2e-datapath` runs one leg per shard (`ai`, `git`, `tasks`, `rest`, `identity`), each
raising a minimal stand on its runner; `metric-coverage-gate` unions the legs' ledgers, and
the umbrella `Run E2E suite` is the required check. It runs on `merge_group` and
`workflow_dispatch`.

To create a new test, use `/metric-test create` or hand-author `<class>/<name>.test.yaml`
and `<class>/test_<name>.py` as above.

## New bronze table for a not-yet-seeded connector

At session start the suite gives the instance the schema a deployment has: it applies the
`src/ingestion/scripts/connectors-ddl/*.sql` snapshot and `scripts/migrations/*.sql`. The
seeder then requires every column a fixture declares to exist in the real table with a
compatible type, and fails on drift. A table the snapshot lacks is created from the schema
YAML as a bare `MergeTree` — enough to seed, but not the shape a connector leaves, so a
spec green on that table proves nothing about a deployment. The snapshot is generated, not
hand-edited; a table added by hand to `connectors-ddl/*.sql` does not survive the next
regeneration. To seed a connector whose bronze tables aren't in the snapshot yet:

1. Make sure the connector is listed in `bootstrap-db/connectors-config.yaml`,
   then regenerate and commit the snapshot (see
   `src/ingestion/scripts/bootstrap-db/README.md`):

   ```bash
   cd src/ingestion/scripts/bootstrap-db
   set -a; source pins.env; [ -f .env ] && source .env; set +a
   ./bootstrap-db.sh connectors-config.yaml
   ./dump-ddl.sh                              # writes ../connectors-ddl/*.sql
   ```

   Commit the resulting `connectors-ddl/*.sql` diff so the new
   `bronze_<snake>.<stream>` tables ship in the snapshot the suite applies.
2. Add a matching `schemas/bronze_<snake>.<stream>.yaml` (every column;
   `additionalProperties: false`) and a base template covering all of them.

## Gotchas (instance operations + cross-test impact)

- **`up` is not `minimal`.** A stand raised with `test-stand up` is seeded through silver;
  the suite refuses it at the door (`SeededWarehouseError`) because clearing those
  relations would delete the roster its caller signs in as. Raise an instance of its own
  with `minimal`.
- **A bare path is not a tree.** `test-stand test tests/datapath/metrics/ai` hands the
  path to pytest beside the default tree, so `tests/stand` runs too and the `datapath`
  dependency group is never selected. Trees go through `--tree=`.
- **Shared venv.** `tests/.venv` holds one dependency set at a time, and `datapath`
  (dbt) conflicts with `dev` (ruff, mypy). Any `uv run --project tests` with a different
  group set — a plain `ruff check` included — re-syncs the venv under a running suite.
  Lint before or after, never during.
- **Editing a `.test.yaml` mid-run corrupts that run.** The `spec` fixture loads the file
  when its module starts, so the edit lands in whichever module is next; the ledger and
  results then describe two versions of the tree. Let the run finish, then re-run.
- **Instance drift.** Without `INSIGHT_STAND_ENV_FILE` a direct pytest run resolves
  `.env.compose.test-stand` — the default instance — and seeds whatever stand that is.
  Every instance uses the same users and database names and differs only by published
  port, so the failure reads as a data mismatch, not a wrong address. The env file names
  the instance (`.env.compose.test-stand-datapath`), and so do the manifest and realm
  export it pairs with.
- **Unknown metric on a fresh instance.** Analytics reads which metrics it can serve
  once, at startup; the session restarts it after applying the schema. A hand-made
  request against a `minimal` instance no suite has touched answers 400 for every key.
- **Inspect the live DB after a run.** The instance stays up after `test`; only `down`
  removes it. Credentials are in the instance env file (the MariaDB database is `analytics`):
  `set -a; source .env.compose.test-stand-datapath; set +a`, then
  `docker exec insight-datapath-clickhouse clickhouse-client --user "$CLICKHOUSE_USER" --password "$CLICKHOUSE_PASSWORD" -q "SELECT … FROM silver.class_<X>"`
  and
  `docker exec insight-datapath-mariadb mariadb -u"$MARIADB_USER" -p"$MARIADB_PASSWORD" analytics -e "SELECT metric_key FROM metric_definitions WHERE origin='builtin'"`.
- **Cross-test impact.** Adding a metric key to a shared response raises the entry
  count for EVERY test that pins `len(...)` of a whole view — bump those in the
  same change. A spec that asserts only the keys it requests is immune to this coupling;
  only a spec that pins a positive count needs the lockstep bump.

## Feature-scenario traceability

When a spec implements a scenario tracked in a feature issue's Testing section,
cite it inside the spec's `description` (`… — #2163 scenario 1`); the full
traceability contract (id-not-prose, box-checking after merge) is the
`quality-vector-tests` skill's tracking section. The scenario's vector has no
marker mechanism in this suite; it lives issue-side only.

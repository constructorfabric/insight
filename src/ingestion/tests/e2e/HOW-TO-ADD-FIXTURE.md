# How to add an E2E fixture

A fixture is a folder under `fixtures/` that exercises ONE API call against
a known bronze input. Adding one is a 3-file commit: input CSVs + spec.yaml
+ expected CSV.

## TL;DR

```bash
cd src/ingestion/tests/e2e

# 1. Scaffold:
./e2e.sh new my_fixture bronze_bamboohr.employees

# 2. Fill in: metric_id + key_columns in spec.yaml, real rows in bronze/*.csv

# 3. Generate the expected response:
./e2e.sh test -k my_fixture --update-snapshots

# 4. Inspect fixtures/my_fixture/expected/response.csv; commit if it
#    matches what production should return.
```

## What lives in a fixture folder

```
fixtures/<name>/
├── bronze/
│   ├── <bronze_schema>.<table>.csv      # one per bronze table you touch
│   └── ...
├── spec.yaml                             # JSON-Schema-validated config
└── expected/
    └── response.csv                      # flat CSV; one row per ApiResponse.items[]
```

Filename rules (the loader fails fast otherwise):

- Bronze CSV: strictly `<bronze_schema>.<table>.csv` — e.g. `bronze_jira.issues.csv`
- `spec.yaml` and `expected/response.csv` are required; everything else is optional

## spec.yaml reference

| Field | Required | Notes |
|---|---|---|
| `spec_version` | yes | Always `1` today |
| `metric_id` | usually | UUID in `metrics` table; required when endpoint references `{metric_id}` |
| `endpoint` | yes | e.g. `/v1/metrics/{metric_id}/query` |
| `method` | no | Default `POST` |
| `request_body` | yes | Sent as JSON for POST/PUT |
| `dbt_selector` | no | Omit for view-only metrics (no silver model in path) |
| `key_columns` | yes | List; used for sort+diff. MUST exist in expected/response.csv |
| `float_tolerance` | no | Default `1.0e-6`. Bump for aggregations |
| `description` | no | Human-readable note |

## Where to get `metric_id`

Three ways, pick the right one for the scenario:

### A. Use a prod-seeded metric (recommended for dashboard regression coverage)

UUIDs are baked into the analytics-api binary's SeaORM migrations:

- [`m20260422_000001_seed_metrics.rs`](../../../backend/services/analytics-api/src/migration/m20260422_000001_seed_metrics.rs) — most dashboard metrics
- [`m20260507_000001_seed_crm_metrics.rs`](../../../backend/services/analytics-api/src/migration/m20260507_000001_seed_crm_metrics.rs) — CRM-specific

Map: row `("00000000000000000001000000000002", "Team Members", ...)` → UUID `00000000-0000-0000-0000-000100000002`.

You can also discover them at runtime once the session is up:

```bash
./e2e.sh shell
curl -s http://127.0.0.1:8081/v1/metrics | jq '.[] | {id, name}'
```

### B. Add a custom metric in seed/metrics.yaml (for narrow smoke tests)

Edit [`seed/metrics.yaml`](seed/metrics.yaml):

```yaml
overrides:
  - id: 00000000-0000-0000-0000-0000face0002       # any UUID you own
    name: My test metric
    description: Direct read of insight.commits_daily for unit X
    query_ref: SELECT person_id, metric_date, commits FROM insight.commits_daily
    is_enabled: true
```

The runner upserts this into MariaDB AFTER analytics-api SeaORM migrations
populate the prod catalog. Reference the UUID from your fixture's
`spec.yaml`. Tracked in [seed/metrics.yaml](seed/metrics.yaml).

### C. POST a metric via the API at session start (not implemented yet)

Possible future enhancement — for now, prefer A or B.

## When you need `dbt_selector`

| Metric reads from | `dbt_selector` |
|---|---|
| `insight.<view>` that's a plain SELECT from bronze | omit (the view evaluates lazily on read) |
| `insight.<view>` that JOINs onto a silver model | required — selector for that silver model with `+` suffix |
| Several silver-layer dependencies | use root selector + `+` (e.g. `+silver_class_focus_metrics+`) |

dbt model names = filenames under `src/ingestion/silver/<domain>/*.sql`
(without `.sql`). Example: `class_focus_metrics`, `class_collab_chat_activity`.

## Writing bronze CSV files

Format:

- First row: column names
- Empty cell = SQL NULL
- Dates in ISO format (`2026-01-15`, `2026-01-15T10:30:00`)
- `_airbyte_*` columns can be omitted — placeholder defaults handle them

Schema lookup:

```bash
./e2e.sh shell
clickhouse-client --host clickhouse -u insight --password "$E2E_CH_PASSWORD" \
    --query "DESCRIBE bronze_bamboohr.employees"
```

Or read [`src/ingestion/scripts/create-bronze-placeholders.sh`](../../scripts/create-bronze-placeholders.sh)
— the placeholder DDLs document every column with its type.

## The `--update-snapshots` workflow

When you don't know the expected response shape ahead of time:

```bash
# 1. Have bronze/*.csv + spec.yaml (no expected/response.csv yet)
./e2e.sh test -k my_fixture --update-snapshots

# Logs:
#   snapshot updated: fixtures/my_fixture/expected/response.csv
#   changes:
#   + ('alice@example.com',)
#   + ('bob@example.com',)

# 2. Read the file:
cat fixtures/my_fixture/expected/response.csv

# 3. Decide: is this what the API SHOULD return for the bronze input I gave?
#    If yes → commit.
#    If no  → fix the bronze input OR the query_ref, repeat.
```

`--update-snapshots` refuses to run under `CI=true` — the framework will
never auto-acknowledge a regression in PR builds.

## Common pitfalls

1. **Wrong UUID format**: spec uses `00000000-0000-0000-0000-XXXXXXXXXXXX` (with dashes). MariaDB stores it as `BINARY(16)`; the runner converts via `uuid.UUID(...).bytes` in `metric_seed.py` and in test queries. You don't pass binary directly in `spec.yaml`.

2. **Metric not found 404**: usually means `metric_id` is wrong OR the metric is `is_enabled = false`. Prod metrics default to enabled; custom ones in `seed/metrics.yaml` need `is_enabled: true` explicit.

3. **Empty response items**: gold view returned 0 rows. Check:
   - Bronze rows present in CH: `SELECT count() FROM bronze_<schema>.<table>`
   - View filter conditions (most prod views have `WHERE x IS NOT NULL`)
   - For `insight.people`: `workEmail` must be non-empty (the view excludes empties)

4. **Float assert flaky**: bump `float_tolerance` to `1.0e-3` or larger. ClickHouse `round(avg(...), 1)` does not produce bit-stable values across rebuilds of the data.

5. **`tenant_id` filter blocks rows**: prod views often filter `WHERE insight_tenant_id = ?` — your bronze rows MUST use `00000000-0000-0000-0000-000000000000` (nil UUID) since the auth stub returns nil. Some bronze tables don't have `insight_tenant_id` at all; check with DESCRIBE.

6. **dbt build fails on missing source**: the bronze placeholder schema may not match the silver model's expected columns. Either add the column to your CSV (with reasonable values) or open an issue against the placeholders script.

7. **Tests slow**: cargo cache invalidated. Check `docker volume ls | grep cargo`. Rebuild the runner image (`./e2e.sh build`) only when `pyproject.toml` or the Dockerfile changes.

## Running just your new fixture

```bash
# By name:
./e2e.sh test -k my_fixture -v

# Multiple:
./e2e.sh test -k "my_fixture or people_smoke"

# Inside a debugger:
./e2e.sh test -k my_fixture --pdb

# Drop into a shell to inspect ClickHouse/MariaDB after a failure:
./e2e.sh shell
# inside: clickhouse-client --host clickhouse -u insight ...
```

## When a test fails

On column-set or cell mismatch, the framework prints the first 20 mismatched cells as `(key=..., column=..., expected=..., actual=...)` lines in the captured stdout. No need to download an artifact — pytest captures it.

To see what the API actually returned vs what's expected:

```bash
# Run with -s to disable captured stdout, --pdb to drop into a debugger:
./e2e.sh test -k my_fixture -v -s --pdb
```

Inside pdb, you can `print(response.items)` against the live response, or query CH directly.

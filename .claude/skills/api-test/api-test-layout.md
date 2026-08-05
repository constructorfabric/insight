# Layout, naming, markers

## One module per path group

`src/ingestion/tests/e2e/api/` — the map lives in `api/__init__.py` and must
stay accurate:

| Module | Path group |
| --- | --- |
| `test_catalog.py` | `POST /v1/catalog/get_metrics` |
| `test_metrics.py` | `/v1/metrics` CRUD + `/{id}/query` + `/queries` |
| `test_queries.py` | `/v1/queries` CRUD + `/{id}/run` |
| `test_metric_thresholds.py` | `/v1/metrics/{id}/thresholds[/{tid}]` |
| `test_admin_thresholds.py` | `/v1/admin/metric-thresholds[/{id}]` |
| `test_columns.py` | `/v1/columns[/{table}]` |
| `test_metric_definitions.py` | `GET /v1/metric-definitions` |
| `test_persons.py` | `GET /v1/persons/{email}` |
| `test_metric_results.py` | `POST /v1/metric-results` |

A new path group gets a new module **and** a new row in `api/__init__.py`.
Shared constants and request shapes go in `api/endpoint_helpers.py`; fixtures in
`api/conftest.py`; nothing else is importable between test modules except a
fixture, a helper, or `purge_tenant_admin_rows`.

## Module docstring = the route table

Every module opens with the operations it covers and the codes per operation —
this is the index reviewers read first, and it makes a missing case visible
without running the gate:

```python
"""Contract: /v1/metrics path group — definition CRUD + the two query endpoints.

  GET    /v1/metrics              200 list · 200 excludes soft-deleted
  POST   /v1/metrics              201 · 400 query_ref · 415 wrong-ct · 400 off-schema (xfail: #1670)
  GET    /v1/metrics/{id}         200 · 400 non-uuid · 404 unknown · 404 soft-deleted
  ...
"""
```

Keep the columns aligned, use `·` between codes, and name the *reason* for each
non-success code (`400 query_ref`, not just `400`). Mark pinned bugs inline
(`(xfail: #1670)`).

## Test names

`test_<operation>_<status>[_<qualifier>]`, lowercase, no method prefix:

```python
def test_create_metric_201(api) -> None: ...
def test_get_metric_404_soft_deleted(api, scratch_metric: dict) -> None: ...
def test_update_threshold_400_non_uuid(api) -> None: ...
```

The status code belongs in the name — the name is what the gate's reader maps
back to a declared code. Do **not** import the Playwright-style
`METHOD /path - should …` title format; the route table already carries paths,
and pytest ids are read in `-q` output where a long title is noise.

The one-line docstring carries the rule, in `METHOD /path → status: why` shape:

```python
def test_delete_metric_404_unknown(api) -> None:
    """Soft delete is not idempotent: an unknown id is a 404, not a no-op."""
```

Omit the docstring only when the name says everything (`test_get_metric_200`).

## Section ordering inside a module

Happy paths and domain errors first, in route order; then a commented divider
for the mechanical contracts shared by the whole group:

```python
# ── body-parse contracts (415 wrong Content-Type, 400 off-schema) ──────────
```

Put the *reason* the whole block exists in that comment (once), not on each
test.

## Markers

`pytest.ini` runs with `--strict-markers`, so only registered markers exist:

```python
pytestmark = pytest.mark.api      # every module in api/, no exceptions
```

- `api` — this suite. Selected in CI as a path (`./e2e.sh test api/`), so the
  marker is for `-m` filtering, not for lane selection.
- `slow` — add to a case that takes > 5s. Rare here; the suite is the fast lane.
- Do **not** invent markers, and do not reach for `identity`, `fixture`,
  `smoke` or `mutating` — those belong to the other suites.

## Where a test does NOT belong

- Metric arithmetic over seeded bronze data → `metrics/*.test.yaml` (see the
  `metric-test` skill). `api/` asserts the contract, not the numbers.
- Framework helpers (resolvers, validators, the spec loader) → `meta/`, pure
  and runnable without the data plane.
- The identity service's own contract → `identity/`, its own spec and gate.

## See Also

- [api-assertions.md](./api-assertions.md) — what each case asserts
- [api-fixtures.md](./api-fixtures.md) — the resources those cases need
- [api-coverage-gate.md](./api-coverage-gate.md) — how completeness is enforced

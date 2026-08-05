# Fixtures

## Inventory

Session-scoped, from `conftest.py` at the e2e root — the data plane and the
binary under test:

| Fixture | What it is |
| --- | --- |
| `compose_stack` | ClickHouse + MariaDB up (idempotent; attaches if already running) |
| `analytics` | the spawned, health-checked analytics process (`lib/analytics.py`) |
| `identity_stub` | in-process Identity backend so `/v1/persons/{email}` answers 200/404 |
| `session_cfg` | `SessionConfig` — DSNs, paths, run mode |

Function-scoped, from `api/conftest.py` — one resource per test, removed after:

| Fixture | Resource |
| --- | --- |
| `api` | the recording httpx client (the coverage chokepoint) |
| `other_tenant_headers` | `Authorization` for a different tenant |
| `scratch_metric` | `e2e-scratch-*` metric definition, soft-deleted in teardown |
| `scratch_saved_query` | `e2e-scratch-query-*` saved query, hard-deleted in teardown |
| `scratch_threshold` | a legacy threshold on the scratch metric (xfails on #1663) |
| `catalog_metric_id` | a real `metric_catalog` row id (read-only) |
| `admin_threshold_row` | own tenant-scope admin threshold row |
| `seeded_columns` | two `table_columns` rows, inserted straight into MariaDB |
| `tenant_override_definition` | a tenant-scoped `metric_definitions` override row |

Plus `purge_tenant_admin_rows(api, metric_id)` — a plain function, not a
fixture, for pre-cleaning before a create that must not hit a UNIQUE conflict.

## Rules

**Function scope by default.** A test owns its rows. That is what keeps cases
one-per-status-code and order-independent, and it is worth the extra request:
this suite is the fast lane.

**Session scope only for the data plane.** Anything session-scoped is shared
with every other test in the run — never put a *mutable row* there.

**Create through the same client the test uses.** Fixtures call `api`, so their
requests land in the coverage ledger too; a fixture that reached around the
client would hide its own operation from the gate.

**Teardown is best-effort.** A delete-case test already removed the row, so a
404 in teardown is expected:

```python
@pytest.fixture
def scratch_metric(api) -> dict:
    m = create_scratch_metric(api, "e2e-scratch")
    yield m
    api.delete(f"/v1/metrics/{m['id']}")   # 404 here is fine — the test may have deleted it
```

Do not assert the teardown status, and do not wrap it in try/except — an
unchecked delete already tolerates both outcomes.

**Multiple resources → reverse order.** Delete in the reverse of creation so a
child never outlives its parent; with two or more, use an `ExitStack` rather
than hand-ordered `yield` blocks.

**Never mutate the metric catalog.** `catalog_metric_id` is read-only.
`metric_catalog` is the metric-coverage gate's universe, and an added or
modified row skews a different gate in a way that is painful to trace.

## Re-run hygiene

The MariaDB volume survives `./e2e.sh test`, so a rerun starts with the previous
run's rows. Two consequences:

- **Unique-composite creates must pre-purge.** `(metric, tenant, scope)` is
  UNIQUE on admin thresholds, so a plain create would 409 on the second run —
  hence `purge_tenant_admin_rows` before the POST.
- **Names are uuid-tagged.** `f"e2e-scratch-{uuid.uuid4().hex[:8]}"`,
  `f"e2e_cols_{tag}_a"`. Never a fixed literal name for a created row.

Assert **membership, never length**: `assert created["id"] in {m["id"] for m in
body["items"]}`, not `len(body["items"]) == 1`. Leftovers and xdist workers both
break a count, and a count proves nothing the membership check doesn't.

## Seeding without a write endpoint

Some catalogs have no API writer (`table_columns` is operator/migration-seeded;
`metric_definitions` is migration-seeded). Those fixtures insert into MariaDB
through `lib.mariadb` and delete the same ids in teardown:

```python
@pytest.fixture
def seeded_columns(session_cfg: SessionConfig) -> dict:
    tag = uuid.uuid4().hex[:8]
    ...
    mariadb.query(session_cfg, "INSERT INTO table_columns (...) VALUES (...)")
    yield rows
    mariadb.query(session_cfg, "DELETE FROM table_columns WHERE id IN (...)")
```

Requirements for this pattern:

- Only when there is genuinely no endpoint. Say so in the docstring, with the
  reason (`there is no write endpoint for this catalog`).
- Insert the **minimum** columns the read path needs, and a uuid tag in every
  human-visible value.
- Delete by the exact ids you generated — never `DELETE … WHERE name LIKE
  'e2e%'`, which would take another worker's rows with it.
- Explain a NULL tenant when you use one (`tenant NULL = platform-visible`) —
  it is a visibility decision, not a placeholder.
- New seeding code passes values as query parameters rather than interpolating
  them into the SQL string; the two existing fixtures still interpolate and are
  worth converting when next touched.

## Fixture docstrings

State the resource, its cleanup, and any non-obvious *why* — these docstrings
are the only description of the rig's data model a reader gets:

```python
"""A scratch metric (`e2e-scratch-*`, deterministic system.one query_ref);
soft-deleted in teardown so it never leaks into `GET /v1/metrics`."""
```

## See Also

- [api-auth-tenancy.md](./api-auth-tenancy.md) — the client fixtures and their bearers
- [api-assertions.md](./api-assertions.md) — the assert form fixtures use for setup
- `api/conftest.py`, `conftest.py` — the fixtures themselves

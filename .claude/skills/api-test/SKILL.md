---
name: api-test
description: "Write, fix, or review the analytics endpoint contract tests in src/ingestion/tests/e2e/api/ — one pytest case per (path, method, status code), fixtures in api/conftest.py, shared constants in api/endpoint_helpers.py, response bodies validated against the committed OpenAPI spec, and the per-operation coverage gate in lib/api_coverage.py. Use when adding coverage for an analytics route, changing an api/test_*.py module or its fixtures, closing a coverage gap the gate reports, retiring an xfail after a product fix, or reviewing any of the above."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep
---

# Endpoint contract tests (api/)

`src/ingestion/tests/e2e/api/` proves the analytics HTTP contract: **every
operation in the committed OpenAPI spec, one test per (path, method, status
code)**, run against a real analytics binary on real MariaDB + ClickHouse. The
suite has no SKIP_LIST — completeness is enforced by a gate, not by intent.

Two invariants shape everything else:

1. **The spec is the body's source of truth, never the status code's.**
   `docs/components/backend/analytics/openapi.json` is generated from the
   handlers' own types (`cargo run -p analytics -- openapi`, drift-gated by
   `.github/workflows/openapi-specs.yml`), so response *schemas* are
   trustworthy — validate against them via `lib/spec_schema.py`. The per-route
   *status-code* lists are stamped uniformly by `.standard_errors` (#1669), so
   every expected code is asserted by hand in the test.
2. **Coverage is measured, not asserted.** Every request flows through the
   recording client in `AnalyticsProcess.client()`, whose httpx hook feeds
   `lib/api_coverage.py`. A documented operation with no test **fails** the
   gate; an unobserved status code and a pinned-bug exclusion that starts
   passing are **advisories** — reported, and yours to act on.

## Pattern files

Read the one you need; they are the detail this file deliberately omits.

- [api-test-layout.md](./api-test-layout.md) — module map, naming, docstring route table, markers
- [api-assertions.md](./api-assertions.md) — status/body/problem discipline, parametrize, eventual consistency
- [api-fixtures.md](./api-fixtures.md) — fixture inventory, scopes, cleanup, seeding without a write endpoint
- [api-auth-tenancy.md](./api-auth-tenancy.md) — gateway-JWT bearers, cross-tenant cases, 401/403 reachability
- [api-coverage-gate.md](./api-coverage-gate.md) — the ledger, BLOCKED/SKIP_LIST hygiene, xfail policy

## Prerequisites

- The operation exists in `docs/components/backend/analytics/openapi.json`. If
  it does not, the spec is stale — regenerate it (`cd src/backend && cargo run
  -p analytics -- openapi`) before writing tests against a route the gate
  cannot see.
- Read the handler. Declared status codes are boilerplate; the codes a route
  can actually answer are in `src/backend/services/analytics/src/api/`.
- `AGENTS.md`: no production-derived information anywhere. Fixture data is
  synthetic and obviously so (`user@example.com`, `e2e-scratch-<uuid8>`).
- `src/ingestion/CLAUDE.md` is the Python style floor: type hints on every
  signature, `parametrize` over copy-paste, comments only for the *why*.

## Steps

1. **Locate the path group.** One module per group (see
   [api-test-layout.md](./api-test-layout.md)). Extend the existing module
   rather than adding a parallel one.
2. **Enumerate the cases.** For the operation, list the codes the handler can
   answer — success, each validation 400, path-parse 400, 404 (unknown and,
   where soft delete exists, deleted), 415, and the auth/tenancy codes from
   [api-auth-tenancy.md](./api-auth-tenancy.md). One test per code.
3. **Reuse the resource fixtures.** `api/conftest.py` already creates and
   removes scratch metrics, saved queries, thresholds, admin rows and seeded
   catalog rows. Add a fixture only when no existing one fits, and never mutate
   the metric catalog — it is the metric gate's universe.
4. **Write the tests.** Assert the exact status code, then the body through
   `spec_schema.assert_matches_spec` (success) or `spec_schema.problem`
   (errors). See [api-assertions.md](./api-assertions.md).
5. **Update the module docstring route table** — it is the suite's index, and
   review reads it before the code.
6. **Run and gate:**
   ```
   cd src/ingestion/tests/e2e
   ./e2e.sh test api/            # whole suite: a -k subset under-fills the ledger
   ./e2e.sh gates api            # per-operation + per-code coverage
   ```
7. **Reconcile the gate.** A newly reachable code, a fixed bug, or a new
   operation all require an edit in `lib/api_coverage.py` — see
   [api-coverage-gate.md](./api-coverage-gate.md). Never silence the gate with
   a skip.

## Fixing common failures

| Symptom | Where to look |
| --- | --- |
| `SpecError: … matches no operation` | Route not in the committed spec — regenerate the spec, or the test is calling the wrong path |
| `SpecError: … declares no <status> response` | Handler answers a code the spec omits (the #1670 family) — assert it with `problem()` and pin the gap with a strict xfail |
| `body violates the committed OpenAPI schema` | Real drift: either the handler changed without a spec regen, or the test built the wrong expectation |
| Gate (blocking): operation exercised by no test | Missing case — add it; `SKIP_LIST` is empty by design |
| Gate (advisory): BLOCKED code now observed | A bug was fixed or the spec was corrected — delete the exclusion and the matching xfail |
| Strict xfail now XPASSes | Same thing from the other side — retire the xfail and its `BLOCKED` codes together |
| Passing locally, failing on a re-run | Leftover rows in a persistent volume; pre-purge (see `purge_tenant_admin_rows`) or uuid-tag the fixture data |

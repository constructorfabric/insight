---
name: stand-api-test
description: "Write, fix, or review HTTP contract tests in tests/stand/api/ — the deployed-stand suite against a real gateway, real Keycloak sessions and real backend images. Covers the operation catalogue, which persona session to take, requires_seed markers, the scratch-resource policy, the hand-written vs generated response models, status-code discipline (404-not-403, 400 vs 415 vs 422) and the endpoint coverage gate. Use when adding or changing anything under tests/stand/api/, closing a coverage-gate gap, or turning a stand-scenarios claim into an API case. For browser journeys use stand-ui-test; for the in-process analytics rig under src/ingestion/tests/e2e/api/ use api-test instead — they are different suites with different rules."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep
---

# Endpoint contract tests on the stand (`tests/stand/api/`)

HTTP tests against a **deployed** Insight — the gateway BFF in front of pinned
service images, sessions won by real Keycloak logins. No browser.

**This is not `src/ingestion/tests/e2e/api/`.** That rig runs an analytics
binary in-process with auth disabled and owns four blocking coverage gates; use
the `api-test` skill for it. The two suites prove different things and the
rules differ — most sharply on mutation, because a stand persists between runs
and the rig discards its whole stack.

Environment, sessions and triage: `insight-stand`. What to test:
`stand-scenarios`.

## Layout

One module per path group, split by service — because that is the axis along
which a test's setup differs. Identity's answers depend on **who is asking**
(the org chart, the admin row, the kind of principal); analytics' mostly on
**what was created**.

| Path | Holds |
|---|---|
| `api/conftest.py` | the `api` client, scratch fixtures, the leak detector, catalogue export |
| `api/operations.py` | every operation the gateway routes, named once |
| `api/scratch.py` | the mutation policy and the resources that implement it |
| `api/schemas/` | response models — see *Models* below |
| `api/analytics/` | `/api/analytics`, one module per path group |
| `api/identity/` | `/api/identity`, one module per concern |
| `api/test_gateway.py` | the edge — 401 swept over every catalogued operation at once |

Extend the existing module for a path group; never add a parallel one.

## The module docstring is the index

Open every module with the route it covers and the codes this module asserts:

```python
"""`POST /v1/visible-persons` — filtering person ids to what the caller may see.

    POST /v1/visible-persons   200 · 415 wrong-ct

<why this route is worth a deployed-path test, and what a regression would leak>
"""
```

Then say what the test is *for*, not what it does. The shipped modules explain
the trap — which failure the assertion rules out, which prior defect motivated
it, which other module holds the same story. That is the house style; match it.

## The catalogue, and why 401 lives in one place

`operations.py` is read from the **route tables** in
`src/backend/services/{analytics,identity-resolution}/src/api/` — *not* from
the committed OpenAPI documents. The identity one is still the .NET contract
and is stale in both directions.

Two consumers, one list:

- `test_gateway.py` asserts 401 for **every** row.
- the per-service modules assert what each operation does *with* a session.

**Never write a per-module 401 test.** A 401 alone proves nothing — the gateway
rejects at the edge before routing, so a path that does not exist answers 401
too. The refusal only means "refused" when the same url is shown to serve
something, which is why the sweep and the service modules must build urls from
one catalogue.

Adding an operation means adding a row to `ANALYTICS_OPERATIONS` or
`IDENTITY_OPERATIONS`. Path parameters use the `SOME_ID` stand-in so the
`template` derives itself; the coverage gate groups by template, and a test
hitting a real id would otherwise be reported as swept-only.

## Choosing the caller

The choice *is* the test in most identity cases. Full table in `insight-stand`;
the ones that decide a case:

| Want | Fixture |
|---|---|
| an ordinary authenticated caller | `api` (a lead — analytics has no admin gate at all) |
| a specific role in the org | `session_for("dev_lead").client` |
| **administrative authority** | `admin_operator_session` |
| **a senior person's view of the org** | `realm_admin_session` |
| a caller in another tenant | `other_tenant_session` |
| `/internal/*` | `service_client` — direct to identity, not gateway-fronted |
| genuinely anonymous | `api_client` |

`realm_admin_session` is **403** on admin-gated routes: `require_admin` reads an
active `admin` row in `identity.person_roles`, never the `insight-admin` realm
role. If a test needs that refusal, take the realm admin; if it needs the route
to answer, take the operator.

Declare the people you depend on:

```python
@pytest.mark.requires_seed("dev_lead", "development_ic", "sales_ic", "other_tenant_lead")
```

Validated at **collection** time against the manifest's `fixtures{}` catalogue,
so a wrongly-seeded stand aborts once naming every missing name.

## Status-code discipline

**Bodies from the spec; status codes never.** Per-operation code lists are
stamped uniformly by `.standard_errors` and describe nothing (#1669); the
identity contract fails the same way by listing only `200`. Every expected code
is asserted per test, from behaviour you read in the handler.

| Situation | Code | Why it matters |
|---|---|---|
| resource outside the caller's scope | **404, not 403** | a 403 confirms the row exists — the refusal itself would leak the org's shape |
| missing grant on an admin route | **403** | the route exists and the caller is known; nothing is leaked by saying so |
| path segment is not a UUID | **400** | `Path<Uuid>` fails deserialization before any handler logic |
| body with the wrong media type | **415** | refused on media type, not parsed |
| body parses but is off-schema | **422** or 400 — assert what the extractor chooses | do not assume; probe |
| unknown metric key | **400, not 404** | it is a bad request, not a missing resource |

Assert the code first, with the body in the message so a failure is diagnosable
in one read:

```python
assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
```

## Models

Read `api/schemas/__init__.py` before adding one — the halves are not
interchangeable:

- `common.py`, `identity.py` — **hand-written**. Identity's committed contract
  is the stale .NET document; generating from it would record its errors as
  fact. `identity.py` names its Rust source file for field.
- `analytics.py`, `authenticator.py` — **GENERATED** by
  `tests/generate_schemas.py` from documents the services emit themselves and
  CI drift-gates. Committed, ruff-excluded, and never edited by hand. Verify
  with `--check`; a test run must never need the generator.

`response.parse(Model)` is itself an assertion: that the payload matched a
shape the contract declares, rather than something the models were loosened to
accept. Narrowing a union (`assert isinstance(view.root, PeriodView)`) is an
assertion too — a different variant means the service answered a question
nobody asked.

## Creating rows

The stand is read-only by contract and reset by volume teardown, never by
TRUNCATE. The exception has an exact shape:

1. Create **through the API**, and delete. Never a database connection — that
   is a back door around the deployed path, which is the only thing this suite
   exercises.
2. Every row carries `SCRATCH_PREFIX` plus the session's `RUN_TAG`.
3. **The metric catalog is out of bounds** — it is the metric gate's universe.
4. Teardown deletes are unchecked: a delete-case test already removed its row.

`no_scratch_rows_survive` fails the session if anything leaks. A leak does not
break the run that made it — it changes what the next run sees, and gets
diagnosed as flakiness.

## What this suite must not assert

- **No metric values.** `golden_metrics` is empty by design. Assert that a
  value is non-null, that entity ids match, that a union resolved — never a
  number.
- **No minted tokens.** Sessions are won by driving the deployed OIDC chain.
  Minting is the rig's path and would mean never exercising the login.
- **Nothing from `src/ingestion/tests/e2e/**`.** That rig is read-only
  reference.
- **No production-derived data** (`AGENTS.md`). The roster is synthetic
  (`@company.nonpresent`); scratch names are `stand-scratch-<tag>`.

## Coverage

Every request records into `.artifacts/stand_observed_endpoints.json`; the
catalogue exports to `.artifacts/stand_operations.json`; the gate
(`tests/lib/insight_stand/coverage.py`) compares them. Both are written
unconditionally — a failing run's ledger is the more useful one.

The gate's own behaviour is tested in `tests/stand/meta/test_coverage_gate.py`.
When changing matching rules, that is the file to extend.

An operation the sweep touched but no test exercised is **not covered**. That is
the gate's central rule and the reason `template` exists.

## Procedure

1. Read the handler in `src/backend/services/<service>/src/api/`. Declared codes
   are boilerplate; reachable codes are in the code.
2. Add the operation to `operations.py` if it is new.
3. Pick the caller — the choice is usually the test.
4. Enumerate cases: success, each validation 400, path-parse 400, 404 unknown,
   415, and the scope/tenant refusals. One test per code.
5. Declare `requires_seed` for every person named, plus any capability marker.
6. Write the tests; extend the module docstring's route table.
7. Run and check the ledger:

```bash
./dev-compose.sh test-stand test tests/stand/api/identity/test_subchart.py
uv run --project tests --frozen ruff check tests/
uv run --project tests --frozen python tests/generate_schemas.py --check
```

8. Hand a claim marked `EXPECTED TO FAIL` to `file-bug-insight` rather than
   softening the assertion.

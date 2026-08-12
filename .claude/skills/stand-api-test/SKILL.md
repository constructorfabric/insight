---
name: stand-api-test
description: "Write, fix, or review HTTP contract tests in tests/stand/api/ — the deployed-stand suite against a real gateway, real Keycloak sessions and real backend images. Covers the operation catalogue, which persona session to take, requires_seed markers, the scratch-resource policy, the hand-written vs generated response models, status-code discipline (identity 404 vs analytics 403 outside a scope, 400 vs 415 vs 422) and the endpoint coverage gate. Use when adding or changing anything under tests/stand/api/, closing a coverage-gate gap, or turning a stand-scenarios claim into an API case. For browser journeys use stand-ui-test. The in-process analytics HTTP rig that once lived at src/ingestion/tests/e2e/api/ (the api-test skill) was retired in favour of this suite — only the data-path metrics rig remains in-process (metric-test)."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Write, Edit, Glob, Grep
---

# Endpoint contract tests on the stand (`tests/stand/api/`)

HTTP tests against a **deployed** Insight — the gateway BFF in front of pinned
service images, sessions won by real Keycloak logins. No browser.

**`src/ingestion/tests/e2e/api/` no longer exists.** That in-process rig's
HTTP contract lanes were retired in favour of this suite ("refactor(e2e):
retire the rig's HTTP contract lanes, keep the data path"); the `api-test`
skill describes the retired suite and remains only as history. The data-path
metrics rig stays in-process — see `metric-test`.

Environment, sessions and triage: `insight-stand`. What to test:
`stand-scenarios`.

## Layout

One module per path group, split by service — because that is the axis along
which a test's setup differs. Identity's answers depend on **who is asking**
(the org chart, the admin row, the kind of principal); analytics' mostly on
**what was created**.

| Path | Holds |
|---|---|
| `api/conftest.py` | the `api` client, the one scratch fixture (`scratch_saved_query`), the leak detector, catalogue export |
| `api/operations.py` | every operation the gateway routes, named once |
| `api/scratch.py` | the mutation policy and the resources that implement it |
| `api/schemas/` | response models — see *Models* below |
| `api/analytics/` | `/api/analytics`, one module per path group |
| `api/identity/` | `/api/identity`, one module per concern |
| `api/test_gateway.py` | the edge — 401 swept over every catalogued operation at once |
| `analytics/drilldown_matrix.py`, `identity/views.py` | reading helpers and per-metric expectations — shared logic, no assertions |
| `*/test_request_contracts.py` | **route-table properties, as a table**: non-UUID path 400, wrong-media-type 415, off-schema body, admin-gate 403. Put them here, not in your new module |

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
`src/backend/services/{analytics,identity-resolution}/src/api/`. Both services
now emit their own OpenAPI document, but the route tables stay the source
here: the catalogue must list what the gateway *routes*, which includes the
`/internal/*` S2S pair that is deliberately outside the generated document.

Two consumers, one list:

- `test_gateway.py` asserts 401 for **every** row.
- the per-service modules assert what each operation does *with* a session.

**Do not re-sweep 401 per operation.** A 401 alone proves nothing — the gateway
rejects at the edge before routing, so a path that does not exist answers 401
too. The refusal only means "refused" when the same url is shown to serve
something, which is why the sweep and the service modules must build urls from
one catalogue.

A *single* premise-check per module is house style, though, and shipped:
`test_an_unauthenticated_caller_never_reaches_any_of_this` — "proven per
operation by `test_gateway.py`, and spot-checked here so this module carries its
own reason for using a session at all." One per module, never one per
operation.

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
stamped by `.standard_errors` and describe nothing (#1669) — a few analytics
operations escape the stamp and are still not trustworthy. Every expected code
is asserted per test, from behaviour you read in the handler, never lifted from
the document.

| Situation | Code | Why it matters |
|---|---|---|
| **identity** person route, target outside the caller's scope | **404** | a 403 confirms the person exists — the refusal itself would leak the org's shape |
| **analytics** visible-set gate, person outside scope | **403** | the deliberate opposite choice; `/v1/metric-results` and `/v1/metric-drilldown` both answer 403 on the same seeded pair identity 404s on |
| missing grant on an identity admin route | **403** | a 404 would leak that the gate ran *after* the lookup |
| path segment is not a UUID | **400** | `Path<Uuid>` fails deserialization before any handler logic |
| body with the wrong media type | **415** | refused on media type, not parsed |
| body parses but is off-schema | **422** or 400 — assert what the extractor chooses | do not assume; probe |
| unknown metric key | **400, not 404** | it is a bad request, not a missing resource |

Assert the code first, with the body in the message so a failure is diagnosable
in one read:

```python
assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
```

## A known product defect is an xfail, not a softened assertion

When the product does not honour the contract, the suite keeps asserting the
contract and marks the case `@pytest.mark.xfail(strict=True, reason=…)`. The
reason names what a caller receives today and what they should. Strict matters:
when the product is fixed the case XPASSes and fails the run, so the marker
retires itself instead of quietly hiding a regression later.

`strict=False` is for a case that is genuinely non-deterministic, not for one
you are unsure about — reach for it rarely and say why in the reason.

The alternative the suite also uses: assert the behaviour as it *is* and name
the intended contract in a comment beside it, when a caller's real experience is
the more useful thing to keep visible (`LEGACY_422` in
`analytics/test_request_contracts.py` is the worked example).

Either way, never soften the assertion to match the bug and move on — that is
the one option that loses the information. A claim marked `EXPECTED TO FAIL` by
`stand-scenarios` lands here as a strict xfail.

## The client, in one place

Everything a test needs comes from the top-level `insight_stand` package:
`analytics_path(suffix)` / `identity_path(suffix)` to build a url,
`ApiClient.get/post/put/delete(path, json_body=…, content=…, headers=…)`,
and on the response `.status_code`, `.text`, `.content_type`,
`.parse(Model)`.

## Models

Read `api/schemas/__init__.py` before adding one — the halves are not
interchangeable:

- `common.py`, `identity_internal.py` — **hand-written**. `common.py` is the
  error envelope and listing wrapper; `identity_internal.py` covers the two
  `/internal/persons/*` S2S routes, which are registered raw and stay out of
  the generated document by design.
- `analytics.py`, `authenticator.py`, `identity.py` — **GENERATED** by
  `tests/generate_schemas.py` from documents the services emit themselves and
  CI drift-gates. Committed, ruff-excluded, and never edited by hand. Verify
  with `--check`; a test run must never need the generator. The generated
  identity models carry the contract's names (`SubchartResponse`,
  `ProfileResponse`); the package re-exports them under the names the suite
  already used, so that rename stops at `schemas/__init__.py`.

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
2. Every row carries `SCRATCH_PREFIX` plus the session's `RUN_TAG` — and it must
   come from `scratch.scratch_name(tag)`, with the row registered via
   `scratch.track(listing_path, id_field, value)`. Formatting the name by hand
   satisfies the rule and silently blinds the detector, because the issued-name
   registry is populated only inside `scratch_name`.
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
- **Never import from or edit `src/ingestion/tests/e2e/**`.** Read it freely —
  `coverage.py` and `tests/stand/meta/` are deliberate ports of rig files — but
  the dependency runs one way.
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
5. Declare `requires_seed` for every person named, plus the capability marker if
   one applies — `requires_service_principal` is **mandatory** for any
   `/internal/*` case, or the test hard-fails on a stand that cannot reach the
   token endpoint instead of skipping with a reason.
6. Write the tests; extend the module docstring's route table. Every test
   carries exactly one quality-vector marker — module-level `pytestmark`
   when the whole module shares a vector, per-test markers throughout a
   mixed module, never both; the why lives with the marker declarations in
   `tests/pyproject.toml`. Contract correctness is `reliability`; access,
   tenancy and refusal-of-access cases are `security`; catalog-breadth
   checks are `versatility`. Collection aborts on any other vector count.
7. Run and check the ledger:

```bash
# NOT a subset: the verb appends your path to a hardcoded `tests/stand`, and
# pytest unions path arguments — so this runs the whole suite, browsers included.
./dev-compose.sh test-stand test -k subchart

# a genuine subset needs pytest directly
uv run --project tests --frozen pytest tests/stand/api/identity/test_subchart.py

uv run --project tests --frozen ruff check tests/
uv run --project tests --frozen python tests/generate_schemas.py --check
```

Then reconcile the gate, which is what "closing a coverage gap" means:

```bash
python3 tests/lib/insight_stand/coverage.py \
  --observed .artifacts/stand_observed_endpoints.json \
  --catalogue .artifacts/stand_operations.json
```

8. Hand a claim marked `EXPECTED TO FAIL` to `file-bug-insight` rather than
   softening the assertion.
9. When the test implements a scenario tracked in a feature issue's Testing
   section, cite it in the test docstring (`#2163 scenario 3`) and keep the
   marker equal to the scenario's vector tag; the full traceability contract
   (id-not-prose, box-checking after merge) is the `quality-vector-tests`
   skill's tracking section.

# The coverage gate

`lib/api_coverage.py` is both halves of one mechanism:

- **Recording** — `record_response` is an httpx response hook on
  `AnalyticsProcess.client()`. It stores `(METHOD, path) → {status codes}` for
  every request the suite makes (metadata only, never the body).
  `conftest.pytest_sessionfinish` dumps the ledger to
  `.artifacts/observed_endpoints.json`.
- **Gating** — `python3 lib/api_coverage.py --observed … --spec …` (stdlib only)
  matches each concrete observed path onto a spec path template and reports
  per-operation and per-status-code coverage.

```
cd src/ingestion/tests/e2e
./e2e.sh test api/         # fills the ledger — run the WHOLE suite
./e2e.sh gates api         # reads .artifacts/, prints the matrix, exits non-zero on a violation
```

A `-k` subset run under-fills the ledger and will fail the gate. `.artifacts/`
merges across local runs, so delete it for a from-scratch measurement. CI mirrors
this as an `e2e-api` lane plus a gate job.

## What blocks vs. what is reported

**Blocking (`gate_violations`, exit 1):**

- `MISSING` — a documented operation no test exercises. `SKIP_LIST` is empty by
  design for analytics, so this is the one that fires when a route lands without
  a test.
- `REDUNDANT SKIP` / `STALE SKIP` — a `SKIP_LIST` entry that is now exercised, or
  that left the spec.
- `REQUIRED_EXTRA` violations — unproven, stale, or now-declared. Analytics has
  an empty `REQUIRED_EXTRA`; the identity suite uses it heavily.

**Advisory (`advisories`, never fails):**

- `uncovered code` — a declared code the suite has not observed. This is how a
  per-status-code gap is surfaced; it does not stop the build, so it is on the
  author to close it.
- `blocked-now-observed` / `stale BLOCKED` — the self-cleaning signals on the
  exclusion sets. Treat these as work to do on the spot: they mean a pinned bug
  is fixed or the spec was corrected.

Read the markdown matrix, not just the exit code: `✓` observed, `✗` declared but
unobserved, `·` excluded, blank = not declared for that operation.

## The exclusion sets

`required(op) = declared − 5xx − UNIVERSAL_BOILERPLATE − BLOCKED[op]`

- **`SERVER_FAULT_FLOOR = 500`** — a black-box test cannot deterministically
  induce a server fault, so 5xx is never required.
- **`UNIVERSAL_BOILERPLATE = {401, 429}`** — dropped from every analytics
  operation. 429 has no rate limiter behind it. **401's stated reason ("auth
  disabled at the gateway") no longer matches the rig** — see
  [api-auth-tenancy.md](./api-auth-tenancy.md) before relying on it.
- **`BLOCKED[op]`** — per-operation declared codes the suite cannot observe.
  Every entry carries its reason inline, and the reason is one of exactly two
  kinds:

```python
BLOCKED: dict[str, frozenset[int]] = {
    # spec over-declaration (`.standard_errors` stamps a uniform set, #1669)
    "GET /v1/metrics": frozenset({400, 403, 404, 409}),  # boilerplate: list, no input/lookup/conflict
    # pinned product bug — the SUCCESS code is unobservable until it is fixed
    "POST /v1/metrics/{id}/thresholds": frozenset({201, 403, 409}),  # 201=#1663
}
```

Never add a third kind. "The test is hard to write" is not a `BLOCKED` reason —
write the test.

## Adding an operation

1. Confirm it is in the committed spec (regenerate if not).
2. Write the cases ([api-test-layout.md](./api-test-layout.md),
   [api-assertions.md](./api-assertions.md)).
3. Run the suite and the gate.
4. For each declared code you did not cover, either cover it or add a `BLOCKED`
   entry with one of the two allowed reasons and the issue number. Leaving it as
   an advisory `✗` is a deliberate, visible gap — acceptable when the reason is
   "needs seeded observation data", not when it is "forgot".

## Retiring a pinned bug

When a product fix lands, the mechanism tells you: the strict `xfail` starts
failing ("XPASS(strict)") and the gate advisory reports
`blocked-now-observed`. Then, in one change:

1. Delete the `@pytest.mark.xfail` (or the `pytest.xfail()` branch in the
   fixture) and keep the assertion, which now asserts the real contract.
2. Delete the matching codes from `BLOCKED`.
3. Re-run the suite and the gate; the operation's coverage should rise.

The pairing is the point: an xfail without a `BLOCKED` entry lets a required code
go unobserved with no signal, and a `BLOCKED` entry without an xfail hides a
missing test.

## Path templates

Matching lives in `path_template_index` + `match_path`, shared with
`lib/spec_schema.py` so body validation and coverage always agree on which
operation a request hit. Within a method, templates with fewer `{param}`
segments are tried first, so a literal route wins over a same-arity template
(`POST /v1/metrics/queries` over `POST /v1/metrics/{id}`). An `Observed but
unmatched` entry in the report means the suite called a path the spec does not
document — an undocumented route or a typo, both worth a look.

## Other suites, same module

`--suite` selects the exclusion sets: `analytics` (this suite),
`identity-rust`, `authenticator`. Do not edit another suite's tables while
working on `api/`; they are gated by their own lanes and their reasons differ
(identity, for instance, *requires* 401).

## See Also

- [api-assertions.md](./api-assertions.md) — the xfail half of the pairing
- [api-auth-tenancy.md](./api-auth-tenancy.md) — the 401 exclusion question
- `lib/api_coverage.py` — the tables and the report renderer

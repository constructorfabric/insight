# Assertion discipline

## The two-part assertion

Every case asserts the **exact status code by hand**, then the **body from the
spec**:

```python
from lib.spec_schema import assert_matches_spec, problem


def test_list_metrics_200(api, scratch_metric: dict) -> None:
    r = api.get("/v1/metrics")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    body = assert_matches_spec(r)
    assert scratch_metric["id"] in {m["id"] for m in body["items"]}


def test_get_metric_404_unknown(api) -> None:
    r = api.get(f"/v1/metrics/{UNKNOWN_ID}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"
    problem(r)
```

Why the split: the spec's response *schemas* are generated from the handler
types and are trustworthy; its per-route *status-code lists* are uniform
`.standard_errors` boilerplate and are not. So the code is the test's own claim,
and the body is checked against the generated contract.

- `assert_matches_spec(r)` returns the parsed body — use its return value
  instead of a second `r.json()`. For a declared body-less response (204) it
  returns `None` and asserts the response carries no bytes.
- `problem(r)` validates the RFC-9457 `Problem` component and asserts
  `body["status"]` echoes the HTTP status. Use it on every error case; it works
  even for a code the spec fails to declare.
- `SpecError` from either means the *spec* and the *route* disagree — that is a
  finding, not a test bug to paper over.

## The status-code assertion message is mandatory

```python
assert r.status_code == 201, f"status={r.status_code} body={r.text}"
```

Without the body in the message, a failure on CI is unactionable. Same shape in
fixtures, prefixed with the setup step:

```python
assert r.status_code == 200, f"catalog setup: status={r.status_code} body={r.text}"
```

## Banned assertion forms

```python
# ❌ a set of acceptable codes — the contract has exactly one
assert r.status_code in (200, 201)

# ❌ httpx truthiness — hides which error came back
assert r.is_success
assert not r.is_error

# ❌ branching on the response
if r.status_code == 404:
    assert r.status_code == 404
else:
    assert r.status_code == 200

# ❌ asserting a body shape by hand when the spec declares it
assert "items" in r.json()
assert isinstance(r.json()["items"], list)
```

The single sanctioned escape is a **pinned product bug** whose observable code
is not yet the contract:

```python
@pytest.fixture
def scratch_threshold(api, scratch_metric: dict) -> dict:
    r = api.post(...)
    if r.status_code == 500:
        pytest.xfail("#1663: threshold create 500s on read-back (DECIMAL value vs f64 entity)")
    ...
```

That form requires (a) an issue number, (b) a matching `BLOCKED` entry so the
gate fails the moment the real code appears. See
[api-coverage-gate.md](./api-coverage-gate.md).

## Known-broken contracts: strict xfail, never skip

Assert the **intended** contract and mark it strict, so the fix flips the test
to a failure that must be cleaned up:

```python
@pytest.mark.xfail(
    reason="#1670: off-schema body should be canonical 400; legacy axum::Json returns 422",
    strict=True,
)
def test_create_metric_400_schema_mismatch(api) -> None:
    """Intended: `name` is a String, a numeric value is an off-schema body → 400."""
    r = api.post("/v1/metrics", json={"name": 123, "query_ref": SCRATCH_QUERY_REF})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
```

`pytest.mark.skip` and bare `pytest.skip()` are not used in this suite: a skip
removes the observation from the ledger and dims the gate. `xfail` keeps the
request — and therefore the recorded code — while the expectation stays honest.

## Shared request shapes

`api/endpoint_helpers.py` owns the constants and the awkward requests, so each
mechanical contract has exactly one definition:

| Helper | Purpose |
| --- | --- |
| `SCRATCH_QUERY_REF` | `SELECT 1 AS one FROM system.one` — validator-accepted, runs on any ClickHouse, returns one deterministic row |
| `UNKNOWN_ID` | a never-created v7 UUID, for unknown-id 404s |
| `NON_UUID` | a non-UUID path segment, for `Path<Uuid>` parse 400s |
| `OTHER_TENANT` | a non-nil tenant that is not the session tenant, for cross-tenant cases |
| `text_body_request(client, method, url)` | sends a `text/plain` body to pin the 415 |
| `create_scratch_metric` / `create_scratch_saved_query` | POST + assert 201 + return the body; caller owns cleanup |

Add to this module rather than re-deriving a constant in a test. A raw request
built inline is acceptable only when it is genuinely single-use (e.g. the
malformed-JSON 400 in `test_batch_queries_400_malformed_json`).

## parametrize over copy-paste

Same assertion across variants → one parametrized test with readable ids:

```python
@pytest.mark.parametrize(
    ("field", "value"),
    [("operator", "not-an-operator"), ("level", "excellent")],
    ids=["bad-operator", "bad-level"],
)
def test_create_threshold_400_invalid_field(api, scratch_metric: dict, field: str, value: str) -> None:
    ...
```

Each generated case is a separate ledger observation, exactly like a hand-written
one. Do not parametrize *across status codes* — a case that expects different
codes for different inputs is two tests with two names.

## Eventual consistency

Nothing in `api/` is eventually consistent today: the analytics writes these
routes make are synchronous, so a direct assertion is correct. If a route ever
needs settle time, poll on the condition with a bounded deadline — never
`time.sleep` in a test, and never a retry loop that swallows the last failure:

```python
deadline = time.monotonic() + 10.0
while time.monotonic() < deadline:
    r = api.get(url)
    if r.status_code == 200 and r.json()["items"]:
        break
    time.sleep(0.25)
assert r.status_code == 200, f"status={r.status_code} body={r.text}"
```

If a second call site appears, that loop becomes a helper in `lib/` with a test
in `meta/`.

## See Also

- [api-fixtures.md](./api-fixtures.md) — where the resources under assertion come from
- [api-coverage-gate.md](./api-coverage-gate.md) — the xfail ↔ BLOCKED pairing
- `lib/spec_schema.py` — the validator; `meta/test_spec_schema.py` — its own tests

"""Meta-tests for the coverage gate — no stand, no network.

A gate that prints a blocking violation while exiting 0 is worse than no gate:
it converts a real gap into a green run plus a wall of text nobody reads. These
pin the mechanics that could do that — the verdict, the exit code and the
rendered report all deriving from one predicate — and the two failure modes this
gate exists for.

Ported from the rig's `identity/test_meta_gate.py`, which pinned the same
property for `lib/api_coverage.py`. The tables differ on 403 (see `coverage.py`'s
docstring for why it is required here rather than excluded); the reason for
testing the gate does not.

"""

from __future__ import annotations

from typing import Any

from insight_stand import coverage

# Two catalogued operations, standing in for the real 45.
QUERIES = coverage.Operation(method="GET", path="/api/analytics/v1/queries")
SUBCHART = coverage.Operation(method="GET", path="/api/identity/v1/subchart")
CATALOGUE = [QUERIES, SUBCHART]


def _ledger(rows: dict[tuple[str, str], list[int]]) -> list[dict[str, Any]]:
    return [
        {"method": method, "path": path, "statuses": statuses}
        for (method, path), statuses in rows.items()
    ]


def _spec(paths: dict[str, dict[str, list[int]]]) -> dict[str, Any]:
    return {
        "paths": {
            path: {
                method: {"responses": {str(code): {} for code in codes}}
                for method, codes in methods.items()
            }
            for path, methods in paths.items()
        }
    }


def _passing_catalogue() -> coverage.CatalogueReport:
    """A catalogue with nothing to report, so a spec finding stands alone."""
    return _catalogue_report(
        {
            (QUERIES.method, QUERIES.path): [200],
            (SUBCHART.method, SUBCHART.path): [200],
        }
    )


def _catalogue_report(rows: dict[tuple[str, str], list[int]]) -> coverage.CatalogueReport:
    return coverage.CatalogueReport(catalogue=CATALOGUE, observed=coverage.by_label(_ledger(rows)))


def test_an_operation_only_the_sweep_touched_is_not_covered() -> None:
    """The failure this gate exists for.

    `api/test_gateway.py` calls every catalogued operation anonymously, so every
    one appears in the ledger whether or not a test ever used it. Treating
    presence as coverage would report 100% for a suite that asserts nothing but
    the edge's refusal — which is exactly the shape a naive port of the rig's
    gate would have had.
    """
    report = _catalogue_report(
        {
            (QUERIES.method, QUERIES.path): [200, 401],
            (SUBCHART.method, SUBCHART.path): [401],
        }
    )

    assert report.exercised == [QUERIES.label]
    assert report.swept_only == [SUBCHART.label]
    assert not report.passed

    violations = coverage.violations(report, None)
    assert any("SWEPT ONLY" in v and SUBCHART.path in v for v in violations), violations


def test_a_real_id_counts_against_the_operation_it_belongs_to() -> None:
    """A concrete url folds onto its catalogued template — see `fold_onto_catalogue`."""
    catalogued = coverage.Operation(
        method="PUT",
        path="/api/analytics/v1/queries/01900000-0000-7000-8000-000000000000",
        template="/api/analytics/v1/queries/{id}",
    )
    report = coverage.CatalogueReport(
        catalogue=[catalogued],
        observed=coverage.by_label(
            _ledger(
                {
                    ("PUT", catalogued.path): [401],  # the sweep
                    ("PUT", "/api/analytics/v1/queries/019fc6c8-020f"): [200],
                }
            )
        ),
    )

    assert report.exercised == [catalogued.key]
    assert not report.swept_only
    assert report.passed


def test_a_catalogue_entry_without_a_template_still_matches_itself() -> None:
    """A ledger from a suite that predates templates must not crash the gate."""
    report = _catalogue_report({(QUERIES.method, QUERIES.path): [200]})
    assert QUERIES.key == QUERIES.label
    assert report.exercised == [QUERIES.label]


def test_a_catalogued_operation_nobody_called_fails() -> None:
    """`operations.py` naming a route no test reaches is a gate failure.

    Distinct from swept-only: this one is absent from the ledger entirely, which
    means even the 401 sweep did not reach it — usually a typo'd path, which
    would otherwise sit in the catalogue looking like coverage forever.
    """
    report = _catalogue_report({(QUERIES.method, QUERIES.path): [200]})

    assert report.unobserved == [SUBCHART.label]
    assert not report.passed
    assert any("NEVER CALLED" in v for v in coverage.violations(report, None))


def test_a_fully_exercised_catalogue_passes() -> None:
    report = _catalogue_report(
        {
            (QUERIES.method, QUERIES.path): [200, 401],
            (SUBCHART.method, SUBCHART.path): [200, 401],
        }
    )
    assert report.passed
    assert not coverage.violations(report, None)


def test_the_verdict_and_the_violations_cannot_disagree() -> None:
    """PASS is `no violations`, not a second opinion about them."""
    failing = _catalogue_report({(SUBCHART.method, SUBCHART.path): [401]})
    rendered = coverage.render(failing, None)
    assert "❌ FAIL" in rendered and "✅ PASS" not in rendered
    assert coverage.violations(failing, None)

    passing = _catalogue_report(
        {
            (QUERIES.method, QUERIES.path): [200],
            (SUBCHART.method, SUBCHART.path): [200],
        }
    )
    assert "✅ PASS" in coverage.render(passing, None)


def test_gateway_prefixes_fold_onto_the_service_contract() -> None:
    """`/api/analytics/v1/queries` is the document's `/v1/queries`.

    The gateway strips the prefix before the service sees the request
    (`routes.yaml`, `strip_prefix: true`), so the ledger's gateway paths and the
    spec's service paths describe the same call. Getting this wrong would report
    every operation as unmatched — a 0% that looks like a broken suite rather
    than a broken matcher.
    """
    spec_ops = coverage.spec_operations(_spec({"/v1/queries": {"get": [200, 404]}}))
    validated, unmatched = coverage.match_against_spec(
        _ledger({("GET", "/api/analytics/v1/queries"): [200]}),
        "/api/analytics",
        spec_ops,
    )

    assert validated == {"GET /v1/queries": {200}}
    assert not unmatched


def test_a_path_parameter_matches_its_template() -> None:
    spec_ops = coverage.spec_operations(_spec({"/v1/queries/{id}": {"get": [200]}}))
    validated, _ = coverage.match_against_spec(
        _ledger({("GET", "/api/analytics/v1/queries/abc-123"): [200]}),
        "/api/analytics",
        spec_ops,
    )
    assert validated == {"GET /v1/queries/{id}": {200}}


def test_a_literal_path_wins_over_a_same_arity_template() -> None:
    """`/v1/queries/run-batch` is not a saved query whose id is "run-batch".

    Both templates have two segments, so ordering decides. Sorting by
    `{param}` count is what makes the answer independent of the order the
    document happened to list them in.
    """
    spec_ops = coverage.spec_operations(
        _spec({"/v1/queries/{id}": {"get": [200]}, "/v1/queries/run-batch": {"get": [200]}})
    )
    validated, _ = coverage.match_against_spec(
        _ledger({("GET", "/api/analytics/v1/queries/run-batch"): [200]}),
        "/api/analytics",
        spec_ops,
    )
    assert validated == {"GET /v1/queries/run-batch": {200}}


def test_401_and_403_are_required_where_a_handler_can_answer_them() -> None:
    """The authorization codes stay required, pinned — on a route that has them.

    `POST /v1/metric-results` is the one analytics operation whose 403 comes
    from a visibility check rather than a role stub, so it is deliberately NOT
    in BLOCKED. in BLOCKED.
    """
    op = "POST /v1/metric-results"
    spec_ops = coverage.spec_operations(
        _spec({"/v1/metric-results": {"post": [200, 401, 403, 429, 500]}})
    )
    report = coverage.SpecReport(spec_ops=spec_ops, validated={op: {200}}, unmatched=[])

    required = report.required[op]
    assert 401 in required and 403 in required
    assert 429 not in required, "no rate limiter fronts this stand"
    assert 500 not in required, "a server fault is not deterministically inducible"
    assert report.uncovered[op] == {401, 403}


def test_403_is_subtracted_only_where_no_handler_can_produce_it() -> None:
    """403 stays a per-route judgement, never universal.

    `.standard_errors` stamps 403 on nearly every analytics route (#1669), so
    requiring it everywhere would demand a response the service has no code to
    send. Which routes can actually refuse, and why, is sourced at
    `_NO_AUTHORIZATION_PATH` in `coverage.py`.
    """
    assert 403 not in coverage.UNIVERSAL_BOILERPLATE, "must stay a per-route judgement"
    assert 403 in coverage.BLOCKED["GET /v1/queries"], "no gate on the saved-query listing"
    assert "POST /v1/metric-results" not in coverage.BLOCKED

    spec_ops = coverage.spec_operations(_spec({"/v1/queries": {"get": [200, 401, 403, 404]}}))
    report = coverage.SpecReport(
        spec_ops=spec_ops, validated={"GET /v1/queries": {200}}, unmatched=[]
    )
    assert report.required["GET /v1/queries"] == {200, 401, 404}, (
        "403 subtracted; a code this list says nothing about is untouched"
    )


def test_an_exclusion_the_document_outgrew_is_reported() -> None:
    """The staleness an operation-level check cannot see.

    A #1669 fidelity fix does not remove the operation — it stops the operation
    over-declaring. An exclusion written against the old text then suppresses
    nothing while still reading as a live judgement about the route, which is
    the worst state for a suppression list to be in. Two entries here reached
    it the moment #2134 landed.
    """
    op = next(iter(coverage.BLOCKED))
    corrected = {op.split(" ", 1)[1]: {op.split(" ", 1)[0].lower(): [200, 401]}}
    report = coverage.SpecReport(
        spec_ops=coverage.spec_operations(_spec(corrected)), validated={op: {200}}, unmatched=[]
    )

    assert report.blocked_undeclared == {op: set(coverage.BLOCKED[op])}
    assert any(
        "stale BLOCKED" in note and "declares" in note for note in coverage.advisories(report)
    )
    assert not any(
        "stale BLOCKED" in v for v in coverage.violations(_passing_catalogue(), report)
    ), "hygiene is reported, never blocking — a corrected document must not fail the gate"


def test_409_is_subtracted_everywhere_a_route_cannot_conflict() -> None:
    """The second sourced exclusion, with one route that really can conflict.

    `already_exists`, `aborted` and `conflict` appear on no read-only, saved-query
    or drilldown-export route, so 409 there is boilerplate the spec declares and
    no handler can send. `POST /v1/metrics` is the exception — a duplicate
    `metric_key` is a real conflict — so it is covered by a test rather than
    blocked, and it is the sole excluded route that does NOT subtract 409.
    """
    non_conflicting = {op for op, codes in coverage.BLOCKED.items() if 409 not in codes}
    assert non_conflicting == {"POST /v1/metrics"}, (
        "only the conflict-capable create route omits the 409 subtraction"
    )
    assert "POST /v1/metric-results" not in coverage.BLOCKED, (
        "#2134 already removed 409 from its declaration; an entry here would be stale"
    )


def test_an_undeclared_code_the_suite_proved_is_reported() -> None:
    """The under-declaration half of #1669.

    A code the route answers but the document omits has no column in the
    matrix, so without this it would be invisible — the suite covers it, the
    contract does not describe it, and only one of those is a problem.
    """
    spec_ops = coverage.spec_operations(_spec({"/v1/queries": {"get": [200]}}))
    report = coverage.SpecReport(
        spec_ops=spec_ops, validated={"GET /v1/queries": {200, 415}}, unmatched=[]
    )

    assert report.undeclared == {"GET /v1/queries": {415}}
    assert any("observed but undeclared" in note for note in coverage.advisories(report))


def test_the_session_ledger_survives_these_tests() -> None:
    """The hazard `meta/conftest.py` exists for, pinned from inside it."""
    coverage.record("GET", "/api/analytics/v1/from-an-earlier-test", 200)

    with coverage.isolated():
        coverage.reset()
        coverage.record("GET", "/api/analytics/v1/only-inside", 500)
        assert coverage.by_label(coverage.observed_rows()) == {
            "GET /api/analytics/v1/only-inside": {500}
        }

    surviving = coverage.by_label(coverage.observed_rows())
    assert surviving["GET /api/analytics/v1/from-an-earlier-test"] == {200}
    assert "GET /api/analytics/v1/only-inside" not in surviving


def test_the_ledger_merges_rather_than_overwrites(tmp_path: Any) -> None:
    """Two partial runs against one stand add up — see `dump()`."""
    target = tmp_path / "ledger.json"

    coverage.reset()
    coverage.record("GET", "/api/analytics/v1/queries", 200)
    coverage.dump(target)

    coverage.reset()
    coverage.record("GET", "/api/analytics/v1/queries", 404)
    coverage.record("GET", "/api/identity/v1/subchart", 200)
    coverage.dump(target)
    coverage.reset()

    merged = coverage.by_label(coverage.load_ledger(target))
    assert merged["GET /api/analytics/v1/queries"] == {200, 404}
    assert merged["GET /api/identity/v1/subchart"] == {200}

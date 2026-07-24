"""Meta-tests for the riskiest harness/gate mechanics — no services spawned.

These exist because a gate that prints a blocking violation while exiting 0
is worse than no gate: they pin verdict/markdown/exit consistency, the strict
list envelope, and the explicit implementation selection.
"""

from __future__ import annotations

import pytest

from identity.contract import list_response
from lib import api_coverage
from lib import identity as identity_lib

pytestmark = pytest.mark.identity


# ── coverage gate: REQUIRED_EXTRA teeth ──────────────────────────────────


@pytest.fixture
def identity_suite():
    """Select the identity suite lists, restoring the defaults afterwards
    (module globals are CLI-scoped state)."""
    saved = (
        api_coverage.SKIP_LIST,
        api_coverage.BLOCKED,
        api_coverage.UNIVERSAL_BOILERPLATE,
        api_coverage.REQUIRED_EXTRA,
    )
    api_coverage.select_suite("identity")
    yield
    (
        api_coverage.SKIP_LIST,
        api_coverage.BLOCKED,
        api_coverage.UNIVERSAL_BOILERPLATE,
        api_coverage.REQUIRED_EXTRA,
    ) = saved


def _spec(paths: dict[str, dict[str, list[int]]]) -> dict:
    return {
        "paths": {
            path: {
                method: {"responses": {str(c): {} for c in codes}}
                for method, codes in methods.items()
            }
            for path, methods in paths.items()
        }
    }


def _ledger(rows: dict[tuple[str, str], list[int]]) -> list[dict]:
    return [
        {"method": m, "path": p, "statuses": statuses} for (m, p), statuses in rows.items()
    ]


SEED_SPEC = _spec({"/v1/persons-seed": {"post": [200, 401, 403]}})


def test_gate_fails_when_required_extra_unproven(identity_suite, monkeypatch) -> None:
    """Every operation touched, but the 202 REQUIRED_EXTRA never observed →
    FAIL and the markdown says so (the exact false-PASS this suite once had)."""
    monkeypatch.setattr(
        api_coverage, "REQUIRED_EXTRA", {"POST /v1/persons-seed": frozenset({202})}
    )
    report = api_coverage.build_report(
        SEED_SPEC, _ledger({("POST", "/v1/persons-seed"): [401, 403]})
    )
    assert not report.passed
    violations = api_coverage.gate_violations(report)
    assert any("MISSING REQUIRED_EXTRA" in v and "202" in v for v in violations), violations
    md = api_coverage.render_markdown(report)
    assert "❌ FAIL" in md and "✅ PASS" not in md, md


def test_gate_passes_when_required_extra_observed(identity_suite, monkeypatch) -> None:
    # Narrow REQUIRED_EXTRA to the one op the synthetic spec declares — the
    # other entries would (correctly) report stale against a one-path spec.
    monkeypatch.setattr(
        api_coverage, "REQUIRED_EXTRA", {"POST /v1/persons-seed": frozenset({202})}
    )
    report = api_coverage.build_report(
        SEED_SPEC, _ledger({("POST", "/v1/persons-seed"): [202, 401, 403]})
    )
    assert report.passed, api_coverage.gate_violations(report)
    assert "✅ PASS" in api_coverage.render_markdown(report)


def test_gate_fails_on_redundant_required_extra(identity_suite, monkeypatch) -> None:
    """The spec now declares the 202 → the entry must be dropped."""
    monkeypatch.setattr(
        api_coverage, "REQUIRED_EXTRA", {"POST /v1/persons-seed": frozenset({202})}
    )
    spec = _spec({"/v1/persons-seed": {"post": [200, 202, 401, 403]}})
    report = api_coverage.build_report(
        spec, _ledger({("POST", "/v1/persons-seed"): [202, 401, 403]})
    )
    assert not report.passed
    assert any(
        "REDUNDANT REQUIRED_EXTRA" in v for v in api_coverage.gate_violations(report)
    )


def test_gate_fails_on_stale_required_extra(identity_suite, monkeypatch) -> None:
    """The operation left the spec → the entry must be dropped."""
    monkeypatch.setattr(
        api_coverage, "REQUIRED_EXTRA", {"POST /v1/persons-seed": frozenset({202})}
    )
    spec = _spec({"/v1/roles": {"get": [200, 401, 403]}})
    report = api_coverage.build_report(spec, _ledger({("GET", "/v1/roles"): [200, 401, 403]}))
    assert not report.passed
    assert any("STALE REQUIRED_EXTRA" in v for v in api_coverage.gate_violations(report))


def test_markdown_verdict_matches_violations(identity_suite, monkeypatch) -> None:
    """PASS in the markdown and a blocking violation are mutually exclusive —
    verdict, violations list, and exit predicate share one source of truth."""
    monkeypatch.setattr(
        api_coverage, "REQUIRED_EXTRA", {"POST /v1/persons-seed": frozenset({202})}
    )
    for ledger in ([401, 403], [202, 401, 403]):
        report = api_coverage.build_report(
            SEED_SPEC, _ledger({("POST", "/v1/persons-seed"): ledger})
        )
        md = api_coverage.render_markdown(report)
        if api_coverage.gate_violations(report):
            assert "❌ FAIL" in md and not report.passed
        else:
            assert "✅ PASS" in md and report.passed


# ── strict list envelope ─────────────────────────────────────────────────


def test_list_response_accepts_the_wire_envelope() -> None:
    items, cursor = list_response({"items": [{"a": 1}], "next_cursor": None})
    assert items == [{"a": 1}] and cursor is None


def test_list_response_rejects_bare_array() -> None:
    with pytest.raises(AssertionError):
        list_response([{"a": 1}])


def test_list_response_rejects_missing_next_cursor() -> None:
    with pytest.raises(AssertionError):
        list_response({"items": []})


def test_list_response_rejects_non_list_items() -> None:
    with pytest.raises(AssertionError):
        list_response({"items": {}, "next_cursor": None})


# ── explicit implementation selection ────────────────────────────────────


def test_implementation_selection_is_explicit(monkeypatch) -> None:
    monkeypatch.delenv("E2E_IDENTITY_IMPLEMENTATION", raising=False)
    monkeypatch.delenv("E2E_IDENTITY_URL", raising=False)
    assert identity_lib.implementation_from_env() == "dotnet"
    monkeypatch.setenv("E2E_IDENTITY_IMPLEMENTATION", "rust")
    assert identity_lib.implementation_from_env() == "rust"
    monkeypatch.setenv("E2E_IDENTITY_IMPLEMENTATION", "cobol")
    with pytest.raises(identity_lib.ApiSpawnError):
        identity_lib.implementation_from_env()


def test_external_url_mode_is_refused(monkeypatch) -> None:
    """Pointing the suite at an arbitrary URL while seeding the local
    throwaway DB would test one deployment's HTTP against another's data."""
    monkeypatch.setenv("E2E_IDENTITY_URL", "http://somewhere:8082")
    with pytest.raises(identity_lib.ApiSpawnError):
        identity_lib.implementation_from_env()


def test_deprecated_lookup_is_a_dotnet_capability() -> None:
    assert identity_lib.supports_deprecated_person_lookup("dotnet")
    assert not identity_lib.supports_deprecated_person_lookup("rust")

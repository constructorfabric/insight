"""Offline guard: every fixture's request actually SCOPES to its own data.

Companion to `test_seed_isolation.py`. That guard proves bronze-key disjointness
(no two fixtures collide at rest). THIS guard proves the other half of shared-world
correctness — that each fixture's *request* narrows to its own rows at query time,
instead of silently falling back to a company-wide (all-fixtures) result. Both are
DB-free static checks, so a bad fixture fails in CI before the stack is built.

Two checks:

A. SUPPORTED-FILTER. The analytics API parses `$filter` by substring-scraping a
   FIXED grammar (services/analytics/src/api/handlers.rs: extract_odata_value /
   extract_odata_in_values + the metric_date injector). A clause it does not
   recognize is SILENTLY DROPPED — no error, HTTP 200 — so the query runs
   UNSCOPED. This is exactly the defect that made the team bullets blend
   company-wide: they filtered `org_unit_id eq`, which the API never implemented
   (only `org_unit_id in`), so the predicate vanished and the value spanned every
   seeded fixture. Every clause a fixture writes must be one the API implements.

B. BENCHMARK-SCOPING. A case that asserts distribution/benchmark stats
   (median / p25 / p75 / range_min / range_max / peer_n) MUST scope its request by
   an identity dimension that namespacing isolates (`person_id eq`/`in`, or
   `org_unit_id in`). A benchmark filtered only by metric_date aggregates over
   whatever is in the shared world — a company-wide blend that silently depends on
   every other fixture. (A metric that is legitimately company-wide / sole-seeded
   can opt out via _COMPANY_WIDE_ACKNOWLEDGED, which forces that to be a conscious,
   documented choice rather than an accident.)
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

from lib.fixture_loader import discover_tests, load as load_test

_METRICS_ROOT = Path(__file__).resolve().parent.parent / "metrics"

# The EXACT ($filter field, operator) pairs handlers.rs implements. Anything else
# is silently dropped by the API, so it must never appear in a fixture.
#   person_id  eq / in                     (extract_odata_value / _in_values)
#   org_unit_id in                         (extract_odata_in_values — NO `eq`)
#   metric_date ge / lt / le               (the date-range injector)
#   drill_id / metric_key / section_id eq  (extract_odata_value)
_SUPPORTED: set[tuple[str, str]] = {
    ("person_id", "eq"), ("person_id", "in"),
    ("org_unit_id", "in"),
    ("metric_date", "ge"), ("metric_date", "lt"), ("metric_date", "le"),
    ("drill_id", "eq"), ("metric_key", "eq"), ("section_id", "eq"),
}

# Response columns that are cohort/company aggregates — asserting one requires an
# identity-scoped request (see check B).
_BENCHMARK_KEYS = ("median", "p25", "p75", "range_min", "range_max", "peer_n")
_BENCHMARK_RE = re.compile(r"\b(?:median|p25|p75|range_min|range_max|peer_n)\b")

# Fixtures that intentionally assert a company-wide / sole-seeded benchmark with no
# identity scope. Add here ONLY with a comment explaining how isolation holds
# (sole-seeder of the metric_key, physical annex, etc.). Empty = every benchmark
# assertion today is identity-scoped, which is the invariant we want to keep.
_COMPANY_WIDE_ACKNOWLEDGED: set[str] = set()

# `<field> <op>` clause detector. We blank out quoted literals first so a value can
# never masquerade as a clause (e.g. a department literally named "R and D", or an
# email). After blanking, every `<word> <op>` in the structure is a real clause.
_QUOTED = re.compile(r"'(?:[^']|'')*'")
_CLAUSE = re.compile(r"\b(\w+)\s+(eq|ne|gt|ge|lt|le|in)\b")


def _clauses(filter_str: str) -> list[tuple[str, str]]:
    structure = _QUOTED.sub(" ", filter_str)
    return _CLAUSE.findall(structure)


def _queries(case: dict) -> list[dict]:
    body = (case.get("request") or {}).get("body") or {}
    return body.get("queries") or []


def _has_identity_scope(filter_str: str | None) -> bool:
    if not isinstance(filter_str, str):
        return False
    clauses = set(_clauses(filter_str))
    return bool(clauses & {("person_id", "eq"), ("person_id", "in"), ("org_unit_id", "in")})


def _asserts_benchmark(case: dict) -> bool:
    for item in case.get("expect") or []:
        equal = item.get("equal")
        if isinstance(equal, dict) and any(k in equal for k in _BENCHMARK_KEYS):
            return True
        assertion = item.get("assert")
        if isinstance(assertion, str) and _BENCHMARK_RE.search(assertion):
            return True
    return False


_FIXTURES = [load_test(p) for p in discover_tests(_METRICS_ROOT)]
_IDS = [ty.name for ty in _FIXTURES]

pytestmark = pytest.mark.smoke


@pytest.mark.parametrize("ty", _FIXTURES, ids=_IDS)
def test_filter_operators_are_supported(ty) -> None:
    """Every `$filter` clause uses a (field, operator) the analytics API implements.

    An unsupported clause is silently dropped, so the request runs unscoped and the
    result blends across all fixtures. `org_unit_id eq` is the canonical trap — the
    API only implements `org_unit_id in`; use `person_id in (roster)` for team/dept.
    """
    bad: list[str] = []
    for case in ty.cases:
        for q in _queries(case):
            f = q.get("$filter")
            if not isinstance(f, str):
                continue
            for field, op in _clauses(f):
                if (field, op) not in _SUPPORTED:
                    bad.append(f"{case.get('name', '?')!r}: '{field} {op}'")
    assert not bad, (
        f"{ty.name}: $filter uses clause(s) the analytics API does NOT implement "
        f"(they are SILENTLY DROPPED -> unscoped/company-wide result): "
        + "; ".join(bad)
        + ". Supported: person_id eq/in, org_unit_id in, metric_date ge/lt/le, "
        "drill_id/metric_key/section_id eq. For team/department scope filter by "
        "person_id IN (the roster), not org_unit_id eq."
    )


@pytest.mark.parametrize("ty", _FIXTURES, ids=_IDS)
def test_benchmark_assertions_are_identity_scoped(ty) -> None:
    """A case asserting cohort stats (median/p25/p75/range/peer_n) must scope its
    request by identity (person_id eq/in or org_unit_id in) so the cohort resolves
    to this fixture, not the whole shared world."""
    if ty.name in _COMPANY_WIDE_ACKNOWLEDGED:
        pytest.skip(f"{ty.name}: explicitly acknowledged company-wide/sole-seeded benchmark")
    unscoped: list[str] = []
    for case in ty.cases:
        if not _asserts_benchmark(case):
            continue
        if not any(_has_identity_scope(q.get("$filter")) for q in _queries(case)):
            unscoped.append(str(case.get("name", "?")))
    assert not unscoped, (
        f"{ty.name}: case(s) assert benchmark stats with NO identity scope "
        f"(person_id/org_unit_id) -> the cohort spans every fixture in the shared "
        f"seed-once world: {unscoped}. Scope by person_id IN (roster), or if the "
        f"metric is genuinely company-wide/sole-seeded add it to "
        f"_COMPANY_WIDE_ACKNOWLEDGED with a reason."
    )

"""Evaluate `expect` rules against a batch response.

Implements `cpt-bronze-to-api-e2e-algo-yaml-eval-expect`
(DoD `cpt-bronze-to-api-e2e-dod-yaml-expect-engine`).

Each rule:
  in:     select the batch result by request id (omit when one query)
  find:   exact field-equality selector → exactly one row of result.items (binds `it`)
  then ONE of:
    equal:  subset equality on the matched row (explicit null supported)
    assert: CEL boolean over bindings `it`, `items`, `result`, `results`, `status`

`find` is intentionally exact-equality only — anything richer (inequalities,
counts, predicates) is expressed in a CEL `assert`, so the rig does not carry a
second selector mini-language (CEL is already the assertion language).

No-unasserted-stat rule
------------------------
A bullet row the API returns carries up to six stat fields:
`value, median, range_min, range_max, p25, p75` (there is no `p50` — the 50th
percentile is returned as `median`). Within one `case`, every `find`-matched row
that CARRIES a stat field MUST have that field asserted — either by an `equal`
key or by referencing it in a CEL `assert` on the same row — otherwise this
module raises `ExpectError`. This stops a fixture from silently ignoring a stat
the API returns (e.g. asserting `value`/`median` but forgetting `p25`/`p75`).

The rule is self-scoping: a stat field ABSENT from the row is never required, so
a row that legitimately returns only `value`/`median`/`range_*` passes without a
`p25`/`p75` assertion. Empty-window cases (a `find` that matches 0 rows binds no
`it`) are naturally exempt. The gate runs AFTER all explicit rules pass, so the
existing first-explicit-failure semantics are preserved.
"""

from __future__ import annotations

import math
import re

from typing import Any

import celpy


# Stat fields the bullet/batch endpoint returns per row. `median` is the 50th
# percentile (the API returns no `p50`). Every one of these that a matched row
# carries must be asserted within its case — see the no-unasserted-stat rule.
_STAT_FIELDS = ("value", "median", "range_min", "range_max", "p25", "p75")


def _stat_refs_in_cel(expr: str) -> set[str]:
    """Stat fields READ off the matched row (`it.<field>` or `it["<field>"]`) in a
    CEL `assert`. Scoped to actual `it` access so a stat NAME that merely appears
    in a string literal or on another object (e.g. `it.metric_key == "p75"`) does
    NOT count as asserting that stat — otherwise a fixture could bypass the
    no-unasserted-stat gate by mentioning the name. `_STAT_FIELDS` are literals,
    but `re.escape` keeps the pattern hardcoded-safe regardless."""
    refs: set[str] = set()
    for f in _STAT_FIELDS:
        name = re.escape(f)
        if re.search(rf"\bit\.{name}\b", expr) or re.search(rf'\bit\[["\']{name}["\']\]', expr):
            refs.add(f)
    return refs


def _values_equal(got: Any, exp: Any) -> bool:
    """Equality for an `equal:` field.

    Two numbers compare with a tolerance: the API rounds floats to 4 decimals,
    so a fractional stat (a ratio, an average) must not be rejected by binary
    float drift. `abs_tol` (1e-6) sits far below the API's 4dp quantum (1e-4), so
    a tolerant pass never masks a real mismatch — two distinct served values
    differ by at least 1e-4. Non-numbers (str / None / bool) compare exactly;
    bool is excluded from the numeric path so `True`/`1` do not cross-match.
    """
    if (
        isinstance(got, (int, float))
        and isinstance(exp, (int, float))
        and not isinstance(got, bool)
        and not isinstance(exp, bool)
    ):
        return math.isclose(got, exp, rel_tol=1e-9, abs_tol=1e-6)
    return got == exp


class ExpectError(AssertionError):
    """A failing expect rule. Message names the case, rule and the mismatch."""


# ---------------------------------------------------------------------------
# find — exact field equality
# ---------------------------------------------------------------------------

def _find(items: list[dict], selector: dict) -> list[dict]:
    """Rows whose every selected field equals the given value (exact match)."""
    return [it for it in items if all(it.get(f) == v for f, v in selector.items())]


# ---------------------------------------------------------------------------
# CEL
# ---------------------------------------------------------------------------

_CEL_ENV = celpy.Environment()


def _eval_cel(expr: str, bindings: dict) -> bool:
    ast = _CEL_ENV.compile(expr)
    prog = _CEL_ENV.program(ast)
    activation = {k: celpy.json_to_cel(v) for k, v in bindings.items()}
    result = prog.evaluate(activation)
    return bool(result)


# ---------------------------------------------------------------------------
# Rule evaluation
# ---------------------------------------------------------------------------

def _select_result(rule: dict, results: list[dict], where: str) -> dict | None:
    if "in" in rule:
        wanted = rule["in"]
        for r in results:
            if r.get("id") == wanted:
                return r
        raise ExpectError(f"{where}: no batch result with id '{wanted}' (have {[r.get('id') for r in results]})")
    if len(results) == 1:
        return results[0]
    return None


def evaluate_case(case: dict, batch: dict, http_status: int) -> None:
    """Run every rule of `case`. Raise ExpectError on the first failure."""
    name = case.get("name", "<unnamed>")
    results = batch.get("results", []) if isinstance(batch, dict) else []

    # Per-case ledger of every `find`-matched row and the stat fields asserted on
    # it, keyed by row identity. Drives the no-unasserted-stat gate after the loop.
    checked: dict[int, dict] = {}

    for i, rule in enumerate(case.get("expect", [])):
        where = f"case '{name}' rule #{i}"
        result = _select_result(rule, results, where)
        items = result.get("items", []) if result else []

        it = None
        if "find" in rule:
            matches = _find(items, rule["find"])
            if len(matches) != 1:
                raise ExpectError(
                    f"{where}: find {rule['find']} matched {len(matches)} rows (expected exactly 1)"
                )
            it = matches[0]

        if it is not None:
            entry = checked.setdefault(
                id(it),
                {"row": it, "find": rule["find"], "where": where, "asserted": set()},
            )

        if "equal" in rule:
            if it is None:
                raise ExpectError(f"{where}: `equal` requires a `find` that selects one row")
            entry["asserted"] |= {k for k in rule["equal"] if k in _STAT_FIELDS}
            for field, exp in rule["equal"].items():
                got = it.get(field)
                if not _values_equal(got, exp):
                    raise ExpectError(f"{where}: {field}: expected {exp!r}, got {got!r}")
        elif "assert" in rule:
            # CANONICAL source of the CEL `assert` bindings (documented in the
            # yaml-rig FEATURE, DESIGN expect-engine component, README, and the
            # /metric-test skill). `it` is None unless this rule had a `find`.
            bindings = {
                "it": it,
                "items": items,
                "result": result,
                "results": results,
                "status": http_status,
            }
            if it is not None:
                entry["asserted"] |= _stat_refs_in_cel(rule["assert"])
            try:
                ok = _eval_cel(rule["assert"], bindings)
            except Exception as e:  # noqa: BLE001 - surface CEL errors as rule failures
                raise ExpectError(f"{where}: CEL error in {rule['assert']!r}: {e}") from e
            if not ok:
                raise ExpectError(f"{where}: assert failed: {rule['assert']}")
        else:
            raise ExpectError(f"{where}: rule must have `equal` or `assert`")

    # No-unasserted-stat gate (runs after every explicit rule passed): every stat
    # field a matched row CARRIES must have been asserted somewhere in this case.
    for e in checked.values():
        present = {f for f in _STAT_FIELDS if f in e["row"]}
        missing = present - e["asserted"]
        if missing:
            raise ExpectError(
                f"{e['where']}: row for find {e['find']} returns {sorted(missing)} "
                f"but no expect rule asserts them — every bullet stat the API "
                f"returns must be checked (add to `equal` or a CEL `assert`)."
            )

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
"""

from __future__ import annotations

from typing import Any

import celpy


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

        if "equal" in rule:
            if it is None:
                raise ExpectError(f"{where}: `equal` requires a `find` that selects one row")
            for field, exp in rule["equal"].items():
                got = it.get(field)
                if got != exp:
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
            try:
                ok = _eval_cel(rule["assert"], bindings)
            except Exception as e:  # noqa: BLE001 - surface CEL errors as rule failures
                raise ExpectError(f"{where}: CEL error in {rule['assert']!r}: {e}") from e
            if not ok:
                raise ExpectError(f"{where}: assert failed: {rule['assert']}")
        else:
            raise ExpectError(f"{where}: rule must have `equal` or `assert`")

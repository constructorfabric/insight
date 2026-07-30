from __future__ import annotations

import math
import re
from functools import cache
from typing import Any

import celpy


class ExpectError(AssertionError):
    pass


_CEL_ENV = celpy.Environment()
_VIEW_ITEMS = {
    "period": "values",
    "peer": "values",
    "timeseries": "series",
    "breakdown": "values",
    "histogram": "values",
}
_REQUIRED_VIEW_FIELDS = {
    "period": {"value"},
    "peer": {"target_value", "p25", "median", "p75", "min", "max", "n"},
    "timeseries": {"points"},
    "breakdown": {"value"},
    "histogram": {"bins"},
}


@cache
def _cel_program(expr: str) -> Any:
    ast = _CEL_ENV.compile(expr)
    return _CEL_ENV.program(ast)


def _eval_cel(expr: str, bindings: dict[str, Any]) -> bool:
    activation = {
        key: celpy.json_to_cel(value)
        for key, value in bindings.items()
        if re.search(rf"\b{re.escape(key)}\b", expr)
    }
    return bool(_cel_program(expr).evaluate(activation))


def _values_equal(got: Any, expected: Any) -> bool:
    if (
        isinstance(got, (int, float))
        and isinstance(expected, (int, float))
        and not isinstance(got, bool)
        and not isinstance(expected, bool)
    ):
        return math.isclose(got, expected, rel_tol=1e-9, abs_tol=1e-6)
    return got == expected


def _matches(value: Any, selector: Any) -> bool:
    if isinstance(selector, dict):
        if isinstance(value, dict):
            return all(key in value and _matches(value[key], expected) for key, expected in selector.items())
        if isinstance(value, list):
            return any(_matches(item, selector) for item in value)
        return False
    return _values_equal(value, selector)


def _select_one(rows: list[dict[str, Any]], selector: dict[str, Any], where: str) -> dict[str, Any]:
    matches = [row for row in rows if _matches(row, selector)]
    if len(matches) != 1:
        raise ExpectError(f"{where}: find {selector} matched {len(matches)} rows (expected exactly 1)")
    return matches[0]


def _select_metric(
    rule: dict[str, Any], metrics: list[dict[str, Any]], where: str
) -> dict[str, Any] | None:
    metric_key = rule.get("metric")
    if metric_key is None:
        return None
    matches = [metric for metric in metrics if metric.get("metric_key") == metric_key]
    if len(matches) != 1:
        raise ExpectError(
            f"{where}: metric {metric_key!r} matched {len(matches)} metrics (expected exactly 1)"
        )
    return matches[0]


def _select_view(
    rule: dict[str, Any], metric: dict[str, Any] | None, where: str
) -> dict[str, Any] | None:
    view_kind = rule.get("view")
    if view_kind is None:
        return None
    if metric is None:
        raise ExpectError(f"{where}: `view` requires `metric`")
    matches = [view for view in metric.get("views", []) if view.get("view") == view_kind]
    if len(matches) != 1:
        raise ExpectError(
            f"{where}: view {view_kind!r} matched {len(matches)} views (expected exactly 1)"
        )
    return matches[0]


def _asserted_fields(rule: dict[str, Any]) -> set[str]:
    fields = set((rule.get("equal") or {}).keys())
    fields.update((rule.get("contains") or {}).keys())
    fields.update(rule.get("nonempty") or [])
    expression = rule.get("assert")
    if expression:
        fields.update(re.findall(r"\bit\.([a-zA-Z_][a-zA-Z0-9_]*)\b", expression))
        fields.update(re.findall(r"\bit\[['\"]([a-zA-Z_][a-zA-Z0-9_]*)['\"]\]", expression))
    return fields


def evaluate_case(case: dict[str, Any], response: Any, http_status: int) -> None:
    name = case.get("name", "<unnamed>")
    metrics = response.get("metrics", []) if isinstance(response, dict) else []
    checked: dict[tuple[str, int], set[str]] = {}
    rows: dict[tuple[str, int], dict[str, Any]] = {}

    for index, rule in enumerate(case.get("expect", [])):
        where = f"case '{name}' rule #{index}"
        metric = _select_metric(rule, metrics, where)
        view = _select_view(rule, metric, where)
        view_kind = view.get("view") if view else None
        items = view.get(_VIEW_ITEMS[view_kind], []) if view_kind else []
        item = _select_one(items, rule["find"], where) if "find" in rule else None
        target = item if item is not None else view if view is not None else metric

        if item is not None and view_kind is not None:
            identity = (view_kind, id(item))
            rows[identity] = item
            checked.setdefault(identity, set()).update(_asserted_fields(rule))

        if "equal" in rule:
            if target is None:
                raise ExpectError(f"{where}: `equal` requires `metric`, `view`, or `find`")
            for field, expected in rule["equal"].items():
                if field not in target:
                    raise ExpectError(f"{where}: {field}: field is missing")
                got = target.get(field)
                if not _values_equal(got, expected):
                    raise ExpectError(f"{where}: {field}: expected {expected!r}, got {got!r}")
        elif "contains" in rule:
            if target is None:
                raise ExpectError(f"{where}: `contains` requires `metric`, `view`, or `find`")
            for field, selector in rule["contains"].items():
                values = target.get(field)
                if not isinstance(values, list) or not any(_matches(value, selector) for value in values):
                    raise ExpectError(f"{where}: {field} contains no match for {selector!r}")
        elif "nonempty" in rule:
            if target is None:
                raise ExpectError(f"{where}: `nonempty` requires `metric`, `view`, or `find`")
            for field in rule["nonempty"]:
                if not target.get(field):
                    raise ExpectError(f"{where}: {field} is empty")
        elif "assert" in rule:
            bindings = {
                "it": item,
                "items": items,
                "view": view,
                "metric": metric,
                "metrics": metrics,
                "status": http_status,
            }
            try:
                passed = _eval_cel(rule["assert"], bindings)
            except Exception as error:
                raise ExpectError(f"{where}: CEL error in {rule['assert']!r}: {error}") from error
            if not passed:
                raise ExpectError(f"{where}: assert failed: {rule['assert']}")
        else:
            raise ExpectError(f"{where}: rule must have `equal`, `contains`, `nonempty`, or `assert`")

    for identity, asserted in checked.items():
        view_kind, _ = identity
        required = _REQUIRED_VIEW_FIELDS[view_kind]
        missing = required - asserted
        if missing:
            raise ExpectError(
                f"case '{name}': {view_kind} row leaves {sorted(missing)} unasserted"
            )

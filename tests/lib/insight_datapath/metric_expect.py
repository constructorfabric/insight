"""Assertions over a metric-results response, and the ledger the coverage gate reads.

The selection rules are the ones the YAML engine enforced: exactly one metric per key,
exactly one view per kind, exactly one row per selector; numbers compare within
rel 1e-9 / abs 1e-6; and every row a test touches must have its view's required
fields asserted before the case ends, so a case cannot read `value` off a peer row and
leave `n` unexamined.
"""

from __future__ import annotations

import json
import math
from collections.abc import Callable, Iterable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

VIEW_ITEMS: dict[str, str] = {
    "period": "values",
    "peer": "values",
    "timeseries": "series",
    "breakdown": "values",
    "rollup": "values",
    "histogram": "values",
}

REQUIRED_VIEW_FIELDS: dict[str, frozenset[str]] = {
    "period": frozenset({"value"}),
    "peer": frozenset({"target_value", "p25", "median", "p75", "min", "max", "n"}),
    "timeseries": frozenset({"points"}),
    "breakdown": frozenset({"value"}),
    "rollup": frozenset({"value", "contributing_entity_count"}),
    "histogram": frozenset({"bins"}),
}


class ExpectError(AssertionError):
    """A selection or an expectation over the response did not hold."""


def values_equal(got: Any, expected: Any) -> bool:
    numeric = (int, float)
    if (
        isinstance(got, numeric)
        and isinstance(expected, numeric)
        and not isinstance(got, bool)
        and not isinstance(expected, bool)
    ):
        return math.isclose(got, expected, rel_tol=1e-9, abs_tol=1e-6)
    return bool(got == expected)


def matches(value: Any, selector: Any) -> bool:
    if isinstance(selector, dict):
        if isinstance(value, dict):
            return all(
                key in value and matches(value[key], expected) for key, expected in selector.items()
            )
        if isinstance(value, list):
            return any(matches(item, selector) for item in value)
        return False
    return values_equal(value, selector)


def some(items: Iterable[Any], **selector: Any) -> list[dict[str, Any]]:
    """The entries of a view or a point list that match `selector`; a Row counts by its fields."""
    entries = [item.fields if isinstance(item, Row) else item for item in items]
    return [entry for entry in entries if matches(entry, selector)]


def one(items: Iterable[Any], **selector: Any) -> dict[str, Any]:
    """The single entry matching `selector`; a nested list matches when any element does."""
    found = some(items, **selector)
    if len(found) != 1:
        raise ExpectError(f"find {selector} matched {len(found)} entries (expected exactly 1)")
    return found[0]


@dataclass
class Ledger:
    """Which metric views the suite asserted, and which it requested — the gate's input."""

    asserted: dict[str, dict[str, set[str]]] = field(default_factory=dict)
    requested: set[str] = field(default_factory=set)

    def record_request(self, body: dict[str, Any]) -> None:
        for metric in body.get("metrics") or []:
            key = metric.get("metric_key")
            if key:
                self.requested.add(str(key))

    def record_assertion(self, metric_key: str, view: str, test_name: str) -> None:
        self.asserted.setdefault(metric_key, {}).setdefault(view, set()).add(test_name)

    def write(self, path: Path) -> None:
        document = {
            "asserted": {
                key: {view: sorted(names) for view, names in views.items()}
                for key, views in self.asserted.items()
            },
            "requested": sorted(self.requested),
        }
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    @staticmethod
    def read(path: Path) -> tuple[dict[str, dict[str, set[str]]], set[str]]:
        document = json.loads(path.read_text(encoding="utf-8"))
        asserted = {
            key: {view: set(names) for view, names in views.items()}
            for key, views in document.get("asserted", {}).items()
        }
        return asserted, set(document.get("requested", []))


@dataclass(frozen=True)
class Row:
    """One selected row, remembering which of its fields the test has asserted."""

    fields: dict[str, Any]
    where: str
    asserted: set[str]

    def __getitem__(self, key: str) -> Any:
        return self.fields[key]

    def equals(self, **expected: Any) -> Row:
        for name, value in expected.items():
            if name not in self.fields:
                raise ExpectError(f"{self.where}: {name}: field is missing")
            got = self.fields[name]
            if not values_equal(got, value):
                raise ExpectError(f"{self.where}: {name}: expected {value!r}, got {got!r}")
            self.asserted.add(name)
        return self

    def contains(self, **selectors: Any) -> Row:
        for name, selector in selectors.items():
            values = self.fields.get(name)
            if not isinstance(values, list) or not any(
                matches(value, selector) for value in values
            ):
                raise ExpectError(f"{self.where}: {name} contains no match for {selector!r}")
            self.asserted.add(name)
        return self

    def check(self, name: str, predicate: Callable[[Any], bool], describe: str = "") -> Row:
        """Assert `predicate` over one field; counts as examining it, like `equals`."""
        if name not in self.fields:
            raise ExpectError(f"{self.where}: {name}: field is missing")
        got = self.fields[name]
        if not predicate(got):
            raise ExpectError(
                f"{self.where}: {name}: {describe or 'predicate'} failed, got {got!r}"
            )
        self.asserted.add(name)
        return self

    def nonempty(self, *names: str) -> Row:
        for name in names:
            if not self.fields.get(name):
                raise ExpectError(f"{self.where}: {name} is empty")
            self.asserted.add(name)
        return self


class MetricResponse:
    """A metric-results response as a test reads it, with every selection exact."""

    def __init__(self, status: int, payload: Any, *, test_name: str, ledger: Ledger) -> None:
        self.status = status
        self.payload = payload
        self._test_name = test_name
        self._ledger = ledger
        self._touched: dict[tuple[str, str, int], Row] = {}

    @property
    def metrics(self) -> list[dict[str, Any]]:
        return list(self.payload.get("metrics", [])) if isinstance(self.payload, dict) else []

    def metric(self, key: str) -> dict[str, Any]:
        found = [metric for metric in self.metrics if metric.get("metric_key") == key]
        if len(found) != 1:
            raise ExpectError(
                f"{self._test_name}: metric {key!r} matched {len(found)} metrics (expected exactly 1)"
            )
        return found[0]

    def view(self, key: str, kind: str) -> dict[str, Any]:
        found = [view for view in self.metric(key).get("views", []) if view.get("view") == kind]
        if len(found) != 1:
            raise ExpectError(
                f"{self._test_name}: {key} view {kind!r} matched {len(found)} views (expected exactly 1)"
            )
        return found[0]

    def items(self, key: str, kind: str) -> list[dict[str, Any]]:
        return list(self.view(key, kind).get(VIEW_ITEMS[kind], []))

    def row(self, key: str, kind: str, **selector: Any) -> Row:
        """The one row of `key`'s `kind` view that matches `selector`."""
        items = self.items(key, kind)
        found = [index for index, item in enumerate(items) if matches(item, selector)]
        if len(found) != 1:
            raise ExpectError(
                f"{self._test_name}: {key}/{kind}: find {selector} matched {len(found)} rows (expected exactly 1)"
            )
        identity = (key, kind, found[0])
        if identity not in self._touched:
            where = f"{self._test_name}: {key}/{kind} {selector}"
            self._touched[identity] = Row(fields=items[found[0]], where=where, asserted=set())
        self._ledger.record_assertion(key, kind, self._test_name)
        return self._touched[identity]

    def series(self, key: str) -> list[dict[str, Any]]:
        """The timeseries entries of `key`, counted as an assertion over their points."""
        return self._whole_view(key, "timeseries")

    def breakdown(self, key: str) -> list[dict[str, Any]]:
        """The breakdown entries of `key`, counted as an assertion over dimensions and value."""
        return self._whole_view(key, "breakdown")

    def histogram(self, key: str) -> list[dict[str, Any]]:
        """The histogram entries of `key`, counted as an assertion over their bins."""
        return self._whole_view(key, "histogram")

    def rows(self, key: str, kind: str) -> list[Row]:
        """Every row of `key`'s `kind` view, counted as an assertion over the view.

        For a rule over the whole list of a period, peer or rollup view; unlike `row`,
        these rows are not held to the completeness check.
        """
        items = self._whole_view(key, kind)
        return [
            Row(fields=item, where=f"{self._test_name}: {key}/{kind} [{index}]", asserted=set())
            for index, item in enumerate(items)
        ]

    def _whole_view(self, key: str, kind: str) -> list[dict[str, Any]]:
        self._ledger.record_assertion(key, kind, self._test_name)
        return self.items(key, kind)

    def check_complete(self) -> None:
        """Every row this test selected has its view's required fields asserted."""
        for (key, kind, _), row in self._touched.items():
            missing = REQUIRED_VIEW_FIELDS[kind] - row.asserted
            if missing:
                raise ExpectError(
                    f"{self._test_name}: {key}/{kind} row leaves {sorted(missing)} unasserted"
                )

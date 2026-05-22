"""Pandas-based cell-precise diff between API response items and expected CSV.

Per the FEATURE.md DoD, on failure the asserter MUST surface the first 20
mismatched cells as `(key, column, expected, actual)` lines visible in the
pytest captured stdout. We build that string ourselves so the output is
copy-paste-friendly and stable across pandas versions.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

import pandas as pd

from e2e_lib.analytics_api import ApiResponse
from e2e_lib.fixture_loader import Fixture, SpecYaml


MAX_DIFF_ROWS = 20


class AssertionFailure(AssertionError):
    """Raised by assert_matches() on diff. Inherits AssertionError so pytest treats it natively."""


@dataclass(frozen=True)
class CellDiff:
    """One row of the cell-precise diff output."""

    key: dict
    column: str
    expected: object
    actual: object

    def render(self) -> str:
        return f"  key={self._render_key()}  column={self.column!r}  expected={self.expected!r}  actual={self.actual!r}"

    def _render_key(self) -> str:
        if not self.key:
            return "(no key)"
        return "{" + ", ".join(f"{k}={v!r}" for k, v in self.key.items()) + "}"


def assert_matches(response: ApiResponse, fixture: Fixture) -> None:
    """Compare `response.items` with `fixture.expected_df`. Raise on mismatch.

    Steps (matching FEATURE.md §3 "Execute Test" algorithm):

      1. HTTP status must be 200
      2. column sets must match exactly
      3. row counts must match (after sorting)
      4. cell-by-cell compare with float tolerance per spec
      5. on any mismatch, render the first 20 mismatched cells
    """
    if response.status_code != 200:
        raise AssertionFailure(
            f"HTTP {response.status_code} != 200\n  body: {response.raw!r}"
        )

    actual_df = pd.DataFrame(response.items)
    expected_df = fixture.expected_df.copy()

    _check_column_sets(actual_df, expected_df)
    actual_sorted, expected_sorted = _sort_by_key(actual_df, expected_df, fixture.spec.key_columns)
    _check_row_counts(actual_sorted, expected_sorted, fixture.spec.key_columns)
    diffs = _cell_diff(actual_sorted, expected_sorted, fixture.spec)

    if diffs:
        msg = _format_diff_message(diffs, expected_sorted, actual_sorted)
        raise AssertionFailure(msg)


def update_snapshot(response: ApiResponse, fixture: Fixture) -> str:
    """Write `response.items` to `fixture.root / "expected/response.csv"`.

    Returns a git-style summary string (logged by the test runner) showing
    which cells changed vs the previous expected CSV. Empty string when the
    new snapshot is byte-equivalent to the old.

    Gated behind the `--update-snapshots` CLI flag (provided by
    `feature-snapshot-update`); calling it directly is only intended for
    fixture-authoring tooling.
    """
    target = fixture.root / "expected" / "response.csv"
    target.parent.mkdir(parents=True, exist_ok=True)
    df = pd.DataFrame(response.items)
    # Preserve expected column order if it exists (stable diffs across PRs)
    if not fixture.expected_df.empty:
        ordered = [c for c in fixture.expected_df.columns if c in df.columns]
        extras = [c for c in df.columns if c not in ordered]
        df = df[ordered + extras]

    summary = _snapshot_diff_summary(
        before=fixture.expected_df,
        after=df,
        key_cols=fixture.spec.key_columns,
    )
    df.to_csv(target, index=False)
    return summary


def _snapshot_diff_summary(
    *, before: pd.DataFrame, after: pd.DataFrame, key_cols: list[str], max_lines: int = 30
) -> str:
    """Produce a `+row` / `-row` / `~cell` summary suitable for the pytest report."""
    if before.empty and after.empty:
        return "(no rows)"
    if before.empty:
        return f"+ added {len(after)} row(s)"
    if after.empty:
        return f"- removed {len(before)} row(s)"

    valid_keys = [k for k in key_cols if k in before.columns and k in after.columns]
    if not valid_keys:
        # Fall back to row-count delta
        delta = len(after) - len(before)
        return f"{'+' if delta >= 0 else '-'} row count {len(before)} -> {len(after)} ({delta:+d})"

    before_idx = before.set_index(valid_keys)
    after_idx = after.set_index(valid_keys)

    added = after_idx.index.difference(before_idx.index)
    removed = before_idx.index.difference(after_idx.index)
    common = after_idx.index.intersection(before_idx.index)

    lines: list[str] = []
    for k in added.tolist()[: max_lines // 3]:
        lines.append(f"+ {k}")
    for k in removed.tolist()[: max_lines // 3]:
        lines.append(f"- {k}")
    for k in common.tolist():
        b_row = before_idx.loc[k]
        a_row = after_idx.loc[k]
        for col in a_row.index:
            if col not in b_row.index:
                continue
            b_val = b_row[col]
            a_val = a_row[col]
            if _cells_equal(b_val, a_val, is_numeric=pd.api.types.is_numeric_dtype(type(a_val)), tolerance=0):
                continue
            lines.append(f"~ {k}  {col}: {b_val!r} -> {a_val!r}")
            if len(lines) >= max_lines:
                lines.append(f"  (truncated at {max_lines} changes)")
                return "\n".join(lines)
    if not lines:
        return "(no changes)"
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# internals
# ---------------------------------------------------------------------------


def _check_column_sets(actual: pd.DataFrame, expected: pd.DataFrame) -> None:
    actual_cols = set(actual.columns)
    expected_cols = set(expected.columns)
    if actual_cols != expected_cols:
        missing = expected_cols - actual_cols
        extra = actual_cols - expected_cols
        parts = []
        if missing:
            parts.append(f"missing in response: {sorted(missing)}")
        if extra:
            parts.append(f"extra in response: {sorted(extra)}")
        raise AssertionFailure("column set mismatch — " + "; ".join(parts))


def _sort_by_key(
    actual: pd.DataFrame, expected: pd.DataFrame, key_cols: list[str]
) -> tuple[pd.DataFrame, pd.DataFrame]:
    actual_sorted = (
        actual.sort_values(key_cols).reset_index(drop=True)[expected.columns.tolist()]
    )
    expected_sorted = expected.sort_values(key_cols).reset_index(drop=True)
    return actual_sorted, expected_sorted


def _check_row_counts(actual: pd.DataFrame, expected: pd.DataFrame, key_cols: list[str]) -> None:
    if len(actual) == len(expected):
        return
    # Identify which keys are missing / extra to make the message actionable
    actual_keys = set(map(tuple, actual[key_cols].astype(str).itertuples(index=False, name=None)))
    expected_keys = set(map(tuple, expected[key_cols].astype(str).itertuples(index=False, name=None)))
    missing = expected_keys - actual_keys
    extra = actual_keys - expected_keys
    parts = [f"row count mismatch: expected={len(expected)}, actual={len(actual)}"]
    if missing:
        parts.append(f"missing keys: {sorted(missing)[:10]}")
    if extra:
        parts.append(f"extra keys: {sorted(extra)[:10]}")
    raise AssertionFailure("\n  ".join(parts))


def _cell_diff(actual: pd.DataFrame, expected: pd.DataFrame, spec: SpecYaml) -> list[CellDiff]:
    diffs: list[CellDiff] = []
    tolerance = spec.float_tolerance
    key_cols = spec.key_columns
    cols = list(expected.columns)

    # Pre-classify columns so we don't reflect numpy types row by row
    numeric_cols = {c for c in cols if pd.api.types.is_numeric_dtype(expected[c])}

    for i in range(len(expected)):
        key = {k: expected.iloc[i][k] for k in key_cols}
        for col in cols:
            e_val = expected.iloc[i][col]
            a_val = actual.iloc[i][col]
            if _cells_equal(e_val, a_val, is_numeric=(col in numeric_cols), tolerance=tolerance):
                continue
            diffs.append(CellDiff(key=key, column=col, expected=e_val, actual=a_val))
            if len(diffs) >= MAX_DIFF_ROWS:
                return diffs
    return diffs


def _cells_equal(expected, actual, *, is_numeric: bool, tolerance: float) -> bool:
    e_na = _is_na(expected)
    a_na = _is_na(actual)
    if e_na and a_na:
        return True
    if e_na or a_na:
        return False
    if is_numeric:
        try:
            return math.isclose(float(expected), float(actual), abs_tol=tolerance, rel_tol=0.0)
        except (TypeError, ValueError):
            return expected == actual
    return expected == actual


def _is_na(value) -> bool:
    try:
        return bool(pd.isna(value))
    except (TypeError, ValueError):
        return False


def _format_diff_message(diffs: list[CellDiff], expected: pd.DataFrame, actual: pd.DataFrame) -> str:
    lines = [
        f"CSV diff: {len(diffs)} mismatched cell(s)"
        + (f" (showing first {MAX_DIFF_ROWS})" if len(diffs) >= MAX_DIFF_ROWS else ""),
        f"  expected rows: {len(expected)}  |  actual rows: {len(actual)}",
        "",
    ]
    for d in diffs:
        lines.append(d.render())
    return "\n".join(lines)

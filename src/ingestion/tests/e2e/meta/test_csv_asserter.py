"""Unit tests for csv-asserter. No compose / no CH / no analytics-api."""

from __future__ import annotations

from pathlib import Path

import pandas as pd
import pytest

from e2e_lib.analytics_api import ApiResponse
from e2e_lib.csv_asserter import (
    AssertionFailure,
    CellDiff,
    MAX_DIFF_ROWS,
    assert_matches,
    update_snapshot,
)
from e2e_lib.fixture_loader import Fixture, SpecYaml


pytestmark = pytest.mark.smoke


# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------


def _spec(
    *,
    key_columns=("person_id",),
    float_tolerance: float = 1e-6,
    metric_id: str = "00000000-0000-0000-0000-000000000001",
) -> SpecYaml:
    return SpecYaml(
        spec_version=1,
        endpoint="/v1/metrics/{metric_id}/query",
        request_body={"$top": 50},
        dbt_selector="+silver_people+",
        key_columns=list(key_columns),
        method="POST",
        metric_id=metric_id,
        float_tolerance=float_tolerance,
    )


def _fixture(
    tmp_path: Path,
    expected: pd.DataFrame,
    *,
    spec: SpecYaml | None = None,
) -> Fixture:
    return Fixture(
        name="dummy",
        root=tmp_path,
        spec=spec or _spec(),
        bronze_csvs=[],
        expected_df=expected,
    )


def _ok_response(items: list[dict]) -> ApiResponse:
    return ApiResponse(status_code=200, items=items, page_info={}, raw={"items": items, "page_info": {}})


# ---------------------------------------------------------------------------
# Happy path
# ---------------------------------------------------------------------------


def test_exact_match_passes(tmp_path: Path) -> None:
    expected = pd.DataFrame(
        [
            {"person_id": "alice", "display_name": "Alice", "score": 1.0},
            {"person_id": "bob", "display_name": "Bob", "score": 2.0},
        ]
    )
    fx = _fixture(tmp_path, expected)
    resp = _ok_response(
        [
            {"person_id": "alice", "display_name": "Alice", "score": 1.0},
            {"person_id": "bob", "display_name": "Bob", "score": 2.0},
        ]
    )
    assert_matches(resp, fx)  # no raise


def test_row_order_does_not_matter(tmp_path: Path) -> None:
    expected = pd.DataFrame(
        [
            {"person_id": "alice", "score": 1},
            {"person_id": "bob", "score": 2},
        ]
    )
    fx = _fixture(tmp_path, expected)
    # API returns rows in reverse order — sort-by-key makes it equivalent
    resp = _ok_response(
        [
            {"person_id": "bob", "score": 2},
            {"person_id": "alice", "score": 1},
        ]
    )
    assert_matches(resp, fx)


def test_float_tolerance(tmp_path: Path) -> None:
    expected = pd.DataFrame([{"person_id": "alice", "rate": 1.0000001}])
    fx = _fixture(tmp_path, expected, spec=_spec(float_tolerance=1e-3))
    resp = _ok_response([{"person_id": "alice", "rate": 1.000001}])
    assert_matches(resp, fx)  # within 1e-3


def test_both_nan_is_equal(tmp_path: Path) -> None:
    expected = pd.DataFrame([{"person_id": "alice", "score": float("nan")}])
    fx = _fixture(tmp_path, expected)
    resp = _ok_response([{"person_id": "alice", "score": None}])
    assert_matches(resp, fx)


def test_multi_column_key(tmp_path: Path) -> None:
    expected = pd.DataFrame(
        [
            {"person_id": "a", "day": "2026-01-01", "v": 1},
            {"person_id": "a", "day": "2026-01-02", "v": 2},
        ]
    )
    fx = _fixture(tmp_path, expected, spec=_spec(key_columns=("person_id", "day")))
    resp = _ok_response(
        [
            {"person_id": "a", "day": "2026-01-02", "v": 2},
            {"person_id": "a", "day": "2026-01-01", "v": 1},
        ]
    )
    assert_matches(resp, fx)


# ---------------------------------------------------------------------------
# Failure paths
# ---------------------------------------------------------------------------


def test_bad_http_status_fails(tmp_path: Path) -> None:
    fx = _fixture(tmp_path, pd.DataFrame([{"person_id": "a"}]))
    resp = ApiResponse(status_code=404, items=[], page_info={}, raw={"detail": "nope"})
    with pytest.raises(AssertionFailure, match="HTTP 404"):
        assert_matches(resp, fx)


def test_column_set_mismatch_extra(tmp_path: Path) -> None:
    fx = _fixture(tmp_path, pd.DataFrame([{"person_id": "a"}]))
    resp = _ok_response([{"person_id": "a", "leaked_column": "boom"}])
    with pytest.raises(AssertionFailure, match="extra in response.*leaked_column"):
        assert_matches(resp, fx)


def test_column_set_mismatch_missing(tmp_path: Path) -> None:
    fx = _fixture(tmp_path, pd.DataFrame([{"person_id": "a", "expected_col": 1}]))
    resp = _ok_response([{"person_id": "a"}])
    with pytest.raises(AssertionFailure, match="missing in response.*expected_col"):
        assert_matches(resp, fx)


def test_row_count_mismatch(tmp_path: Path) -> None:
    expected = pd.DataFrame(
        [
            {"person_id": "a", "v": 1},
            {"person_id": "b", "v": 2},
        ]
    )
    fx = _fixture(tmp_path, expected)
    resp = _ok_response([{"person_id": "a", "v": 1}])
    with pytest.raises(AssertionFailure, match="row count mismatch"):
        assert_matches(resp, fx)


def test_cell_mismatch_renders_diff(tmp_path: Path) -> None:
    expected = pd.DataFrame(
        [
            {"person_id": "alice", "display_name": "Alice", "score": 1.0},
            {"person_id": "bob", "display_name": "Bob", "score": 2.0},
        ]
    )
    fx = _fixture(tmp_path, expected)
    resp = _ok_response(
        [
            {"person_id": "alice", "display_name": "Alice", "score": 1.0},
            {"person_id": "bob", "display_name": "Robert", "score": 5.0},  # 2 wrong cells
        ]
    )
    with pytest.raises(AssertionFailure) as exc:
        assert_matches(resp, fx)
    msg = str(exc.value)
    assert "2 mismatched cell" in msg
    assert "person_id='bob'" in msg
    assert "'display_name'" in msg
    assert "expected='Bob'" in msg
    assert "actual='Robert'" in msg
    assert "'score'" in msg


def test_diff_capped_at_max(tmp_path: Path) -> None:
    n = MAX_DIFF_ROWS + 5
    expected = pd.DataFrame([{"person_id": str(i), "v": 1} for i in range(n)])
    fx = _fixture(tmp_path, expected)
    resp = _ok_response([{"person_id": str(i), "v": 2} for i in range(n)])
    with pytest.raises(AssertionFailure) as exc:
        assert_matches(resp, fx)
    msg = str(exc.value)
    assert f"showing first {MAX_DIFF_ROWS}" in msg
    # Body has at most MAX_DIFF_ROWS diff lines
    diff_lines = [l for l in msg.splitlines() if "expected=" in l]
    assert len(diff_lines) == MAX_DIFF_ROWS


# ---------------------------------------------------------------------------
# Snapshot update
# ---------------------------------------------------------------------------


def test_update_snapshot_writes_csv(tmp_path: Path) -> None:
    expected = pd.DataFrame([{"person_id": "alice", "score": 1.0}])
    fx = _fixture(tmp_path, expected)
    resp = _ok_response([{"person_id": "alice", "score": 7.0}])
    summary = update_snapshot(resp, fx)

    target = tmp_path / "expected" / "response.csv"
    assert target.exists()
    df = pd.read_csv(target)
    assert list(df.columns) == ["person_id", "score"]
    assert df.iloc[0]["score"] == 7.0
    # Summary line MUST capture the changed cell
    assert "~ alice" in summary or "score" in summary


def test_update_snapshot_summary_added_rows(tmp_path: Path) -> None:
    before = pd.DataFrame([{"person_id": "alice", "score": 1.0}])
    fx = _fixture(tmp_path, before)
    resp = _ok_response(
        [
            {"person_id": "alice", "score": 1.0},
            {"person_id": "bob", "score": 2.0},
        ]
    )
    summary = update_snapshot(resp, fx)
    assert summary.startswith("+ ") or "bob" in summary


def test_update_snapshot_summary_removed_rows(tmp_path: Path) -> None:
    before = pd.DataFrame(
        [
            {"person_id": "alice", "score": 1.0},
            {"person_id": "bob", "score": 2.0},
        ]
    )
    fx = _fixture(tmp_path, before)
    resp = _ok_response([{"person_id": "alice", "score": 1.0}])
    summary = update_snapshot(resp, fx)
    assert "- " in summary or "bob" in summary


def test_update_snapshot_no_changes(tmp_path: Path) -> None:
    expected = pd.DataFrame([{"person_id": "alice", "score": 1.0}])
    fx = _fixture(tmp_path, expected)
    resp = _ok_response([{"person_id": "alice", "score": 1.0}])
    summary = update_snapshot(resp, fx)
    assert summary == "(no changes)"


def test_update_snapshot_preserves_column_order(tmp_path: Path) -> None:
    # expected has columns in a specific order
    before = pd.DataFrame([{"person_id": "a", "name": "Alice", "score": 1.0}])
    # API returns them in a different order
    resp_items = [{"score": 1.0, "person_id": "a", "name": "Alice"}]
    fx = _fixture(tmp_path, before)
    update_snapshot(_ok_response(resp_items), fx)

    target = tmp_path / "expected" / "response.csv"
    df = pd.read_csv(target)
    # Order matches the previous expected, not the API response
    assert list(df.columns) == ["person_id", "name", "score"]

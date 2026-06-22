from __future__ import annotations

import pytest

from source_gitlab.streams.windowing import (
    CommittedDateWindowing,
    UnwindowableWindow,
    UpdatedAtWindowing,
    _parse,
    _to_utc_z,
)

SPLIT_ITERATION_BOUND = 200


def _rolling() -> UpdatedAtWindowing:
    return UpdatedAtWindowing.__new__(UpdatedAtWindowing)


def _bisect() -> CommittedDateWindowing:
    return CommittedDateWindowing.__new__(CommittedDateWindowing)


class TestParse:
    def test_offset_timestamp_normalizes_to_utc(self) -> None:
        assert _to_utc_z(_parse("2026-06-01T12:00:00.000+03:00")) == "2026-06-01T09:00:00Z"

    def test_zulu_timestamp_parses_on_floor_python(self) -> None:
        assert _to_utc_z(_parse("2026-06-01T12:00:00Z")) == "2026-06-01T12:00:00Z"

    def test_date_only_input_normalizes_to_utc_midnight(self) -> None:
        assert _to_utc_z(_parse("2024-01-01")) == "2024-01-01T00:00:00Z"


class TestRollingWindow:
    def test_next_window_starts_one_second_before_last_record(self) -> None:
        out = _rolling()._window_split(
            {"updated_after": "2026-01-01T00:00:00Z", "updated_before": None},
            "2026-06-01T00:00:00Z",
        )
        assert out == [{"updated_after": "2026-05-31T23:59:59Z", "updated_before": None}]

    def test_open_upper_bound_is_preserved(self) -> None:
        out = _rolling()._window_split(
            {"updated_after": None, "updated_before": "2026-12-31T00:00:00Z"},
            "2026-06-01T00:00:00Z",
        )
        assert out[0]["updated_before"] == "2026-12-31T00:00:00Z"

    def test_empty_window_at_cap_is_unwindowable(self) -> None:
        with pytest.raises(UnwindowableWindow):
            _rolling()._window_split(
                {"updated_after": "2026-01-01T00:00:00Z", "updated_before": None}, None
            )

    def test_single_timestamp_flood_terminates(self) -> None:
        window = {"updated_after": None, "updated_before": None}
        for _ in range(SPLIT_ITERATION_BOUND):
            try:
                window = _rolling()._window_split(window, "2026-03-01T12:00:00+00:00")[0]
            except UnwindowableWindow:
                return
        pytest.fail("rolling split over a single-timestamp flood did not terminate")


class TestBisectWindow:
    def test_split_yields_two_contiguous_halves(self) -> None:
        out = _bisect()._window_split(
            {"since": "2020-01-01T00:00:00Z", "until": "2026-01-01T00:00:00Z"}, None
        )
        assert len(out) == 2
        assert out[0]["since"] == "2020-01-01T00:00:00Z"
        assert out[0]["until"] == out[1]["since"]
        assert out[1]["until"] == "2026-01-01T00:00:00Z"

    def test_open_upper_bound_is_preserved_on_right(self) -> None:
        out = _bisect()._window_split({"since": "2020-01-01T00:00:00Z", "until": None}, None)
        assert out[0]["until"] is not None
        assert out[1]["until"] is None

    def test_one_second_window_is_unwindowable(self) -> None:
        with pytest.raises(UnwindowableWindow):
            _bisect()._window_split(
                {"since": "2026-01-01T00:00:00Z", "until": "2026-01-01T00:00:01Z"}, None
            )

    def test_date_only_open_window_splits_without_crashing(self) -> None:
        out = _bisect()._window_split({"since": "2024-01-01", "until": None}, None)
        assert len(out) == 2
        assert out[0]["since"] == "2024-01-01T00:00:00Z"

    def test_date_only_finite_window_bisects(self) -> None:
        out = _bisect()._window_split(
            {"since": "2024-01-01", "until": "2024-12-31"}, None
        )
        assert len(out) == 2
        assert out[0]["since"] == "2024-01-01T00:00:00Z"
        assert out[0]["until"] == out[1]["since"]

    def test_open_ended_flood_terminates_via_right_recursion(self) -> None:
        window = {"since": None, "until": None}
        for _ in range(SPLIT_ITERATION_BOUND):
            try:
                window = _bisect()._window_split(window, None)[1]
            except UnwindowableWindow:
                return
        pytest.fail("bisect right-recursion did not terminate")

    def test_bounded_flood_terminates_via_left_recursion(self) -> None:
        window = {"since": "2026-01-01T00:00:00Z", "until": "2026-01-01T00:05:00Z"}
        for _ in range(SPLIT_ITERATION_BOUND):
            try:
                window = _bisect()._window_split(window, None)[0]
            except UnwindowableWindow:
                return
        pytest.fail("bisect left-recursion did not terminate")

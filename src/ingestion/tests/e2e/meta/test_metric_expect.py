"""The rules a ported spec is held to: exact selection, completeness, tolerant numbers."""

from __future__ import annotations

import pytest
from lib.metric_expect import ExpectError, Ledger, MetricResponse, one, some, values_equal

ERIN = "erin@example.com"

PAYLOAD = {
    "metrics": [
        {
            "metric_key": "m.count",
            "views": [
                {"view": "period", "values": [{"entity_id": ERIN, "value": 3}]},
                {
                    "view": "peer",
                    "values": [
                        {
                            "entity_id": ERIN,
                            "target_value": 3,
                            "p25": 1,
                            "median": 2,
                            "p75": 3,
                            "min": 1,
                            "max": 3,
                            "n": 3,
                        }
                    ],
                },
                {
                    "view": "timeseries",
                    "series": [
                        {
                            "entity_id": ERIN,
                            "points": [
                                {"bucket_start": "2026-01-01", "value": None},
                                {"bucket_start": "2026-01-02", "value": 3},
                            ],
                        }
                    ],
                },
                {
                    "view": "breakdown",
                    "values": [
                        {"entity_id": ERIN, "dimensions": [{"key": "tool", "value": "a"}], "value": 1},
                        {"entity_id": ERIN, "dimensions": [{"key": "tool", "value": "b"}], "value": 2},
                    ],
                },
            ],
        }
    ]
}


def _response() -> tuple[MetricResponse, Ledger]:
    ledger = Ledger()
    return MetricResponse(200, PAYLOAD, test_name="t", ledger=ledger), ledger


def test_a_selector_must_match_exactly_one_row() -> None:
    r, _ = _response()
    with pytest.raises(ExpectError, match="matched 0 rows"):
        r.row("m.count", "period", entity_id="nobody@example.com")
    with pytest.raises(ExpectError, match="matched 2 rows"):
        r.row("m.count", "breakdown", entity_id=ERIN)


def test_a_selected_row_must_have_its_required_fields_asserted() -> None:
    r, _ = _response()
    r.row("m.count", "peer", entity_id=ERIN).equals(target_value=3)
    with pytest.raises(ExpectError, match="leaves \\['max', 'median', 'min', 'n', 'p25', 'p75'\\] unasserted"):
        r.check_complete()

    r.row("m.count", "peer", entity_id=ERIN).equals(p25=1, median=2, p75=3, min=1, max=3).check("n", lambda n: n > 0)
    r.check_complete()


def test_reading_a_field_does_not_count_as_asserting_it() -> None:
    r, _ = _response()
    assert r.row("m.count", "period", entity_id=ERIN)["value"] == 3
    with pytest.raises(ExpectError, match="leaves \\['value'\\] unasserted"):
        r.check_complete()


def test_whole_view_rows_are_recorded_but_not_held_to_completeness() -> None:
    r, ledger = _response()
    assert len(r.rows("m.count", "period")) == 1
    assert len(r.series("m.count")) == 1
    r.check_complete()
    assert ledger.asserted == {"m.count": {"period": {"t"}, "timeseries": {"t"}}}


def test_one_and_some_select_through_a_list_field() -> None:
    r, _ = _response()
    assert one(r.breakdown("m.count"), entity_id=ERIN, dimensions={"key": "tool", "value": "b"})["value"] == 2
    assert some(r.breakdown("m.count"), dimensions={"key": "tool", "value": "z"}) == []
    with pytest.raises(ExpectError, match="matched 2 entries"):
        one(r.breakdown("m.count"), entity_id=ERIN)


def test_a_null_bucket_is_left_alone_by_a_valued_selector() -> None:
    r, _ = _response()
    points = one(r.series("m.count"), entity_id=ERIN)["points"]
    assert some(points, value=3) == [{"bucket_start": "2026-01-02", "value": 3}]
    assert one(points, bucket_start="2026-01-01")["value"] is None


@pytest.mark.parametrize(
    ("got", "expected", "equal"),
    [(3, 3.0, True), (0.1 + 0.2, 0.3, True), (3, 3.001, False), (None, None, True), ("a", "a", True)],
)
def test_numbers_compare_within_tolerance_and_nothing_else_does(got: object, expected: object, equal: bool) -> None:
    assert values_equal(got, expected) is equal, f"should be {equal}: {got!r} vs {expected!r}"

"""What an assertion over a metric-results response guarantees.

Every selection is exact — one metric per key, one view per kind, one row per selector —
a row a test touches must have its view's required fields asserted before the case ends,
and a view is booked as covered only by an assertion, never by being read.
"""

from __future__ import annotations

from typing import Any

import pytest
from insight_datapath.metric_expect import ExpectError, Ledger, MetricResponse

ALICE = "alice@example.com"
KEY = "collab.emails_sent"


def _payload() -> dict[str, Any]:
    return {
        "metrics": [
            {
                "metric_key": KEY,
                "computation": "sum",
                "views": [
                    {"view": "period", "values": [{"entity_id": ALICE, "value": 40}]},
                    {
                        "view": "peer",
                        "values": [
                            {
                                "entity_id": ALICE,
                                "target_value": 40,
                                "p25": 10,
                                "median": 20,
                                "p75": 30,
                                "min": 5,
                                "max": 40,
                                "n": 5,
                            }
                        ],
                    },
                    {
                        "view": "timeseries",
                        "bucket": "day",
                        "series": [
                            {
                                "entity_id": ALICE,
                                "dimensions": [],
                                "points": [{"bucket_start": "2026-01-01", "value": 40}],
                            }
                        ],
                    },
                ],
            }
        ]
    }


def _response(ledger: Ledger | None = None) -> MetricResponse:
    return MetricResponse(
        200, _payload(), test_name="t", ledger=ledger if ledger is not None else Ledger()
    )


def test_a_case_that_asserts_every_view_passes() -> None:
    r = _response()
    r.row(KEY, "period", entity_id=ALICE).equals(value=40)
    r.row(KEY, "peer", entity_id=ALICE).equals(
        target_value=40, p25=10, median=20, p75=30, min=5, max=40, n=5
    )
    r.row(KEY, "timeseries", entity_id=ALICE).equals(
        points=[{"bucket_start": "2026-01-01", "value": 40}]
    )
    r.check_complete()


def test_a_wrong_value_fails_naming_both_numbers() -> None:
    r = _response()
    with pytest.raises(ExpectError, match="value: expected 99"):
        r.row(KEY, "period", entity_id=ALICE).equals(value=99)


def test_an_expected_null_still_requires_the_field_to_be_there() -> None:
    r = _response()
    with pytest.raises(ExpectError, match="missing: field is missing"):
        r.row(KEY, "period", entity_id=ALICE).equals(missing=None)


def test_a_metric_the_response_does_not_carry_fails() -> None:
    r = _response()
    with pytest.raises(ExpectError, match="matched 0 metrics"):
        r.metric("collab.missing")


def test_a_view_the_metric_does_not_carry_fails() -> None:
    r = _response()
    with pytest.raises(ExpectError, match="matched 0 views"):
        r.view(KEY, "breakdown")


def test_a_selector_matching_nobody_fails() -> None:
    r = _response()
    with pytest.raises(ExpectError, match="matched 0 rows"):
        r.row(KEY, "period", entity_id="nobody@example.com")


def test_a_selector_matching_several_rows_fails() -> None:
    payload = _payload()
    payload["metrics"][0]["views"][0]["values"].append({"entity_id": ALICE, "value": 41})
    r = MetricResponse(200, payload, test_name="t", ledger=Ledger())
    with pytest.raises(ExpectError, match="matched 2 rows"):
        r.row(KEY, "period", entity_id=ALICE)


def test_a_peer_row_left_half_asserted_fails_the_case() -> None:
    r = _response()
    r.row(KEY, "peer", entity_id=ALICE).equals(target_value=40)
    with pytest.raises(ExpectError, match=r"leaves .* unasserted"):
        r.check_complete()


def test_a_predicate_counts_the_field_it_examined() -> None:
    r = _response()
    r.row(KEY, "period", entity_id=ALICE).check("value", lambda v: float(v) == 40.0, "is 40")
    r.check_complete()


def test_a_failing_predicate_names_the_rule_and_the_value() -> None:
    r = _response()
    with pytest.raises(ExpectError, match="is under 10 failed, got 40"):
        r.row(KEY, "period", entity_id=ALICE).check("value", lambda v: float(v) < 10, "is under 10")


def test_contains_matches_an_entry_of_a_nested_list() -> None:
    r = _response()
    r.row(KEY, "timeseries", entity_id=ALICE).contains(
        points={"bucket_start": "2026-01-01", "value": 40.0}
    )
    r.check_complete()


def test_contains_fails_when_no_entry_matches() -> None:
    r = _response()
    with pytest.raises(ExpectError, match="contains no match"):
        r.row(KEY, "timeseries", entity_id=ALICE).contains(points={"value": 99})


def test_nonempty_accepts_a_populated_list_and_counts_it() -> None:
    r = _response()
    r.row(KEY, "timeseries", entity_id=ALICE).nonempty("points")
    r.check_complete()


def test_nonempty_fails_on_an_empty_list() -> None:
    payload = _payload()
    payload["metrics"][0]["views"][2]["series"][0]["points"] = []
    r = MetricResponse(200, payload, test_name="t", ledger=Ledger())
    with pytest.raises(ExpectError, match="points is empty"):
        r.row(KEY, "timeseries", entity_id=ALICE).nonempty("points")


def test_two_selectors_reaching_one_row_share_what_it_has_asserted() -> None:
    """The second selection names itself in a failure, and completeness sees both."""
    r = _response()
    r.row(KEY, "peer", entity_id=ALICE).equals(target_value=40, p25=10, median=20, p75=30)
    r.row(KEY, "peer", n=5).equals(min=5, max=40, n=5)
    r.check_complete()


def test_a_view_a_test_only_read_is_not_booked_as_covered() -> None:
    ledger = Ledger()
    r = _response(ledger)
    r.rows(KEY, "period")
    assert ledger.asserted == {}


def test_a_view_a_test_asserted_through_is_booked() -> None:
    ledger = Ledger()
    r = _response(ledger)
    r.rows(KEY, "period")[0].equals(value=40)
    assert ledger.asserted == {KEY: {"period": {"t"}}}

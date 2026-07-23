from __future__ import annotations

import pytest

from lib.expect_engine import ExpectError, evaluate_case


pytestmark = pytest.mark.smoke


def _response():
    return {
        "metrics": [
            {
                "metric_key": "collab.emails_sent",
                "computation": "sum",
                "views": [
                    {
                        "view": "period",
                        "values": [{"entity_id": "alice@example.com", "value": 40}],
                    },
                    {
                        "view": "peer",
                        "values": [
                            {
                                "entity_id": "alice@example.com",
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
                                "entity_id": "alice@example.com",
                                "dimensions": [],
                                "points": [{"bucket_start": "2026-01-01", "value": 40}],
                            }
                        ],
                    },
                ],
            }
        ]
    }


def _case(expect):
    return {"name": "t", "request": {}, "expect": expect}


def test_full_pass():
    case = _case(
        [
            {"assert": "status == 200"},
            {"metric": "collab.emails_sent", "equal": {"computation": "sum"}},
            {
                "metric": "collab.emails_sent",
                "view": "period",
                "find": {"entity_id": "alice@example.com"},
                "equal": {"value": 40},
            },
            {
                "metric": "collab.emails_sent",
                "view": "peer",
                "find": {"entity_id": "alice@example.com"},
                "equal": {
                    "target_value": 40,
                    "p25": 10,
                    "median": 20,
                    "p75": 30,
                    "min": 5,
                    "max": 40,
                    "n": 5,
                },
            },
            {
                "metric": "collab.emails_sent",
                "view": "timeseries",
                "find": {"entity_id": "alice@example.com"},
                "equal": {"points": [{"bucket_start": "2026-01-01", "value": 40}]},
            },
        ]
    )
    evaluate_case(case, _response(), 200)


def test_equal_mismatch_fails():
    case = _case(
        [
            {
                "metric": "collab.emails_sent",
                "view": "period",
                "find": {"entity_id": "alice@example.com"},
                "equal": {"value": 99},
            }
        ]
    )
    with pytest.raises(ExpectError, match="value: expected 99"):
        evaluate_case(case, _response(), 200)


def test_expected_null_requires_present_field():
    case = _case(
        [
            {
                "metric": "collab.emails_sent",
                "view": "period",
                "find": {"entity_id": "alice@example.com"},
                "equal": {"missing": None},
            }
        ]
    )
    with pytest.raises(ExpectError, match="missing: field is missing"):
        evaluate_case(case, _response(), 200)


def test_unknown_metric_fails():
    case = _case([{"metric": "collab.missing", "assert": "true"}])
    with pytest.raises(ExpectError, match="matched 0 metrics"):
        evaluate_case(case, _response(), 200)


def test_unknown_view_fails():
    case = _case(
        [{"metric": "collab.emails_sent", "view": "breakdown", "assert": "true"}]
    )
    with pytest.raises(ExpectError, match="matched 0 views"):
        evaluate_case(case, _response(), 200)


def test_find_no_match_fails():
    case = _case(
        [
            {
                "metric": "collab.emails_sent",
                "view": "period",
                "find": {"entity_id": "nobody@example.com"},
                "equal": {"value": 1},
            }
        ]
    )
    with pytest.raises(ExpectError, match="matched 0 rows"):
        evaluate_case(case, _response(), 200)


def test_unasserted_peer_fields_fail():
    case = _case(
        [
            {
                "metric": "collab.emails_sent",
                "view": "peer",
                "find": {"entity_id": "alice@example.com"},
                "equal": {"target_value": 40},
            }
        ]
    )
    with pytest.raises(ExpectError, match="leaves .* unasserted"):
        evaluate_case(case, _response(), 200)


def test_cel_assertions_count_fields():
    case = _case(
        [
            {
                "metric": "collab.emails_sent",
                "view": "period",
                "find": {"entity_id": "alice@example.com"},
                "assert": "double(it.value) == 40.0",
            }
        ]
    )
    evaluate_case(case, _response(), 200)


def test_nested_contains_passes():
    case = _case(
        [
            {
                "metric": "collab.emails_sent",
                "view": "timeseries",
                "find": {"entity_id": "alice@example.com"},
                "contains": {"points": {"bucket_start": "2026-01-01", "value": 40.0}},
            }
        ]
    )
    evaluate_case(case, _response(), 200)


def test_nested_contains_mismatch_fails():
    case = _case(
        [
            {
                "metric": "collab.emails_sent",
                "view": "timeseries",
                "find": {"entity_id": "alice@example.com"},
                "contains": {"points": {"value": 99}},
            }
        ]
    )
    with pytest.raises(ExpectError, match="contains no match"):
        evaluate_case(case, _response(), 200)


def test_nonempty_passes():
    case = _case(
        [
            {
                "metric": "collab.emails_sent",
                "view": "timeseries",
                "find": {"entity_id": "alice@example.com"},
                "nonempty": ["points"],
            }
        ]
    )
    evaluate_case(case, _response(), 200)

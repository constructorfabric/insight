"""Unit tests for expect_engine (DoD cpt-bronze-to-api-e2e-dod-yaml-expect-engine).
Pure: no ClickHouse / dbt. Requires `cel-python`.
"""

from __future__ import annotations

import pytest

from lib.expect_engine import ExpectError, evaluate_case


pytestmark = pytest.mark.offline


def _batch():
    return {
        "results": [
            {
                "id": "collab",
                "status": "ok",
                "items": [
                    {"metric_key": "m365_emails_sent", "value": 40, "median": 20,
                     "range_min": 10, "range_max": 40},
                    {"metric_key": "slack_dm_ratio", "value": None,
                     "median": None, "range_min": None, "range_max": None},
                ],
            }
        ]
    }


def _case(expect):
    return {"name": "t", "request": {}, "expect": expect}


def test_full_pass():
    case = _case([
        {"assert": "status == 200"},
        {"in": "collab", "assert": "result.status == 'ok'"},
        {"in": "collab", "find": {"metric_key": "m365_emails_sent"},
         "equal": {"value": 40, "median": 20, "range_min": 10, "range_max": 40}},
        {"in": "collab", "assert": "size(items) == 2"},
        # Assert every stat this row carries — the no-unasserted-stat gate requires it.
        {"in": "collab", "find": {"metric_key": "slack_dm_ratio"},
         "equal": {"value": None, "median": None, "range_min": None, "range_max": None}},
    ])
    evaluate_case(case, _batch(), 200)  # no raise


def test_equal_mismatch_fails():
    case = _case([{"in": "collab", "find": {"metric_key": "m365_emails_sent"},
                   "equal": {"value": 99}}])
    with pytest.raises(ExpectError, match="value: expected 99"):
        evaluate_case(case, _batch(), 200)


def test_find_no_match_fails():
    case = _case([{"in": "collab", "find": {"metric_key": "nope"}, "equal": {"value": 1}}])
    with pytest.raises(ExpectError, match="matched 0 rows"):
        evaluate_case(case, _batch(), 200)


def test_unknown_result_id_fails():
    case = _case([{"in": "ghost", "assert": "true"}])
    with pytest.raises(ExpectError, match="no batch result with id 'ghost'"):
        evaluate_case(case, _batch(), 200)


def test_assert_false_fails():
    case = _case([{"in": "collab", "assert": "size(items) == 99"}])
    with pytest.raises(ExpectError, match="assert failed"):
        evaluate_case(case, _batch(), 200)


def test_cel_inequality_and_null():
    # Numeric inequalities cast with double() (CEL won't compare int to double);
    # null compares directly. Each row asserts every stat it carries (value /
    # median / range_min / range_max) so the no-unasserted-stat gate is satisfied.
    case = _case([
        {"in": "collab", "find": {"metric_key": "m365_emails_sent"},
         "assert": "double(it.value) > 39.5 && double(it.value) < 40.5 && "
                   "double(it.median) == 20.0 && double(it.range_min) == 10.0 && "
                   "double(it.range_max) == 40.0"},
        {"in": "collab", "find": {"metric_key": "slack_dm_ratio"},
         "assert": "it.value == null && it.median == null && "
                   "it.range_min == null && it.range_max == null"},
    ])
    evaluate_case(case, _batch(), 200)


def test_find_is_exact_equality_on_any_field():
    # `find` matches exact field equality — including non-key fields, no operators.
    # The matched row carries the four stats, so assert all of them too.
    case = _case([{"in": "collab", "find": {"value": 40},
                   "equal": {"metric_key": "m365_emails_sent", "value": 40,
                             "median": 20, "range_min": 10, "range_max": 40}}])
    evaluate_case(case, _batch(), 200)


def test_in_optional_with_single_result():
    # The batch has a single result, so `in` may be omitted. m365_emails_sent
    # carries the four stats — assert all of them for the no-unasserted-stat gate.
    case = _case([{"find": {"metric_key": "m365_emails_sent"},
                   "equal": {"value": 40, "median": 20,
                             "range_min": 10, "range_max": 40}}])
    evaluate_case(case, _batch(), 200)  # `in` omitted → sole result

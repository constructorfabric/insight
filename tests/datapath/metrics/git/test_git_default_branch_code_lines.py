"""Code lines whose commit reached the repository's default branch.

A commit's branch scope is the connector's flag OR membership of the default-branch set,
which a merged request into the default branch and matching content both confer. Its
neighbours own those two rules and the classifier; what neither asserts is this measure's
own emission, which is built from a different dimension tuple than the unscoped ones and
was broken down nowhere.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_default_branch_code_lines"

ERIN = "erin@example.com"
DAVE = "dave@example.com"


def test_a_line_counts_only_if_it_is_code_and_its_commit_landed(spec: SpecRun) -> None:
    """26 lines land: 12 by the connector's flag, 5 by the merged request, 9 by the
    content. Repointing the request's link row reads 21; dropping the classifier reads 66.
    src/b.rs is code that nothing says has landed, so it is the non-default 7."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.default_branch_code_lines", "views": [{"view": "period"}]},
                    {
                        "metric_key": "git.non_default_branch_code_lines",
                        "views": [{"view": "period"}],
                    },
                    {"metric_key": "git.code_lines", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_code_lines", "period", entity_id=ERIN).equals(value=26)
    r.row("git.non_default_branch_code_lines", "period", entity_id=ERIN).equals(value=7)
    r.row("git.code_lines", "period", entity_id=ERIN).equals(value=33)


def test_a_landed_test_file_raises_the_branch_total_but_not_its_code_lines(
    spec: SpecRun,
) -> None:
    """tests/a_test.rs landed, so it sits inside the unclassified default-branch total —
    12 plus 40 plus 5 plus 9 — while contributing nothing to the code one."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.default_branch_lines_added", "views": [{"view": "period"}]}
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_lines_added", "period", entity_id=ERIN).equals(value=66)


def test_each_change_type_carries_only_the_landed_code_lines_of_its_own_files(
    spec: SpecRun,
) -> None:
    """A line that lost its classification would arrive under the unknown grain."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.default_branch_code_lines",
                        "views": [{"view": "breakdown", "dimensions": ["change_type"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    for change_type, lines in (("modified", 12), ("added", 9), ("renamed", 5)):
        r.row(
            "git.default_branch_code_lines",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "change_type", "value": change_type},
        ).equals(value=lines)

    assert not some(
        r.breakdown("git.default_branch_code_lines"),
        dimensions={"key": "change_type", "value": "__unknown__"},
    ), "every landed row has its change type"


def test_an_extension_counts_only_the_landed_lines_of_the_code_files_sharing_it(
    spec: SpecRun,
) -> None:
    """src/b.rs is an rs file too, so a scope that stopped excluding it would read 28."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.default_branch_code_lines",
                        "views": [{"view": "breakdown", "dimensions": ["file_extension"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    for extension, lines in (("rs", 21), ("go", 5)):
        r.row(
            "git.default_branch_code_lines",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "file_extension", "value": extension},
        ).equals(value=lines)

    assert not some(
        r.breakdown("git.default_branch_code_lines"),
        dimensions={"key": "file_extension", "value": "__unknown__"},
    ), "every landed row has its extension"


def test_a_commit_counts_on_the_day_its_work_was_written_not_the_day_it_was_committed(
    spec: SpecRun,
) -> None:
    """dbcl-late was written on the 2nd and committed on the 5th. Its eleven lines belong
    to the window holding the 2nd, and the window holding the 5th has nothing — a build
    dating commits by the committer reads the two the other way round."""
    for window, lines in ((("2026-10-01", "2026-10-02"), 11), (("2026-10-05", "2026-10-06"), None)):
        r = spec.call(
            {
                "url": "/v1/metric-results",
                "method": "POST",
                "body": {
                    "entity": {"type": "person", "ids": [DAVE]},
                    "period": {"from": window[0], "to": window[1]},
                    "metrics": [
                        {
                            "metric_key": "git.default_branch_code_lines",
                            "views": [{"view": "period"}],
                        }
                    ],
                },
            }
        )
        assert r.status == 200

        r.row("git.default_branch_code_lines", "period", entity_id=DAVE).equals(value=lines)


def test_an_empty_window_is_null_not_zero(spec: SpecRun) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {"metric_key": "git.default_branch_code_lines", "views": [{"view": "period"}]}
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_code_lines", "period", entity_id=ERIN).equals(value=None)

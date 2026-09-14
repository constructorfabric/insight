"""Per-file line counts of a person's authored commits, split every way the rig can ask.

One commit carries every file kind the classifier distinguishes, each with its own counts,
so a breakdown value is unique to the rows it should hold and no two classifications can be
confused. Precedence runs vendored over test over docs over config over code; a binary file
the proxy reports no counts for contributes zero rather than blanking the commit; and the
branch-scope halves partition each total, which the scoped metrics read back.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_line_counts"

ERIN = "erin@example.com"


def test_lines_sum_over_every_file_row_and_split_by_category(spec: SpecRun) -> None:
    """Totals count every file row, the binary one as zero. Vendored holds both the
    test-named file under node_modules and the lockfile, and code lines never see it."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.lines_added",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["category"]},
                        ],
                    },
                    {
                        "metric_key": "git.lines_removed",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["category"]},
                        ],
                    },
                    {"metric_key": "git.code_lines", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.lines_added", "period", entity_id=ERIN).equals(value=164)
    r.row("git.lines_removed", "period", entity_id=ERIN).equals(value=111)

    for category, added, removed in (
        ("vendored", 120, 70),
        ("test", 11, 1),
        ("docs", 7, 3),
        ("config", 4, 0),
        ("code", 22, 37),
    ):
        selector = {"key": "category", "value": category}
        r.row("git.lines_added", "breakdown", entity_id=ERIN, dimensions=selector).equals(
            value=added
        )
        r.row("git.lines_removed", "breakdown", entity_id=ERIN, dimensions=selector).equals(
            value=removed
        )

    assert not some(
        r.breakdown("git.lines_added"), dimensions={"key": "category", "value": "__unknown__"}
    ), "every file row has its grain, so no unknown category may appear"

    r.row("git.code_lines", "period", entity_id=ERIN).equals(value=22)


def test_a_rename_counts_its_edited_lines_and_a_removal_its_removed_lines(spec: SpecRun) -> None:
    """The change types use the proxy's vocabulary, and a removal adds nothing."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.lines_added",
                        "views": [{"view": "breakdown", "dimensions": ["change_type"]}],
                    },
                    {
                        "metric_key": "git.lines_removed",
                        "views": [{"view": "breakdown", "dimensions": ["change_type"]}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    for change_type, added, removed in (
        ("modified", 46, 29),
        ("added", 115, 51),
        ("renamed", 3, 1),
        ("removed", 0, 30),
    ):
        selector = {"key": "change_type", "value": change_type}
        r.row("git.lines_added", "breakdown", entity_id=ERIN, dimensions=selector).equals(
            value=added
        )
        r.row("git.lines_removed", "breakdown", entity_id=ERIN, dimensions=selector).equals(
            value=removed
        )


def test_file_extension_is_independent_of_category(spec: SpecRun) -> None:
    """`.yaml` spans test and config and `.rs` spans every code change type, while the
    binary file is present with zero lines rather than absent."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.lines_added",
                        "views": [{"view": "breakdown", "dimensions": ["file_extension"]}],
                    },
                    {
                        "metric_key": "git.lines_removed",
                        "views": [{"view": "breakdown", "dimensions": ["file_extension"]}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    for extension, added, removed in (("rs", 27, 38), ("yaml", 10, 0)):
        selector = {"key": "file_extension", "value": extension}
        r.row("git.lines_added", "breakdown", entity_id=ERIN, dimensions=selector).equals(
            value=added
        )
        r.row("git.lines_removed", "breakdown", entity_id=ERIN, dimensions=selector).equals(
            value=removed
        )

    r.row(
        "git.lines_added",
        "breakdown",
        entity_id=ERIN,
        dimensions={"key": "file_extension", "value": "png"},
    ).equals(value=0)


def test_branch_scope_partitions_the_total_and_the_scoped_metrics_read_the_same_halves(
    spec: SpecRun,
) -> None:
    """The landed commit and the in-flight one split each total, and the four scoped
    metrics serve exactly those halves."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.lines_added",
                        "views": [{"view": "breakdown", "dimensions": ["branch_scope"]}],
                    },
                    {
                        "metric_key": "git.lines_removed",
                        "views": [{"view": "breakdown", "dimensions": ["branch_scope"]}],
                    },
                    {"metric_key": "git.default_branch_lines_added", "views": [{"view": "period"}]},
                    {
                        "metric_key": "git.non_default_branch_lines_added",
                        "views": [{"view": "period"}],
                    },
                    {
                        "metric_key": "git.default_branch_lines_removed",
                        "views": [{"view": "period"}],
                    },
                    {
                        "metric_key": "git.non_default_branch_lines_removed",
                        "views": [{"view": "period"}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    for scope, added, removed in (("default", 155, 107), ("non_default", 9, 4)):
        selector = {"key": "branch_scope", "value": scope}
        r.row("git.lines_added", "breakdown", entity_id=ERIN, dimensions=selector).equals(
            value=added
        )
        r.row("git.lines_removed", "breakdown", entity_id=ERIN, dimensions=selector).equals(
            value=removed
        )

    r.row("git.default_branch_lines_added", "period", entity_id=ERIN).equals(value=155)
    r.row("git.non_default_branch_lines_added", "period", entity_id=ERIN).equals(value=9)
    r.row("git.default_branch_lines_removed", "period", entity_id=ERIN).equals(value=107)
    r.row("git.non_default_branch_lines_removed", "period", entity_id=ERIN).equals(value=4)


def test_empty_window(spec: SpecRun) -> None:
    """A window holding none of the commits serves null, not zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2025-01-01", "to": "2025-01-31"},
                "metrics": [
                    {"metric_key": "git.lines_added", "views": [{"view": "period"}]},
                    {"metric_key": "git.lines_removed", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.lines_added", "period", entity_id=ERIN).equals(value=None)
    r.row("git.lines_removed", "period", entity_id=ERIN).equals(value=None)

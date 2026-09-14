"""Every line removed by a commit that reached the repository's default branch.

A whole-file deletion contributes every line the file held, which makes this measure the
one most exposed to how a change's content is identified — in both directions: two
removals of different content at one path are two removals, and one removal carried by
two commits is one. Both halves are owned on the unscoped measure elsewhere; what is new
here is the change-type split of the scoped measure and the docs and unknown values of
its category split.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_default_branch_lines_removed"

ERIN = "erin@example.com"
WINDOW = {"from": "2026-10-01", "to": "2026-10-02"}


def _request(*metrics: dict, period: dict | None = None) -> dict:
    return {
        "url": "/v1/metric-results",
        "method": "POST",
        "body": {
            "entity": {"type": "person", "ids": [ERIN]},
            "period": period or WINDOW,
            "metrics": list(metrics),
        },
    }


def test_content_identity_holds_in_both_directions(spec: SpecRun) -> None:
    """The two removals at `docs/a.md` took different content, so both count; the two
    rows reaching one content at `src/c.rs` are one change, so only the earlier counts.
    The commit whose diff never arrived contributes through its own stats."""
    r = spec.call(
        _request(
            {"metric_key": "git.default_branch_lines_removed", "views": [{"view": "period"}]},
            {"metric_key": "git.non_default_branch_lines_removed", "views": [{"view": "period"}]},
        )
    )
    assert r.status == 200

    r.row("git.default_branch_lines_removed", "period", entity_id=ERIN).equals(value=72)
    r.row("git.non_default_branch_lines_removed", "period", entity_id=ERIN).equals(value=7)


def test_a_commit_with_no_collected_diff_reports_under_an_unknown_category(
    spec: SpecRun,
) -> None:
    """The fallback path cannot classify what it never saw, so its lines arrive under an
    unknown category rather than being dropped or guessed into a real one."""
    r = spec.call(
        _request(
            {
                "metric_key": "git.default_branch_lines_removed",
                "views": [{"view": "breakdown", "dimensions": ["category"]}],
            }
        )
    )
    assert r.status == 200

    for category, removed in (("code", 16), ("docs", 26), ("__unknown__", 30)):
        r.row(
            "git.default_branch_lines_removed",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "category", "value": category},
        ).equals(value=removed)

    categories = r.breakdown("git.default_branch_lines_removed")
    assert len(categories) == 3, f"a fourth category appeared: {categories!r}"


def test_a_whole_file_deletion_reads_as_removed_not_as_an_edit(spec: SpecRun) -> None:
    """Both deletions land under `removed` and the edit under `modified`; the fallback,
    which knows neither, lands under an unknown change type of its own.

    The re-add removed nothing and reports ZERO rather than being absent; the
    zero-versus-absent distinction itself is owned on the unscoped measures.
    """
    r = spec.call(
        _request(
            {
                "metric_key": "git.default_branch_lines_removed",
                "views": [{"view": "breakdown", "dimensions": ["change_type"]}],
            }
        )
    )
    assert r.status == 200

    for change_type, removed in (
        ("modified", 16),
        ("removed", 26),
        ("__unknown__", 30),
        ("added", 0),
    ):
        r.row(
            "git.default_branch_lines_removed",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "change_type", "value": change_type},
        ).equals(value=removed)

    change_types = r.breakdown("git.default_branch_lines_removed")
    assert len(change_types) == 4, f"a fifth change type appeared: {change_types!r}"


def test_an_empty_window_is_null_not_zero(spec: SpecRun) -> None:
    r = spec.call(
        _request(
            {"metric_key": "git.default_branch_lines_removed", "views": [{"view": "period"}]},
            period={"from": "2026-12-01", "to": "2026-12-31"},
        )
    )
    assert r.status == 200

    r.row("git.default_branch_lines_removed", "period", entity_id=ERIN).equals(value=None)

"""A commit whose file changes never reached the warehouse still reports its own size.

Bronze: GitHub commits carrying their source additions/deletions, with file-change rows
for only some of them. Lines added / Lines removed count every commit's size while Code
lines counts only the commits whose file grain is known; an empty commit reads zero, not
an absent value. A commit whose change was collected and then lost the content dedup is
not uncollected: Commit size keeps only the part of its stats the dedup did not remove.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_uncollected_file_changes"

ALICE = "alice@example.com"
BOB = "bob@example.com"
CAROL = "carol@example.com"
DAVE = "dave@example.com"

REPOSITORY = "git-test:constructor/insight"


def test_a_commit_with_no_file_changes_still_reports_its_own_size(spec: SpecRun) -> None:
    """Both of alice's commits count and both sizes count with them; the breakdown names the
    unknown file grain instead of dropping it, and Code lines counts only the known half."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ALICE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                    {
                        "metric_key": "git.lines_added",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["category"]},
                        ],
                    },
                    {"metric_key": "git.lines_removed", "views": [{"view": "period"}]},
                    {"metric_key": "git.code_lines", "views": [{"view": "period"}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "period", entity_id=ALICE).equals(value=2)
    r.row("git.lines_added", "period", entity_id=ALICE).equals(value=30)
    r.row("git.lines_removed", "period", entity_id=ALICE).equals(value=3)

    r.row(
        "git.lines_added",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "category", "value": "code"},
    ).equals(value=10)
    r.row(
        "git.lines_added",
        "breakdown",
        entity_id=ALICE,
        dimensions={"key": "category", "value": "__unknown__"},
    ).equals(value=20)

    r.row("git.code_lines", "period", entity_id=ALICE).equals(value=10)
    r.row("git.commit_size", "period", entity_id=ALICE).equals(value=16.5)


def test_a_commit_that_changed_nothing_reports_zero_not_an_absent_value(spec: SpecRun) -> None:
    """bob's commit changed nothing: zero size, not an absent value."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [BOB]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "period"}]},
                    {"metric_key": "git.lines_removed", "views": [{"view": "period"}]},
                    {"metric_key": "git.commit_size", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "period", entity_id=BOB).equals(value=1)
    r.row("git.lines_added", "period", entity_id=BOB).equals(value=0)
    r.row("git.lines_removed", "period", entity_id=BOB).equals(value=0)
    r.row("git.commit_size", "period", entity_id=BOB).equals(value=0)


def test_commit_that_lost_the_content_dedup_is_not_treated_as_uncollected(spec: SpecRun) -> None:
    """carol's repeat commit lost the content dedup, so no `__unknown__` grain appears for it
    and keeps only the 15 lines the dedup did not remove, so her sizes {11, 15} median to 13."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [CAROL]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.commits", "views": [{"view": "period"}]},
                    {
                        "metric_key": "git.lines_added",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["category"]},
                        ],
                    },
                    {"metric_key": "git.commit_size", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.commits", "period", entity_id=CAROL).equals(value=2)
    r.row("git.lines_added", "period", entity_id=CAROL).equals(value=10)
    r.row(
        "git.lines_added",
        "breakdown",
        entity_id=CAROL,
        dimensions={"key": "category", "value": "code"},
    ).equals(value=10)

    unknown_grain = some(
        r.breakdown("git.lines_added"),
        entity_id=CAROL,
        dimensions={"key": "category", "value": "__unknown__"},
    )
    assert unknown_grain == []

    r.row("git.commit_size", "period", entity_id=CAROL).equals(value=13)


def test_an_uncollected_size_on_a_branch_reaches_the_lines_but_not_the_code_lines(
    spec: SpecRun,
) -> None:
    """dave's pair sits on a branch that has not landed. The commit whose diff never
    arrived carries its reported size into the branch-scoped line measures under the
    unknown grain, and contributes nothing to the branch-scoped code lines, which count
    only the file whose category is known. The default side of every pair is absent, not
    zero."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [DAVE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {"metric_key": "git.non_default_branch_commits", "views": [{"view": "period"}]},
                    {
                        "metric_key": "git.non_default_branch_lines_added",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["category"]},
                        ],
                    },
                    {
                        "metric_key": "git.non_default_branch_lines_removed",
                        "views": [{"view": "period"}],
                    },
                    {
                        "metric_key": "git.non_default_branch_code_lines",
                        "views": [{"view": "period"}],
                    },
                    {"metric_key": "git.default_branch_lines_added", "views": [{"view": "period"}]},
                    {"metric_key": "git.default_branch_code_lines", "views": [{"view": "period"}]},
                    {"metric_key": "git.lines_added", "views": [{"view": "period"}]},
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.non_default_branch_commits", "period", entity_id=DAVE).equals(value=2)
    r.row("git.non_default_branch_lines_added", "period", entity_id=DAVE).equals(value=42)
    r.row("git.non_default_branch_lines_removed", "period", entity_id=DAVE).equals(value=7)
    r.row("git.non_default_branch_code_lines", "period", entity_id=DAVE).equals(value=12)
    r.row("git.lines_added", "period", entity_id=DAVE).equals(value=42)

    for category, lines in (("code", 12), ("__unknown__", 30)):
        r.row(
            "git.non_default_branch_lines_added",
            "breakdown",
            entity_id=DAVE,
            dimensions={"key": "category", "value": category},
        ).equals(value=lines)

    r.row("git.default_branch_lines_added", "period", entity_id=DAVE).equals(value=None)
    r.row("git.default_branch_code_lines", "period", entity_id=DAVE).equals(value=None)


def test_an_uncollected_size_files_under_the_same_repository_as_the_rows_beside_it(
    spec: SpecRun,
) -> None:
    """The reported size is built from a different dimension tuple than the file-derived
    rows, and a key missing from it would split one repository into two — so the whole of
    dave's branch total has to arrive under a single repository."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [DAVE]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.non_default_branch_lines_added",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row(
        "git.non_default_branch_lines_added",
        "breakdown",
        entity_id=DAVE,
        dimensions={"key": "repository", "value": REPOSITORY},
    ).equals(value=42)
    assert len(r.breakdown("git.non_default_branch_lines_added")) == 1, (
        "one repository, so one row — a key missing from either tuple would split it"
    )

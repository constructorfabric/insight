"""Every line added by a commit that reached the repository's default branch.

The lines come from the file rows when a commit has any and from the commit's own stats
when it has none — two emission paths building their own dimension tuples. The second is
what this fixture is for: `git_uncollected_file_changes` owns that path for the unscoped
measures and asserts none of the branch-scoped ones, so nothing said the fallback reaches
this measure at all. A breakdown groups by a hidden source-id key as well as the visible
ones, so the row count of the repository split is what fails if a fallback row loses it.
"""

from __future__ import annotations

import pytest
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_default_branch_lines_added"

ERIN = "erin@example.com"


def test_a_commit_with_no_collected_diff_still_reports_its_lines_under_an_unknown_category(
    spec: SpecRun,
) -> None:
    """11 + 20 arrive from the file rows and 30 from the commit that has none.

    The fallback path cannot classify what it never saw, so its lines land under
    `__unknown__` rather than being dropped or guessed into a real category.
    """
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.default_branch_lines_added",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["category"]},
                        ],
                    },
                    {
                        "metric_key": "git.non_default_branch_lines_added",
                        "views": [{"view": "period"}],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_lines_added", "period", entity_id=ERIN).equals(value=61)
    r.row("git.non_default_branch_lines_added", "period", entity_id=ERIN).equals(value=7)

    for category, lines in (("code", 11), ("docs", 20), ("__unknown__", 30)):
        r.row(
            "git.default_branch_lines_added",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "category", "value": category},
        ).equals(value=lines)

    categories = r.breakdown("git.default_branch_lines_added")
    assert len(categories) == 3, f"a fourth category row appeared: {categories!r}"


def test_one_repository_answers_with_one_row_whichever_path_its_lines_came_from(
    spec: SpecRun,
) -> None:
    """A fallback row whose dimension tuple loses the source-id key answers as its own
    repository: two rows carrying one repository's name, splitting its lines between
    them, one of them unlinkable. The row count is what stands in, since the key itself
    reaches the response only as a link the rig registers no source for."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.default_branch_lines_added",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    repositories = r.breakdown("git.default_branch_lines_added")
    assert len(repositories) == 1, (
        f"one repository answered with more than one row: {repositories!r}"
    )

    r.row(
        "git.default_branch_lines_added",
        "breakdown",
        entity_id=ERIN,
        dimensions={"key": "repository", "value": "git-test:acme/dbla"},
    ).equals(value=61)


def test_an_uncollected_size_on_the_default_branch_reaches_the_lines_but_not_the_code_lines(
    spec: SpecRun,
) -> None:
    """dbla-blind's thirty lines are inside the lines measure above and must be absent
    here: with no file rows there is no path to classify, so the code-lines measure is
    src/a.rs alone. The other scope's half of this rule lives in
    git_uncollected_file_changes; a fallback emitted into this measure reads 41."""
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
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_code_lines", "period", entity_id=ERIN).equals(value=11)
    r.row("git.non_default_branch_code_lines", "period", entity_id=ERIN).equals(value=7)


def test_an_empty_window_is_null_not_zero(spec: SpecRun) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [
                    {"metric_key": "git.default_branch_lines_added", "views": [{"view": "period"}]}
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.default_branch_lines_added", "period", entity_id=ERIN).equals(value=None)

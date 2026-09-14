"""Lines added to files the classifier calls code, and nothing else.

Tests, configuration, documentation and vendored content are all excluded, so what this
metric measures is decided entirely by where the classifier draws its lines. Precedence
runs vendored, test, docs, config, code, and a path answering to two rules answers to the
earlier one. Because the metric is binary it cannot itself tell docs from config, so that
one claim is asserted on the unclassified total's category split in the same request.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_code_lines"

ERIN = "erin@example.com"
REPOSITORY = "git-test:acme/codelines"


def test_only_code_paths_reach_the_metric_and_docs_outranks_config(spec: SpecRun) -> None:
    """24 of 130 lines are code. The category split says where the other 106 went, and
    `docs/settings.yaml` is the load-bearing row: `.yaml` answers to the config rule too,
    so 47 under docs with no config row at all is the only place that order is visible."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.code_lines",
                        "views": [{"view": "breakdown", "dimensions": ["repository"]}],
                    },
                    {
                        "metric_key": "git.lines_added",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["category"]},
                        ],
                    },
                ],
            },
        }
    )
    assert r.status == 200

    r.row(
        "git.code_lines",
        "breakdown",
        entity_id=ERIN,
        dimensions={"key": "repository", "value": REPOSITORY},
    ).equals(value=24)
    r.row("git.lines_added", "period", entity_id=ERIN).equals(value=130)

    for category, added in (("code", 24), ("test", 59), ("docs", 47)):
        r.row(
            "git.lines_added",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "category", "value": category},
        ).equals(value=added)

    categories = r.breakdown("git.lines_added")
    for absent in ("config", "vendored"):
        assert not some(categories, dimensions={"key": "category", "value": absent}), (
            f"no path here is {absent}, so the classifier must produce no such row"
        )


def test_lines_follow_their_commits_branch_scope(spec: SpecRun) -> None:
    """The landed commit's three code files against the in-flight one's, and the two
    scopes partition the total."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.code_lines",
                        "views": [{"view": "breakdown", "dimensions": ["branch_scope"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    for scope, lines in (("default", 21), ("non_default", 3)):
        r.row(
            "git.code_lines",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "branch_scope", "value": scope},
        ).equals(value=lines)


def test_a_rename_is_its_own_change_type_and_carries_only_its_edited_lines(
    spec: SpecRun,
) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.code_lines",
                        "views": [{"view": "breakdown", "dimensions": ["change_type"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    for change_type, lines in (("modified", 14), ("added", 6), ("renamed", 4)):
        r.row(
            "git.code_lines",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "change_type", "value": change_type},
        ).equals(value=lines)


def test_an_extension_gathers_every_code_file_sharing_it(spec: SpecRun) -> None:
    """`rs` spans app, moved and wip; `go` is util's alone. `test/single.rs` is an rs file
    the classifier calls test: were that rule to stop firing, rs would read 38. The other
    extensions appear at all only for a comparable reason, so none may have a row.

    The absence guard is the weaker half: extensions are compared lower-case because gold
    lower-cases them, so a regression losing BOTH the classifier's case-insensitivity and
    that lower-casing would slip past it. The total asserted in the first case is what
    catches that one."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-02"},
                "metrics": [
                    {
                        "metric_key": "git.code_lines",
                        "views": [{"view": "breakdown", "dimensions": ["file_extension"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    for extension, lines in (("rs", 18), ("go", 6)):
        r.row(
            "git.code_lines",
            "breakdown",
            entity_id=ERIN,
            dimensions={"key": "file_extension", "value": extension},
        ).equals(value=lines)

    extensions = r.breakdown("git.code_lines")
    for absent in ("yaml", "kt", "md", "ts"):
        assert not some(extensions, dimensions={"key": "file_extension", "value": absent}), (
            f"every {absent} file here is classified out of the metric"
        )


def test_an_empty_window_is_null_not_zero(spec: SpecRun) -> None:
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-12-01", "to": "2026-12-31"},
                "metrics": [{"metric_key": "git.code_lines", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200

    r.row("git.code_lines", "period", entity_id=ERIN).equals(value=None)

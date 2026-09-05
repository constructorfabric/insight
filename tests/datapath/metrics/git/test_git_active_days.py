"""Distinct calendar days carrying at least one authored, non-merge commit.

Bronze: the commits each git connector reports, with the author date as the connector
received it, offset and all. Silver: one class row per commit, its author date parsed to
a single instant, so two spellings of one moment become one value. Gold: one `commit_day`
row per person, day and dimension tuple, and the metric counts those days distinctly
rather than summing the rows. Erin's seven day rows are three days — 10-01 reached from
three repositories across two connectors, 10-02 from two connectors, and one instant
written both three hours ahead of UTC and in UTC. Summing the rows reads 7, counting the
merge-only day reads 4, and splitting the offset pair reads 4. Every level collapses its
own days, so the five repository values sum to 6 against a total of 3.

INVARIANT: no assertion names an absolute date for the offset pair. `toDate` renders in
the warehouse's own timezone, so which date those two rows land on is a property of the
stand, while their landing on the SAME date holds under every timezone.
"""

from __future__ import annotations

import pytest
from insight_datapath.metric_expect import some
from insight_datapath.spec_runner import SpecRun

pytestmark = pytest.mark.fixture

SPEC = "git_active_days"

ERIN = "erin@example.com"

REPOSITORY_API = {"key": "repository", "value": "git-test:acme/api"}
REPOSITORY_WEB = {"key": "repository", "value": "git-test:acme/web"}
REPOSITORY_TOOLS = {"key": "repository", "value": "git-test:globex/tools"}
REPOSITORY_MOBILE = {"key": "repository", "value": "bitbucket-test:acme/mobile"}
REPOSITORY_PLATFORM = {"key": "repository", "value": "gitlab-test:acme/platform"}

SOURCE_GITHUB = {"key": "source", "value": "github"}
SOURCE_BITBUCKET = {"key": "source", "value": "bitbucket_cloud"}
SOURCE_GITLAB = {"key": "source", "value": "gitlab"}

PROJECT_GITHUB_ACME = {"key": "project", "value": "git-test:acme"}
PROJECT_GITHUB_GLOBEX = {"key": "project", "value": "git-test:globex"}
PROJECT_BITBUCKET_ACME = {"key": "project", "value": "bitbucket-test:acme"}
PROJECT_GITLAB_ACME = {"key": "project", "value": "gitlab-test:acme"}


def test_a_repository_counts_its_own_days_and_the_total_counts_each_day_once(
    spec: SpecRun,
) -> None:
    """Five repositories hold six days between them because every repository that saw
    10-01 counts it, and the total counts that day once: three days, not six."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-07"},
                "metrics": [
                    {
                        "metric_key": "git.active_days",
                        "views": [
                            {"view": "period"},
                            {"view": "breakdown", "dimensions": ["repository"]},
                        ],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.active_days", "period", entity_id=ERIN).equals(value=3)

    by_repository = r.breakdown("git.active_days")
    assert len(by_repository) == 5, f"one value per repository: {by_repository!r}"

    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=REPOSITORY_API).equals(value=2)
    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=REPOSITORY_WEB).equals(value=1)
    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=REPOSITORY_TOOLS).equals(
        value=1
    )
    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=REPOSITORY_MOBILE).equals(
        value=1
    )
    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=REPOSITORY_PLATFORM).equals(
        value=1
    )


def test_a_connector_counts_only_the_days_it_saw(spec: SpecRun) -> None:
    """Github holds all three days; 10-01 for bitbucket and 10-02 for gitlab are days
    github already holds, and each connector still counts its own."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-07"},
                "metrics": [
                    {
                        "metric_key": "git.active_days",
                        "views": [{"view": "breakdown", "dimensions": ["source"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    by_source = r.breakdown("git.active_days")
    assert len(by_source) == 3, f"one value per connector: {by_source!r}"

    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITHUB).equals(value=3)
    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=SOURCE_BITBUCKET).equals(
        value=1
    )
    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=SOURCE_GITLAB).equals(value=1)


def test_a_project_gathers_its_repositories_days_and_stays_inside_its_connector(
    spec: SpecRun,
) -> None:
    """Four values, three of which render as the label `acme`: the source id inside the
    value is the only thing keeping those namespaces apart. acme/api contributes two days
    and acme/web one, but they share 10-01, so the github project reads 2 rather than 3."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-07"},
                "metrics": [
                    {
                        "metric_key": "git.active_days",
                        "views": [{"view": "breakdown", "dimensions": ["project"]}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    by_project = r.breakdown("git.active_days")
    assert len(by_project) == 4, f"one value per namespaced project: {by_project!r}"

    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=PROJECT_GITHUB_ACME).equals(
        value=2
    )
    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=PROJECT_GITHUB_GLOBEX).equals(
        value=1
    )
    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=PROJECT_BITBUCKET_ACME).equals(
        value=1
    )
    r.row("git.active_days", "breakdown", entity_id=ERIN, dimensions=PROJECT_GITLAB_ACME).equals(
        value=1
    )


def test_one_instant_written_two_ways_is_one_active_day_and_a_merge_day_is_none(
    spec: SpecRun,
) -> None:
    """Four commits from two connectors share 10-01 and read as one day, two commits share
    10-02, and 10-05 holds a merge and nothing else, so it carries no day at all."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-07"},
                "metrics": [
                    {
                        "metric_key": "git.active_days",
                        "views": [{"view": "timeseries", "bucket": "day"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    daily = r.row("git.active_days", "timeseries", entity_id=ERIN)
    daily.contains(points={"bucket_start": "2026-10-01", "value": 1})
    daily.contains(points={"bucket_start": "2026-10-02", "value": 1})
    assert some(daily["points"], bucket_start="2026-10-05", value=None), (
        "a merge is not authored work: 2026-10-05 is not active"
    )


def test_a_week_bucket_collapses_the_days_inside_it_rather_than_summing_them(
    spec: SpecRun,
) -> None:
    """The week is the only bucket wider than the distinct key, so the only place where
    summing day rows instead of counting days would show: seven rows, three days."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-10-01", "to": "2026-10-07"},
                "metrics": [
                    {
                        "metric_key": "git.active_days",
                        "views": [{"view": "timeseries", "bucket": "week"}],
                    }
                ],
            },
        }
    )
    assert r.status == 200

    r.row("git.active_days", "timeseries", entity_id=ERIN).contains(points={"value": 3})


def test_an_empty_window_is_null_not_zero(spec: SpecRun) -> None:
    """A month holding no commits serves no count, rather than claiming zero active days."""
    r = spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [ERIN]},
                "period": {"from": "2026-11-01", "to": "2026-11-30"},
                "metrics": [{"metric_key": "git.active_days", "views": [{"view": "period"}]}],
            },
        }
    )
    assert r.status == 200
    r.row("git.active_days", "period", entity_id=ERIN).equals(value=None)

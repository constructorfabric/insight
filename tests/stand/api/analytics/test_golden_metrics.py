"""`POST /v1/metric-results` — exact task totals against the seed's own arithmetic.

    POST /v1/metric-results   200, values EQUAL to the manifest's golden_metrics

Every other case in this suite asserts shape, consistency or non-nullness; none
of them would notice the seed silently halving what it writes, or gold counting
each close twice — self-consistent wrong numbers pass self-consistency checks.
This module is the one place with an independent oracle: the seed manifest's
`golden_metrics`, derived by the seeder from the same deterministic plan the
row generator emits from, never read back out of the database. A regression
anywhere in the chain — the generator dropping bug-type issues, gold
misclassifying a kind, the period view double-counting — lands here as a wrong
exact number.

Skips, with the reason, on a stand whose manifest predates `golden_metrics`,
carries a newer version of it, or was seeded without the silver step.
"""

from __future__ import annotations

from collections.abc import Callable
from datetime import date, timedelta

import pytest
from insight_stand import GoldenTasks, Manifest, PersonaSession, analytics_path

from ..schemas import MetricResultsResponse, PeriodView
from . import MAX_QUERY_SPAN_DAYS

METRIC_RESULTS = analytics_path("/v1/metric-results")

#: metric_key → the golden per-person field it must equal. The three keys read
#: the `tasks_closed` / `bugs_fixed` / `closed_non_bug` evidence measures, which
#: are the family the seed derives exact expectations for.
GOLDEN_TASK_METRICS = {
    "tasks.closed": "tasks_closed",
    "tasks.bugs_fixed": "bugs_fixed",
    "tasks.closed_non_bug": "closed_non_bug",
}

#: Both inside a development lead's visible set, so one session answers for
#: both — and a lead plus an IC covers the two persona volumes the generator
#: scales differently.
GOLDEN_PERSONAS = ("dev_lead", "development_ic")


def _golden_tasks(manifest: Manifest) -> GoldenTasks:
    golden = manifest.golden_metrics
    if golden is None:
        pytest.skip(f"the manifest at {manifest.source_path} carries no golden_metrics")
    if golden.tasks is None:
        pytest.skip(f"golden_metrics version {golden.version} is newer than this suite understands")
    if "silver" not in manifest.seeded:
        pytest.skip(
            "the stand was seeded without the silver step — the golden numbers "
            "describe rows it does not have"
        )
    return golden.tasks


def _golden_periods(tasks: GoldenTasks, manifest: Manifest) -> list[tuple[str, str]]:
    """The exact range the golden numbers describe, as askable periods.

    Never `query_window`: that helper trims a long seed window to the queryable
    tail, and a trimmed period drops early closes the totals include. The API
    answers at most `MAX_QUERY_SPAN_DAYS` per request, so a wider window is
    covered by DISJOINT adjacent slices instead — the three metrics are sums
    over the close date, so the slices add back to the window total exactly.
    """
    start_text, _, end_text = tasks.window.partition("..")
    assert start_text and end_text, (
        f"the manifest at {manifest.source_path} carries golden window "
        f"{tasks.window!r}, which is not a `from..to` range"
    )
    start = date.fromisoformat(start_text)
    end = date.fromisoformat(end_text)

    periods: list[tuple[str, str]] = []
    while start <= end:
        slice_end = min(start + timedelta(days=MAX_QUERY_SPAN_DAYS), end)
        periods.append((start.isoformat(), slice_end.isoformat()))
        start = slice_end + timedelta(days=1)
    return periods


@pytest.mark.requires_seed(*GOLDEN_PERSONAS)
@pytest.mark.reliability
def test_task_totals_equal_the_seeds_own_arithmetic(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    tasks = _golden_tasks(stand_manifest)
    periods = _golden_periods(tasks, stand_manifest)
    api = session_for("dev_lead").client

    for fixture_name in GOLDEN_PERSONAS:
        person = stand_manifest.fixture(fixture_name)
        expected = tasks.per_person.get(person.email)
        assert expected is not None, (
            f"golden_metrics carries no entry for {fixture_name} ({person.email}) — "
            f"the manifest and the roster disagree about who tracks tasks"
        )

        observed: dict[str, float] = dict.fromkeys(GOLDEN_TASK_METRICS, 0)
        for start, end in periods:
            response = api.post(
                METRIC_RESULTS,
                json_body={
                    "entity": {"type": "person", "ids": [person.uuid]},
                    "period": {"from": start, "to": end},
                    "metrics": [
                        {"metric_key": key, "views": [{"view": "period"}]}
                        for key in GOLDEN_TASK_METRICS
                    ],
                },
            )
            assert response.status_code == 200, (
                f"{start}..{end}: status={response.status_code} {response.text[:300]}"
            )

            for metric in response.parse(MetricResultsResponse).metrics:
                for view in metric.root.views:
                    assert isinstance(view.root, PeriodView), (
                        f"asked for the period view and got {type(view.root).__name__}"
                    )
                    for value in view.root.values:
                        assert value.entity_id == person.uuid
                        # Null means gold found no rows in this slice, which
                        # contributes zero to the window total.
                        observed[metric.root.metric_key] += value.value or 0

        for metric_key, field in GOLDEN_TASK_METRICS.items():
            want = getattr(expected, field)
            got = observed[metric_key]
            assert got == want, (
                f"{metric_key} for {fixture_name} ({person.email}) over "
                f"{tasks.window}: the stand answered {got} while the seed's own "
                f"arithmetic says {want} — somewhere between the generator, gold "
                f"and the period view a close was lost, invented or reclassified"
            )

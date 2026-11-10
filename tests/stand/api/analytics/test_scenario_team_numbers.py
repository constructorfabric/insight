from __future__ import annotations

from collections import defaultdict
from collections.abc import Callable, Sequence

import pytest
from insight_stand import Manifest, Person, PersonaSession, identity_path

from ..identity.views import forest_emails
from ..schemas import MetricResultsResponse, ProblemDocument, SubchartForest
from . import query_window
from .team_numbers import (
    GOLDEN_FIELDS,
    _ask_results,
    _golden,
    _period_values,
    _slices,
    _task_metric_labels,
    _team,
    _uuids,
)

SUBCHART = identity_path("/v1/subchart")


def _assert_subchart_holds(session: PersonaSession, team: Sequence[Person]) -> None:
    response = session.client.get(SUBCHART)
    assert response.status_code == 200, f"subchart: {response.status_code} {response.text[:300]}"

    missing = {person.email for person in team} - forest_emails(response.parse(SubchartForest))
    assert not missing, f"{session.name}'s subchart does not hold their team: {sorted(missing)}"


@pytest.mark.requires_seed("dev_lead", "hr_lead")
@pytest.mark.parametrize("lead", ["dev_lead", "hr_lead"])
@pytest.mark.reliability
def test_a_lead_reads_the_whole_team_in_one_request_and_every_task_total_is_the_seeds(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest, lead: str
) -> None:
    golden = _golden(stand_manifest)
    session = session_for(lead)
    team = _team(stand_manifest, lead)

    _task_metric_labels(session.client)
    _assert_subchart_holds(session, team)

    totals: defaultdict[tuple[str, str], float] = defaultdict(float)
    for start, end in _slices(golden):
        for cell, value in _period_values(session.client, _uuids(team), start, end).items():
            totals[cell] += value or 0

    for person in team:
        expected = golden.per_person.get(person.email)
        assert expected is not None, f"golden_metrics carries no entry for {person.email}"
        for key, field in GOLDEN_FIELDS.items():
            assert totals[key, person.uuid] == getattr(expected, field), (
                f"{lead}'s team view gives {person.email} {totals[key, person.uuid]} {key} "
                f"over {golden.window}; the seed wrote {getattr(expected, field)}"
            )


@pytest.mark.requires_seed("dev_lead", "hr_lead")
@pytest.mark.security
def test_a_team_request_naming_one_person_from_another_team_is_refused_whole(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    start, end = query_window(stand_manifest)
    api = session_for("hr_lead").client
    own = _uuids(_team(stand_manifest, "hr_lead"))
    other = _uuids(_team(stand_manifest, "dev_lead"))

    answered = _ask_results(api, own, start, end)
    assert answered.status_code == 200, f"{answered.status_code} {answered.text[:300]}"
    answered.parse(MetricResultsResponse)

    for label, ids in (
        ("the development team", other),
        ("the HR team plus the development lead", [*own, other[0]]),
    ):
        refused = _ask_results(api, ids, start, end)
        assert refused.status_code == 403, (
            f"hr_lead asking for {label} answered {refused.status_code}: {refused.text[:300]}"
        )
        assert refused.parse(ProblemDocument).status == 403

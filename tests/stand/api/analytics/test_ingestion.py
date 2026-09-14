"""`GET /v1/ingestion/intensity` on analytics — bronze extraction intensity.

    GET /v1/ingestion/intensity   200 for the admin operator · 400 closed-set
                                  grain/series · 400 malformed window ·
                                  400 malformed scope · 403 for everybody else

The ops lens behind the admin-only Ingestion surface. Two things make it worth a
deployed-path test rather than only unit coverage.

First, it is the one analytics read that is NOT tenant-scoped. Bronze rows carry
no tenant, so the usual visible-set gate has nothing to scope by and the whole
boundary is the admin grant — the same `admin` row in `identity.person_roles`
that `/v1/usage/summary` reads, which the seed gives to the admin operator alone.
A regression that dropped the gate would expose every connector's extraction
volume to any signed-in caller, and nothing else in the response would look
wrong.

Second, every caller-supplied value here reaches a `merge(REGEXP('^bronze_'))`
scan over every bronze database. `grain` and `series` pick SQL expressions and
`scope` lands in a predicate, so the cases below are about refusing input before
it becomes part of a statement — the failure to rule out is one where an
unparseable or over-wide value is carried far enough to become a 500, or a scan
of everything.

The counts themselves are not asserted. The stand's bronze databases hold
whatever its connectors were seeded with, and this suite asserts no metric
values (see `no metric values` in the suite's contract). What is asserted is the
shape, the echoed window, and the refusals.

The 401 half is in `test_gateway.py`, swept over every operation at once.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, PersonaSession, analytics_path

from ..schemas import IngestionIntensityResponse, ProblemDocument

INTENSITY = analytics_path("/v1/ingestion/intensity")


@pytest.mark.security
def test_an_unauthenticated_caller_never_reaches_any_of_this(
    api_client: ApiClient,
) -> None:
    """Proven per operation by `test_gateway.py`, spot-checked here so this
    module carries its own reason for using a session at all."""
    assert api_client.get(INTENSITY).status_code == 401


@pytest.mark.security
def test_intensity_is_refused_without_the_admin_grant(api: ApiClient) -> None:
    """The gate is the whole boundary on this route.

    There is no tenant to fall back on: bronze rows carry none. If this answered
    200 to an ordinary caller, every connector's extraction volume would be
    readable by anyone signed in, and the payload would look entirely normal.
    """
    response = api.get(INTENSITY)
    assert response.status_code == 403, (
        f"ingestion intensity answered {response.status_code} to a caller holding no "
        f"admin grant: {response.text[:300]}"
    )
    problem = response.parse(ProblemDocument)
    assert problem.status == 403
    assert problem.detail, "the refusal carries no detail a caller can act on"


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.security
def test_the_realm_admin_role_is_not_the_admin_grant(
    realm_admin_session: PersonaSession,
) -> None:
    """`require_admin` reads an active `admin` row, never the realm role.

    Worth pinning on a new admin-gated route: wiring the gate to the realm role
    instead would widen it to everyone the IdP calls an admin.
    """
    response = realm_admin_session.client.get(INTENSITY)
    assert response.status_code == 403, (
        f"the realm admin reached ingestion intensity with {response.status_code}: "
        f"{response.text[:300]}"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_default_window_is_resolved_and_echoed(
    admin_operator_session: PersonaSession,
) -> None:
    """A caller may pin neither bound, so the response says what was read.

    The echo is load-bearing for the surface above: it labels the chart's axis
    domain. A response that echoed the REQUEST rather than the resolved window
    would let the chart claim a period it never plotted.
    """
    response = admin_operator_session.client.get(INTENSITY)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    body = response.parse(IngestionIntensityResponse)
    assert body.grain == "15m", "the default grain changed"
    assert body.series == "connector", "an unscoped read should band by connector"
    assert body.scope is None
    assert body.from_ < body.to, f"window is inverted: {body.from_} .. {body.to}"
    assert body.truncated is False, "the default window should not reach the group cap"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_buckets_carry_a_key_and_a_count(
    admin_operator_session: PersonaSession,
) -> None:
    """The row shape the charts pivot on.

    No count is asserted — the stand's bronze content is whatever its connectors
    were seeded with. What must hold is that every point is addressable: a
    bucket, a band, and a non-negative number.
    """
    response = admin_operator_session.client.get(INTENSITY, params={"grain": "1s"})
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    body = response.parse(IngestionIntensityResponse)
    assert body.grain == "1s"
    for point in body.points:
        assert point.bucket, "a bucket with no timestamp cannot be plotted"
        # Zone-less by contract: the reader appends Z. A bucket that arrived
        # already offset would be re-cut into the reader's timezone.
        assert "+" not in point.bucket and not point.bucket.endswith("Z"), (
            f"bucket carries a zone marker: {point.bucket}"
        )
        assert point.key, "a band with no name cannot be coloured or legended"
        assert point.rows >= 0


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_total_read_collapses_to_one_band(
    admin_operator_session: PersonaSession,
) -> None:
    """`series=total` is what makes the long trend answerable.

    Banding a 30-day window by connector would multiply 15-minute buckets by the
    connector count for a chart that sums them back down — and reach the group
    cap on the way.
    """
    response = admin_operator_session.client.get(INTENSITY, params={"series": "total"})
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    body = response.parse(IngestionIntensityResponse)
    assert body.series == "total"
    assert {point.key for point in body.points} <= {"all"}, (
        "a total read returned more than the single band it promises"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_scoped_read_bands_by_stream_and_echoes_the_scope(
    admin_operator_session: PersonaSession,
) -> None:
    """The drill-down contract: scope in, streams out.

    A scope naming no existing database is not an error — a connector that has
    never synced has no rows, and an empty series is the honest answer. What must
    not happen is the scope being ignored and an org-wide read served instead,
    which the echo is here to catch.
    """
    response = admin_operator_session.client.get(
        INTENSITY, params={"scope": "bronze_nonpresent_connector"}
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    body = response.parse(IngestionIntensityResponse)
    assert body.scope == "bronze_nonpresent_connector"
    assert body.series == "stream", "a scoped read should default to banding by stream"
    assert body.points == [], (
        "a scope matching no bronze database returned rows, so it was not applied"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.security
@pytest.mark.parametrize(
    "scope",
    [
        "silver_jira",
        "jira",
        "bronze_jira; DROP TABLE x",
        "bronze_*",
        "bronze_Jira",
        "bronze_jira'",
        "bronze_jira`",
        "../bronze_jira",
    ],
)
def test_a_scope_that_is_not_a_bronze_slug_is_refused(
    admin_operator_session: PersonaSession, scope: str
) -> None:
    """The scope predicate sits inside a merge() scan over every bronze database.

    Validated as a strict slug rather than escaped, so the cases that matter are
    the ones that would either widen the scan (`bronze_*`, a silver database) or
    end the predicate early.
    """
    response = admin_operator_session.client.get(INTENSITY, params={"scope": scope})
    assert response.status_code == 400, (
        f"scope {scope!r} answered {response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 400


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
@pytest.mark.parametrize("grain", ["1m", "INTERVAL 1 DAY", "15", "15M", "day"])
def test_a_grain_outside_the_closed_set_is_refused(
    admin_operator_session: PersonaSession, grain: str
) -> None:
    """`grain` picks a bucketing EXPRESSION, so the set has to be closed.

    An accepted arbitrary value would be an interval expression evaluated inside
    the merge() scan.
    """
    response = admin_operator_session.client.get(INTENSITY, params={"grain": grain})
    assert response.status_code == 400, (
        f"grain {grain!r} answered {response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 400


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_series_outside_the_closed_set_is_refused(
    admin_operator_session: PersonaSession,
) -> None:
    """`series` picks the key COLUMN, for the same reason `grain` is closed."""
    response = admin_operator_session.client.get(INTENSITY, params={"series": "source_database"})
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 400


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
@pytest.mark.parametrize(
    ("params", "why"),
    [
        ({"from": "2026-08-26"}, "a bare date is not an instant"),
        ({"from": "2026-08-26 10:00:00"}, "zone-less, so the instant is ambiguous"),
        ({"from": "yesterday"}, "not parseable at all"),
        (
            {"from": "2026-08-26T11:00:00Z", "to": "2026-08-26T10:00:00Z"},
            "inverted bounds",
        ),
        (
            {"from": "2026-08-26T10:00:00Z", "to": "2026-08-26T10:00:00Z"},
            "an empty window",
        ),
    ],
)
def test_a_malformed_window_is_refused_rather_than_queried(
    admin_operator_session: PersonaSession, params: dict[str, str], why: str
) -> None:
    """The window is parsed before ClickHouse is asked anything.

    Both bounds are caller-supplied and reach a `parseDateTime64BestEffort`
    argument, so the failure to avoid is one where an unparseable value is
    carried far enough to become a 500.
    """
    response = admin_operator_session.client.get(INTENSITY, params=params)
    assert response.status_code == 400, (
        f"{why}: answered {response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 400


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_window_too_wide_for_its_grain_is_refused(
    admin_operator_session: PersonaSession,
) -> None:
    """A day of one-second buckets is 86_400 groups per band.

    Refused rather than truncated: a clipped answer at this size is not a chart
    anybody can read, and the scan behind it is over every bronze database.
    """
    response = admin_operator_session.client.get(
        INTENSITY,
        params={
            "grain": "1s",
            "from": "2026-08-25T12:00:00Z",
            "to": "2026-08-26T12:00:00Z",
        },
    )
    assert response.status_code == 400, (
        f"a 24h window at 1s grain answered {response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 400


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_an_offset_bound_is_accepted_and_normalised(
    admin_operator_session: PersonaSession,
) -> None:
    """RFC 3339 includes offsets, and a hand-edited link may carry one.

    The echoed window must name the same INSTANT, not the same digits — a bound
    silently read as UTC would shift the chart by the offset.
    """
    response = admin_operator_session.client.get(
        INTENSITY,
        params={
            "from": "2026-08-26T09:00:00+02:00",
            "to": "2026-08-26T10:00:00+02:00",
        },
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    body = response.parse(IngestionIntensityResponse)
    assert body.from_.startswith("2026-08-26T07:00:00"), (
        f"an offset `from` was not normalised to UTC: {body.from_}"
    )
    assert body.to.startswith("2026-08-26T08:00:00"), (
        f"an offset `to` was not normalised to UTC: {body.to}"
    )

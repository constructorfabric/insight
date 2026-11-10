from __future__ import annotations

from collections import defaultdict
from datetime import date, timedelta

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, analytics_path

from ..schemas import IngestionIntensityResponse
from . import query_window

INTENSITY = analytics_path("/v1/ingestion/intensity")

pytestmark = pytest.mark.reliability


def _seeded_window(manifest: Manifest) -> dict[str, str]:
    start, end = query_window(manifest, max_days=398)
    after_end = date.fromisoformat(end) + timedelta(days=1)
    return {"from": f"{start}T00:00:00Z", "to": f"{after_end.isoformat()}T00:00:00Z"}


def _intensity(client: ApiClient, **params: str) -> IngestionIntensityResponse:
    response = client.get(INTENSITY, params=params)
    assert response.status_code == 200, f"{params}: {response.status_code} {response.text[:300]}"
    body = response.parse(IngestionIntensityResponse)
    assert body.truncated is False, f"{params} reached the group cap, so its sums are partial"
    return body


def _per_bucket(body: IngestionIntensityResponse) -> dict[str, int]:
    sums: dict[str, int] = defaultdict(int)
    for point in body.points:
        sums[point.bucket] += point.rows
    return dict(sums)


def _band(body: IngestionIntensityResponse, key: str) -> dict[str, int]:
    return {point.bucket: point.rows for point in body.points if point.key == key}


@pytest.mark.requires_seed("admin_operator")
def test_an_operator_drilling_from_the_total_to_a_stream_sees_the_same_rows(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    admin = admin_operator_session.client
    window = _seeded_window(stand_manifest)

    by_connector = _intensity(admin, series="connector", **window)
    if not by_connector.points:
        pytest.skip(f"no bronze rows were extracted in {window['from']}..{window['to']}")

    total = _intensity(admin, series="total", **window)
    assert {point.key for point in total.points} == {"all"}
    assert _band(total, "all") == _per_bucket(by_connector)

    for connector in sorted({point.key for point in by_connector.points}):
        scope = f"bronze_{connector}"
        by_stream = _intensity(admin, scope=scope, **window)

        assert (by_stream.scope, by_stream.series) == (scope, "stream")
        assert by_stream.points, f"{scope} lost every row the org-wide read gave it"
        assert _per_bucket(by_stream) == _band(by_connector, connector), scope

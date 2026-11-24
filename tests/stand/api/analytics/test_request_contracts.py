"""Body contracts every analytics route shares, asserted once over all of them.

    body-taking routes   415 when the body arrives as text/plain

This is a property of the ROUTE TABLE rather than of any handler: every body
extractor refuses on media type before it parses. Stating
each one per endpoint is a pile of near-identical tests and a list that silently
stops matching the router. Here the list IS the assertion — a route added to
`operations.py` and not to the table below is visible as an absence in one
place.

Ordering is the substance. A 415 that arrived as a 422 would mean the body was
read before its media type was checked. This is not visible from the status
code alone, which is why the tables pin the code that must come FIRST.

Worth asserting through a real gateway specifically: a proxy that rewrote or
dropped `Content-Type` would turn every 415 below into a 422 or a 2xx, and an
in-process rig cannot see that happen.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, analytics_path
from insight_stand.api import JsonValue

from .. import scratch

# Quality vector of this module's tests.
pytestmark = pytest.mark.reliability

#: Every route that reads a body. Ids are well-formed-but-unknown on purpose:
#: the path must parse, so that what the response reports is the media type and
#: not the segment.
BODY_ROUTES: tuple[tuple[str, str], ...] = (
    ("POST", "/v1/metric-results"),
    ("POST", "/v1/reports/preview"),
    ("POST", "/v1/reports/export"),
    ("POST", "/v1/metric-drilldown"),
    ("POST", "/v1/metric-drilldown/export"),
    ("POST", "/v1/metrics"),
    ("PUT", f"/v1/metrics/{scratch.UNKNOWN_METRIC_KEY}"),
    ("POST", "/v1/metrics/import"),
    ("POST", "/v1/usage/events"),
    ("POST", "/v1/feedback"),
)


def _id(value: str) -> str:
    return value


@pytest.mark.parametrize(("method", "suffix"), BODY_ROUTES, ids=_id)
def test_a_body_with_the_wrong_media_type_is_415(api: ApiClient, method: str, suffix: str) -> None:
    """Refused on `Content-Type`, not parsed and then judged.

    The body is valid JSON, so anything but 415 means the extractor read it
    before checking how it was labelled — 422 would say it was parsed against
    the schema, 2xx that it was accepted outright.
    """
    response = api.request(
        method, analytics_path(suffix), content="{}", headers={"Content-Type": "text/plain"}
    )
    assert response.status_code == 415, (
        f"{method} {suffix} answered {response.status_code} to a text/plain body: "
        f"{response.text[:300]}"
    )


#: Well-formed JSON that is not the request type. Every body route answers
#: Axum's own 422 with a plain-text envelope rather than the canonical 400 its
#: spec declares — the extractor's default, never chosen. That is #1670, now
#: uniform: the `CanonicalJson` extractor went with the catalog and admin
#: surfaces that used it, so closing the bug means reintroducing one here
#: rather than pointing these routes at an extractor that already exists.
#:
#: Asserted as it BEHAVES, with the intended contract named — a strict xfail
#: would say the same thing, but this way the suite keeps telling you what a
#: caller actually receives today.
LEGACY_422 = 422

OFF_SCHEMA_ROUTES: tuple[tuple[str, str, int], ...] = (
    ("POST", "/v1/metric-results", LEGACY_422),
    ("POST", "/v1/reports/preview", LEGACY_422),
    ("POST", "/v1/reports/export", LEGACY_422),
    ("POST", "/v1/metric-drilldown", LEGACY_422),
    ("POST", "/v1/metric-drilldown/export", LEGACY_422),
    ("POST", "/v1/metrics", LEGACY_422),
    ("PUT", f"/v1/metrics/{scratch.UNKNOWN_METRIC_KEY}", LEGACY_422),
    ("POST", "/v1/metrics/import", LEGACY_422),
)

#: A JSON STRING where every one of these routes declares an object. Two
#: nearby shapes do NOT work:
#:
#:   {"stand": …}  an unknown KEY is not off-schema for a type whose fields are
#:                 all optional — serde ignores what it does not recognise, the
#:                 request is a valid all-default one, and the handler runs.
#:   []            neither is an ARRAY. serde's derived struct deserializer
#:                 accepts a sequence as well as a map, matching fields
#:                 positionally, so an empty array is again all-defaults.
#:
#: A scalar is the shape that cannot become a struct either way: there is no
#: `visit_str` on a struct visitor, so it fails at the extractor whatever the
#: fields are. Route-independent, which is what this table needs.
OFF_SCHEMA_BODY: JsonValue = "not the request type"

#: `POST /v1/usage/events` is deliberately absent: it is the only
#: body route that declares no 400, so there is no gap between declaration and
#: behaviour for this table to pin.


@pytest.mark.parametrize(
    ("method", "suffix", "expected"),
    OFF_SCHEMA_ROUTES,
    ids=[f"{m} {s}" for m, s, _ in OFF_SCHEMA_ROUTES],
)
def test_an_off_schema_body_is_refused_with_the_code_the_extractor_chooses(
    api: ApiClient, method: str, suffix: str, expected: int
) -> None:
    """A body that is valid JSON but not the declared request type.

    Every route here DECLARES 400 — `.standard_errors` puts it on all of them —
    and every one answers 422 instead, because plain `axum::Json` rejects with
    its own envelope before any of this product's error machinery runs.

    Pinning what it does rather than what it should do is what makes it
    actionable: a route that changes extractor changes this table, and nothing
    else has to notice.
    """
    response = api.request(method, analytics_path(suffix), json_body=OFF_SCHEMA_BODY)
    assert response.status_code == expected, (
        f"{method} {suffix} answered {response.status_code} to an off-schema body, "
        f"expected {expected} (axum::Json, #1670): {response.text[:300]}"
    )

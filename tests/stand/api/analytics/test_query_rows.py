"""`POST /v1/query/rows` — the rows one input of a metric's computation folded.

    POST /v1/query/rows  200 · 400 a cursor this question did not issue
                         403 outside the visible set · 404 unknown metric

The 401 half is in `test_gateway.py` and the 415 half in
`test_request_contracts.py`, both swept over every operation at once.

The page walk is the case worth having here: a position is opaque, bound to the
question that issued it, and the only thing a caller can do with it is ask for
the next page. So one test resumes a genuine cursor and one shows every way a
cursor can fail to be this question's, each refused before any rows are read.

Ordering is the other: the server decides it, so the tests read it back off the
wire rather than re-deriving it, and the walk over a sorted question is checked
against an unsorted read of the same rows — an order that dropped or repeated a
row would still look ordered on any single page.
"""

from __future__ import annotations

import base64
import json
from collections import Counter

import pytest
from insight_stand import ApiClient, Manifest, analytics_path
from insight_stand.api import JsonValue

from ..schemas import ProblemDocument
from ..schemas.analytics import RowColumn, RowsResponse
from . import query_window

QUERY_ROWS = analytics_path("/v1/query/rows")

#: Composes one input, so a page needs no `input` to say which to read.
GIT_COMMITS = "git.commits"

#: Well-formed and carried by no definition, so a refusal is the catalogue's and
#: not a spelling rejection dressed as one.
UNKNOWN_METRIC = "stand.does_not_exist"

#: One row a page, so the walk issues a cursor from as little seeded evidence as
#: possible.
_PAGE_SIZE = 1


def _request(
    manifest: Manifest,
    metric: str,
    subject_id: str,
    *,
    cursor: str | None = None,
    page_size: int = _PAGE_SIZE,
    sort: dict[str, JsonValue] | None = None,
) -> dict[str, JsonValue]:
    start, end = query_window(manifest)
    request: dict[str, JsonValue] = {
        "metric": metric,
        "subjects": {"type": "persons", "ids": [subject_id]},
        "time": {"from": start, "to": end},
        "page_size": page_size,
    }
    if cursor is not None:
        request["cursor"] = cursor
    if sort is not None:
        request["sort"] = sort
    return request


def _encoded(payload: bytes) -> str:
    """Url-safe unpadded base64, the encoding `rows/cursor.rs` reads a position from."""
    return base64.urlsafe_b64encode(payload).decode("ascii").rstrip("=")


def _envelope(**fields: JsonValue) -> str:
    """A cursor-shaped position, encoded the way the service encodes its own."""
    return _encoded(json.dumps(fields).encode("utf-8"))


#: Each one is well-formed up to the single property it breaks, so a refusal is
#: attributable to that property rather than to a string the service would never
#: have produced.
UNUSABLE_CURSORS: tuple[tuple[str, str], ...] = (
    ("not base64", "@@not-a-cursor@@"),
    ("base64 of something that is not an envelope", _encoded(b"not an envelope")),
    (
        "an unsupported envelope version",
        _envelope(v=2, fp="0" * 64, snap="00000000-0000-0000-0000-000000000000", epoch=0, key=[]),
    ),
    (
        "issued for another question",
        _envelope(v=1, fp="0" * 64, snap="00000000-0000-0000-0000-000000000000", epoch=0, key=[]),
    ),
)


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_rows_resumes_the_page_its_own_cursor_names(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """The second page describes the same columns as the first and repeats none of its rows."""
    person = stand_manifest.fixture("dev_lead")

    response = api.post(QUERY_ROWS, json_body=_request(stand_manifest, GIT_COMMITS, person.uuid))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    first = response.parse(RowsResponse)
    assert first.metric == GIT_COMMITS
    assert first.input, "a page reported no input of the computation it read"
    assert first.columns, "a page described no columns"
    assert len(first.rows) <= _PAGE_SIZE
    assert all(len(row) == len(first.columns) for row in first.rows), (
        f"a row carries a different number of values than the {len(first.columns)} columns "
        f"describe: {first.rows}"
    )

    assert first.next_cursor is not None, (
        f"the seeded {GIT_COMMITS} evidence for {person.email} fit one page at page_size="
        f"{_PAGE_SIZE}, so no cursor was issued and the walk cannot be exercised"
    )

    resumed = api.post(
        QUERY_ROWS,
        json_body=_request(stand_manifest, GIT_COMMITS, person.uuid, cursor=first.next_cursor),
    )
    assert resumed.status_code == 200, (
        f"resuming a cursor the service issued answered {resumed.status_code}: {resumed.text[:300]}"
    )

    second = resumed.parse(RowsResponse)
    assert second.columns == first.columns
    assert second.input == first.input
    assert len(second.rows) <= _PAGE_SIZE
    assert second.next_cursor != first.next_cursor, "the walk reissued the position it resumed from"

    seen = {tuple(row) for row in first.rows}
    assert not seen.intersection(tuple(row) for row in second.rows), (
        "the second page repeats a row the first already reported"
    )


@pytest.mark.parametrize(
    ("label", "cursor"), UNUSABLE_CURSORS, ids=[c[0] for c in UNUSABLE_CURSORS]
)
@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_rows_refuses_a_cursor_it_did_not_issue(
    api: ApiClient, stand_manifest: Manifest, label: str, cursor: str
) -> None:
    """A position that is not this question's is refused, never resumed against other rows."""
    person = stand_manifest.fixture("dev_lead")

    response = api.post(
        QUERY_ROWS,
        json_body=_request(stand_manifest, GIT_COMMITS, person.uuid, cursor=cursor),
    )
    assert response.status_code == 400, (
        f"a cursor that is {label} answered {response.status_code}, expected 400: "
        f"{response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 400


@pytest.mark.requires_seed("sales_ic")
@pytest.mark.security
def test_query_rows_refuses_a_person_out_of_scope(api: ApiClient, stand_manifest: Manifest) -> None:
    """The rows behind a hidden person's value are refused exactly as the value is."""
    outsider = stand_manifest.fixture("sales_ic")

    response = api.post(QUERY_ROWS, json_body=_request(stand_manifest, GIT_COMMITS, outsider.uuid))
    assert response.status_code == 403, (
        f"paging {outsider.email}, who is outside the lead's scope, answered "
        f"{response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 403


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_rows_reports_an_unknown_metric_as_not_found(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A key the definitions do not carry is refused before any input is resolved."""
    person = stand_manifest.fixture("dev_lead")

    response = api.post(QUERY_ROWS, json_body=_request(stand_manifest, UNKNOWN_METRIC, person.uuid))
    assert response.status_code == 404, (
        f"an unknown metric answered {response.status_code}, expected 404: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 404


#: Reported by every metric's page, so the ordering tests need no per-metric
#: knowledge of which columns a dataset happens to carry.
_SORT_COLUMN = "date"

#: The widest page the API will answer, so a baseline read walks as few pages as
#: the seeded evidence allows.
_WIDEST_PAGE = 250

#: A walk that never ends is a defect in the walk, not evidence about the stand.
_MAX_PAGES = 40


def _column_index(columns: list[RowColumn], key: str) -> int:
    """Where a column sits in every row, which the page describes once."""
    for index, column in enumerate(columns):
        if column.key == key:
            return index
    raise AssertionError(
        f"a page of {GIT_COMMITS} reports no `{key}` column to order by; it reports "
        f"{[column.key for column in columns]}"
    )


def _walk(
    api: ApiClient,
    manifest: Manifest,
    person_id: str,
    *,
    page_size: int,
    sort: dict[str, JsonValue] | None = None,
) -> tuple[list[tuple[JsonValue, ...]], list[RowColumn]]:
    """Every row of a question, read one page at a time the way a caller reads it."""
    rows: list[tuple[JsonValue, ...]] = []
    columns: list[RowColumn] = []
    cursor: str | None = None

    for page_number in range(_MAX_PAGES):
        response = api.post(
            QUERY_ROWS,
            json_body=_request(
                manifest,
                GIT_COMMITS,
                person_id,
                cursor=cursor,
                page_size=page_size,
                sort=sort,
            ),
        )
        assert response.status_code == 200, (
            f"page {page_number} of a walk over sort={sort} answered "
            f"{response.status_code}: {response.text[:300]}"
        )

        page = response.parse(RowsResponse)
        if page_number == 0:
            columns = page.columns
        assert page.columns == columns, "a page of the same walk described other columns"

        rows.extend(tuple(row) for row in page.rows)
        cursor = page.next_cursor
        if cursor is None:
            return rows, columns

    raise AssertionError(
        f"a walk over sort={sort} at page_size={page_size} did not end within {_MAX_PAGES} pages"
    )


def _assert_reported_in_order(values: list[JsonValue], direction: str, described: str) -> None:
    """Non-strict: rows tying on the sorted column are still both reported."""
    present = [value for value in values if value is not None]
    assert values[: len(present)] == present, (
        f"{described} reported a row carrying no {_SORT_COLUMN} before one that carries a "
        f"value; rows carrying none are reported last either way: {values}"
    )
    assert present == sorted(present, reverse=direction == "desc"), (
        f"{described} did not report its rows in {direction} order of {_SORT_COLUMN}: {present}"
    )


@pytest.mark.parametrize("direction", ["asc", "desc"])
@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_rows_reports_a_sorted_page_in_the_order_it_was_asked_for(
    api: ApiClient, stand_manifest: Manifest, direction: str
) -> None:
    """The server decides the order, so a page arrives already in it."""
    person = stand_manifest.fixture("dev_lead")

    response = api.post(
        QUERY_ROWS,
        json_body=_request(
            stand_manifest,
            GIT_COMMITS,
            person.uuid,
            page_size=_WIDEST_PAGE,
            sort={"column": _SORT_COLUMN, "direction": direction},
        ),
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    page = response.parse(RowsResponse)
    assert page.rows, (
        f"no {GIT_COMMITS} evidence is seeded for {person.email}, so there is no order to read"
    )

    sorted_at = _column_index(page.columns, _SORT_COLUMN)
    _assert_reported_in_order(
        [row[sorted_at] for row in page.rows], direction, f"a page sorted {direction}"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_rows_walks_a_sorted_question_over_exactly_the_rows_it_holds(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """Ordering rearranges a question's rows; it neither drops nor repeats one."""
    person = stand_manifest.fixture("dev_lead")

    unsorted, columns = _walk(api, stand_manifest, person.uuid, page_size=_WIDEST_PAGE)
    assert len(unsorted) >= 2, (
        f"the seeded {GIT_COMMITS} evidence for {person.email} is {len(unsorted)} row(s), so no "
        f"page boundary can be crossed and the walk cannot be exercised"
    )

    # Small enough that the walk crosses at least one boundary whatever is seeded,
    # and inside the page cap however much is.
    page_size = min(250, max(1, len(unsorted) // 3))
    walked, sorted_columns = _walk(
        api,
        stand_manifest,
        person.uuid,
        page_size=page_size,
        sort={"column": _SORT_COLUMN, "direction": "desc"},
    )

    assert sorted_columns == columns, "ordering a question changed the columns it reports"
    assert Counter(walked) == Counter(unsorted), (
        f"walking {GIT_COMMITS} sorted at page_size={page_size} reported "
        f"{len(walked)} rows against {len(unsorted)} read unsorted: the walk "
        f"{'repeated' if len(walked) > len(unsorted) else 'lost'} rows across a page boundary"
    )

    sorted_at = _column_index(sorted_columns, _SORT_COLUMN)
    _assert_reported_in_order(
        [row[sorted_at] for row in walked], "desc", "a walk over a sorted question"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_query_rows_refuses_a_column_the_page_does_not_report(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A page can only be ordered by something it reports, and says so."""
    person = stand_manifest.fixture("dev_lead")

    response = api.post(
        QUERY_ROWS,
        json_body=_request(
            stand_manifest,
            GIT_COMMITS,
            person.uuid,
            sort={"column": "not_a_column", "direction": "asc"},
        ),
    )
    assert response.status_code == 400, (
        f"ordering by a column no page reports answered {response.status_code}, expected 400: "
        f"{response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 400

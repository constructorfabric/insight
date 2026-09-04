"""`POST /v1/query` — the query contract over the declared datasets.

    POST /v1/query   200 · 400 (unknown dataset, undeclared dimension, a limit
                              over the cap, a body naming a tenant)

Deployed-path because no unit test reaches these: the session's tenant becoming
the scan's leading predicate, the compiled statement being SQL a real ClickHouse
accepts, and the gold relation the declaration binds having been built.

No expected number is written into this file; the 200 cases reconcile two
independent queries against each other.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, ApiResponse, Manifest, analytics_path
from insight_stand.api import JsonValue

from ..schemas import ProblemDocument
from ..schemas.analytics import QueryAnswer
from . import query_window

QUERY = analytics_path("/v1/query")

#: The one dataset this build declares. A key the service does not carry is the
#: other half of the pair, and it is well-formed so a refusal is the
#: declaration's rather than a spelling rejection dressed as one.
GIT_COMMITS = "git_commits"
UNKNOWN_DATASET = "stand_does_not_exist"


def _query(
    manifest: Manifest, *, grain: str | None = None, **overrides: JsonValue
) -> dict[str, JsonValue]:
    start, end = query_window(manifest)
    time: dict[str, JsonValue] = {"from": start, "to": end}
    if grain is not None:
        time["grain"] = grain

    body: dict[str, JsonValue] = {
        "dataset": GIT_COMMITS,
        "aggregates": [{"name": "commits", "fn": "count"}],
        "time": time,
    }
    body.update(overrides)
    return body


def _post(api: ApiClient, body: dict[str, JsonValue]) -> ApiResponse:
    return api.post(QUERY, json_body=body)


def _violated_fields(response: ApiResponse) -> list[str]:
    """Every request field the refusal names, so a caller could repair the query."""
    violations = response.parse(ProblemDocument).context.get("field_violations")
    assert isinstance(violations, list), (
        f"a refusal must carry field_violations: {response.text[:300]}"
    )

    fields: list[str] = []
    for violation in violations:
        assert isinstance(violation, dict), f"a violation must be an object: {violation}"
        field = violation.get("field")
        assert isinstance(field, str), f"a violation must name a field: {violation}"
        fields.append(field)
    return fields


def _column_names(answer: QueryAnswer) -> list[str]:
    return [column.name for column in answer.columns]


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_a_bucketed_query_answers_a_typed_table_whose_rows_match_its_columns(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    body = _query(
        stand_manifest,
        grain="month",
        group_by=[{"axis": "dimension", "field": "repository"}, {"axis": "time"}],
        order=[{"by": "commits", "dir": "desc"}],
        limit=200,
    )

    response = _post(api, body)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    answer = response.parse(QueryAnswer)
    assert _column_names(answer) == ["repository", "time", "commits"], (
        f"the answer's columns are not the ones the query asked for: {answer.columns}"
    )
    assert [column.kind for column in answer.columns] == ["dimension", "bucket", "aggregate"]
    for row in answer.rows:
        assert len(row) == len(answer.columns), (
            f"a row carries {len(row)} values for {len(answer.columns)} columns: {row}"
        )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_the_buckets_of_a_grouped_count_sum_to_the_same_window_folded_whole(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """A bucketed and an ungrouped count read the same rows, so they must agree
    whatever the stand was seeded with."""
    whole = _post(api, _query(stand_manifest))
    assert whole.status_code == 200, f"status={whole.status_code} {whole.text[:300]}"
    total_rows = whole.parse(QueryAnswer).rows
    assert len(total_rows) == 1, f"an ungrouped query answers one row, got {total_rows}"
    total = total_rows[0][0]

    bucketed_body = _query(stand_manifest, grain="month", group_by=[{"axis": "time"}], limit=10_000)
    bucketed = _post(api, bucketed_body)
    assert bucketed.status_code == 200, f"status={bucketed.status_code} {bucketed.text[:300]}"

    summed = sum(row[1] for row in bucketed.parse(QueryAnswer).rows)
    assert summed == total, (
        f"the monthly buckets sum to {summed} and the same window folded whole "
        f"is {total} — grouping changed which rows were counted"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.versatility
@pytest.mark.parametrize("grain", ["day", "week", "month"])
def test_every_declared_grain_answers_a_bucket_column(
    api: ApiClient, stand_manifest: Manifest, grain: str
) -> None:
    """Shape only: bucket boundaries are the compiler's rendered-SQL goldens' job."""
    body = _query(stand_manifest, grain=grain, group_by=[{"axis": "time"}], limit=10_000)

    response = _post(api, body)
    assert response.status_code == 200, (
        f"grain={grain} status={response.status_code} {response.text[:300]}"
    )
    answer = response.parse(QueryAnswer)
    assert _column_names(answer) == ["time", "commits"]


@pytest.mark.reliability
@pytest.mark.parametrize(
    ("label", "overrides", "field"),
    [
        ("a dataset this build does not declare", {"dataset": UNKNOWN_DATASET}, "dataset"),
        (
            "a dimension the dataset does not declare",
            {"group_by": [{"axis": "dimension", "field": "branch"}]},
            "group_by[0].field",
        ),
        ("a row ceiling over the cap", {"limit": 1_000_000}, "limit"),
        (
            "an aggregate over a column that is not a measurable",
            {"aggregates": [{"name": "added", "fn": "sum", "field": "message"}]},
            "aggregates[0].field",
        ),
    ],
)
def test_a_query_the_dataset_cannot_answer_is_refused_naming_the_field(
    api: ApiClient,
    stand_manifest: Manifest,
    label: str,
    overrides: dict[str, JsonValue],
    field: str,
) -> None:
    """400 rather than 404 throughout, the unknown dataset included: this is a
    statement about the request, not about a missing resource."""
    body = _query(stand_manifest)
    body.update(overrides)

    response = _post(api, body)

    assert response.status_code == 400, (
        f"{label}: status={response.status_code} {response.text[:300]}"
    )
    assert field in _violated_fields(response), (
        f"{label}: the refusal did not name {field!r}: {response.text[:300]}"
    )


@pytest.mark.reliability
@pytest.mark.parametrize(
    ("label", "overrides"),
    [
        (
            "an operand belonging to another filter operator",
            {"filters": [{"field": "source", "op": "eq", "values": ["github"]}]},
        ),
        (
            "a fold naming a column its variant does not read",
            {"aggregates": [{"name": "commits", "fn": "count", "field": "lines_added"}]},
        ),
        (
            "a fold missing the column its variant reads",
            {"aggregates": [{"name": "added", "fn": "sum"}]},
        ),
        (
            "a group axis that names no axis",
            {"group_by": [{"dimension": "repository"}]},
        ),
    ],
)
def test_an_operand_from_another_variant_is_refused_before_anything_validates(
    api: ApiClient, stand_manifest: Manifest, label: str, overrides: dict[str, JsonValue]
) -> None:
    """Each body pairs a tag with an operand another variant takes, so the refusal
    is the body extractor's rather than any dataset rule's."""
    body = _query(stand_manifest)
    body.update(overrides)

    response = _post(api, body)

    assert response.status_code in {400, 422}, (
        f"{label}: status={response.status_code} {response.text[:300]}"
    )


@pytest.mark.security
def test_a_query_cannot_name_a_tenant_of_its_own(api: ApiClient, stand_manifest: Manifest) -> None:
    """The contract refuses every key it does not declare, so a query has no lever
    to scope itself to another tenant."""
    body = _query(stand_manifest)
    body["tenant_id"] = "00000000-0000-0000-0000-000000000000"

    response = _post(api, body)

    assert response.status_code in {400, 422}, (
        f"an off-contract key was accepted: status={response.status_code} {response.text[:300]}"
    )

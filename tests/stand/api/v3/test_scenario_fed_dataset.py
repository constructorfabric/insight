from __future__ import annotations

from typing import Final

import pytest
from insight_stand import ADMIN_OPERATOR_FIXTURE, ApiClient, JsonValue, scratch_name, v3_path

from ..schemas.v3 import Problem
from ..schemas.v3_untyped import Dataset, DatasetDependents, DatasetPage, DatasetRecords
from .fed_records import DECLARATION_FIELDS, RECORDS, Ingest, _create, _declare, _expect, _run
from .fed_records import ingest as ingest
from .fed_records import removals as removals

_INGEST_BODY_LIMIT_BYTES: Final[int] = 1_048_576


@pytest.mark.reliability
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE)
def test_records_pushed_into_a_dataset_feed_a_metric_until_the_dataset_is_taken_away(
    v3_admin: ApiClient, ingest: Ingest, removals: list[str]
) -> None:
    dataset = _declare(v3_admin, removals)
    dataset_path = v3_path(f"/v1/datasets/{dataset.name}")
    records_path = f"{dataset_path}/records"

    listed = _expect(v3_admin.get(v3_path("/v1/datasets"), params={"q": dataset.name}), 200).parse(
        DatasetPage
    )
    assert listed.names == [dataset.name]
    assert (listed.items[0].title, listed.items[0].fields) == (
        dataset.name,
        len(DECLARATION_FIELDS),
    )

    opened = _expect(v3_admin.get(dataset_path), 200).parse(Dataset)
    assert opened == dataset

    ingest.feed(dataset.name, RECORDS)

    by_day = sorted(RECORDS, key=lambda record: record.day)
    for direction, expected in (("asc", by_day), ("desc", by_day[::-1])):
        page = _expect(
            v3_admin.get(records_path, params={"order_by": "day", "direction": direction}), 200
        ).parse(DatasetRecords)
        assert page.total == len(RECORDS)
        assert [record.raw_data for record in page.records] == [record.raw() for record in expected]

    metric = _create(
        v3_admin,
        removals,
        "metrics",
        {
            "dataset": dataset.name,
            "fields": [
                {"field": "lines", "type": "int", "agg": "sum", "as_name": "lines"},
                {"agg": "count", "type": "int", "as_name": "records"},
            ],
        },
    )
    metric_path = v3_path(f"/v1/metrics/{metric.id}")

    run = _run(v3_admin, f"{metric_path}/run")
    assert run.columns == ["lines", "records"]
    assert run.rows == [[sum(record.lines for record in RECORDS), len(RECORDS)]]

    dependents = _expect(v3_admin.get(f"{dataset_path}/dependents"), 200).parse(DatasetDependents)
    assert dependents.metrics == [metric.id]

    in_use = _expect(v3_admin.delete(dataset_path), 409).parse(Problem)
    assert [violation["type"] for violation in in_use.context["violations"]] == ["in_use"]

    _expect(v3_admin.delete(metric_path), 204)
    _expect(v3_admin.delete(dataset_path), 204)

    _expect(v3_admin.get(records_path), 404).parse(Problem)
    _expect(v3_admin.get(dataset_path), 404).parse(Problem)
    _expect(ingest.post({"dataset": dataset.name, "raw_data": RECORDS[0].raw()}, ingest.token), 404)


@pytest.mark.security
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE)
def test_a_record_without_the_instance_token_is_refused_and_never_stored(
    v3_admin: ApiClient, ingest: Ingest, scratch_v3_dataset: Dataset
) -> None:
    body: JsonValue = {
        "dataset": scratch_v3_dataset.name,
        "raw_data": {"day": RECORDS[0].day, "lines": RECORDS[0].lines},
    }

    for token in (None, f"{ingest.token}-not-this-one"):
        refused = _expect(ingest.post(body, token), 401).parse(Problem)
        assert refused.context["reason"] == "INVALID_INGEST_TOKEN"

    page = _expect(
        v3_admin.get(v3_path(f"/v1/datasets/{scratch_v3_dataset.name}/records")), 200
    ).parse(DatasetRecords)
    assert page.total == 0


@pytest.mark.reliability
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE)
def test_ingest_refuses_an_undeclared_dataset_and_an_oversized_record(
    v3_admin: ApiClient, ingest: Ingest, scratch_v3_dataset: Dataset
) -> None:
    undeclared = scratch_name("undeclared")
    record: dict[str, JsonValue] = {"day": RECORDS[0].day, "lines": RECORDS[0].lines}

    _expect(ingest.post({"dataset": undeclared, "raw_data": record}, ingest.token), 404).parse(
        Problem
    )
    catalogue = _expect(v3_admin.get(v3_path("/v1/datasets"), params={"q": undeclared}), 200).parse(
        DatasetPage
    )
    assert catalogue.names == []

    oversized = {**record, "padding": "x" * _INGEST_BODY_LIMIT_BYTES}
    _expect(
        ingest.post({"dataset": scratch_v3_dataset.name, "raw_data": oversized}, ingest.token),
        413,
    )

    page = _expect(
        v3_admin.get(v3_path(f"/v1/datasets/{scratch_v3_dataset.name}/records")), 200
    ).parse(DatasetRecords)
    assert page.total == 0

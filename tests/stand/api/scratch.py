"""Scratch resources: create through the API, delete before the test ends.

The stand is seeded once per run and is read-only by contract — reset by volume
teardown, never by TRUNCATE. Covering the write half of the API surface needs an
exception to that, and this is its exact shape:

1. A test may create rows **through the API**, and must delete them. Never a
   database connection: that would hand every test a back door around the
   deployed path, which is the only thing this suite exists to exercise.
2. Every created row carries `SCRATCH_PREFIX` in its name.
3. The metric **catalog** is out of bounds. It is the metric-coverage gate's
   universe, and a stand suite has no business editing it.
4. Teardown deletes are best-effort — a delete-case test has already removed its
   row, so a 404 there is expected rather than a failure.
5. The correction journal cannot be deleted from, so its rows are made
   recognisable instead: every correction goes under `SCRATCH_SOURCE_TYPE` /
   `SCRATCH_SOURCE_ID`, the pair the seed preflight exempts. One written under
   any other connector blocks the next seed of the stand — which is what a
   `merge` or `detach` happy-path would write, so those stay out of bounds.

Rule 2 exists to make rule 1 checkable. Every name is registered here, and
`conftest.py`'s session-scoped detector fails the run if any survives it. The
in-process rig needs no such check because it discards its whole stack; a stand
persists between runs, so a leak silently changes what the NEXT run sees. That
is the one place this suite should be stricter than the rig.
"""

from __future__ import annotations

import uuid
from collections.abc import Sequence
from dataclasses import dataclass
from typing import Final

from insight_stand import (
    ANALYTICS_PREFIX,
    RUN_TAG,
    SCRATCH_PREFIX,
    SCRATCH_SOURCE_ID,
    SCRATCH_SOURCE_TYPE,
    ApiClient,
    QueryParams,
    analytics_path,
    identity_path,
    issued_names,
    scratch_name,
    v3_path,
)

from .schemas import CustomMetric, SavedQuery
from .schemas.v3 import RuleResponse
from .schemas.v3_untyped import (
    CreatedId,
    DashboardDefinition,
    Dataset,
    Folder,
    MetricDefinition,
    WidgetDefinition,
)

# The naming and the connector instance now live in `insight_stand`, so the UI
# journeys can file a correction under the same pair without importing across
# suites. Re-exported here because every existing caller reads them from this
# module, and because the issued-name registry must stay single.

#: SQL the query gate accepts (a single `SELECT … FROM db.table`) that executes
#: deterministically on ANY ClickHouse — `system.one` has exactly one row — so
#: `POST /v1/queries/{id}/run` drives the real engine end to end.
SCRATCH_QUERY_REF: Final[str] = "SELECT 1 AS one FROM system.one"

#: A well-formed v7 UUID nothing claims, for the unknown-id 404 cases.
UNKNOWN_ID: Final[str] = "01900000-0000-7000-8000-000000000000"

#: A second one, for the routes that need two persons told apart. Merge
#: refuses a person named as both sides before it checks anything else, so a
#: single id would prove the validator rather than the gate.
OTHER_UNKNOWN_ID: Final[str] = "01900000-0000-7000-8000-0000000000ff"

#: Not a UUID, for the path-parse 400 cases: every `{id}` route binds
#: `Path<Uuid>`, whose deserialization failure is a 400 raised before any
#: handler logic runs.
NON_UUID: Final[str] = "not-a-uuid"

#: A well-formed dotted metric_key nothing claims, for the unknown-key 404 cases.
#: NOT a UUID: `/v1/metrics/{metric_key}` binds `Path<String>`, so an unknown key
#: is a handler 404, never a path-parse 400.
UNKNOWN_METRIC_KEY: Final[str] = "example.absent_metric"

#: Observation SQL that the single-SELECT gate accepts and the author-time column
#: probe passes: one read emitting every observation-contract column, wrapped by
#: the compiler as `FROM (<sql>)`. Self-contained so it runs on any ClickHouse —
#: `system.one` has exactly one row — and carries only synthetic placeholder data.
SCRATCH_OBSERVATION_SQL: Final[str] = (
    "SELECT "
    "generateUUIDv4() AS tenant_id, "
    "'scratch' AS source_key, "
    "'person' AS entity_type, "
    "'' AS entity_id, "
    "toDate('2026-01-01') AS metric_date, "
    "'events' AS measure_key, "
    "now() AS observed_at, "
    "toFloat64(0) AS value, "
    "'' AS subject_key, "
    "map('bucket', 'default') AS dimensions "
    "FROM system.one"
)


@dataclass(frozen=True)
class _Tracked:
    listing: str
    id_field: str
    value: str
    key: str
    params: QueryParams | None


#: Rows this session created that have NO name to namespace — a person-role
#: assignment and a visibility grant are identified only by their id — so the
#: sweep looks them up in their listing by id.
_CREATED_IDS: list[_Tracked] = []


def tracked_ids() -> tuple[str, ...]:
    return tuple(entry.value for entry in _CREATED_IDS)


def track(
    listing_path: str,
    id_field: str,
    value: object,
    *,
    key: str = "items",
    params: QueryParams | None = None,
) -> str:
    """Register a nameless created row, and return its id.

    `create_*` helpers below name what they make; these are the resources that
    have nothing to name, so the id is the only handle the sweep can use.

    Tracked rows are never untracked, even after the test deletes them. That is
    the point: `DELETE` on a temporal resource REVOKES rather than removes (see
    `surviving_scratch_rows`), so leaving the id registered turns the sweep into
    a second, independent check that the revoke actually landed.
    """
    identifier = str(value)
    _CREATED_IDS.append(_Tracked(listing_path, id_field, identifier, key, params))
    return identifier


def create_saved_query(client: ApiClient, tag: str, sql: str = SCRATCH_QUERY_REF) -> SavedQuery:
    """`POST /v1/queries` → 201, validated. The caller hard-deletes it.

    `sql` overridable for the parameter-binding cases, which need a statement
    that REFERENCES a named parameter — the deterministic default has none, and
    a query with no parameters cannot show which value was bound to one.
    """
    name = scratch_name(tag)
    response = client.post(
        analytics_path("/v1/queries"),
        json_body={
            "name": name,
            "description": "stand endpoint-contract scratch saved query",
            "sql": sql,
        },
    )
    assert response.status_code == 201, (
        f"create saved query: status={response.status_code} body={response.text[:300]}"
    )
    query = response.parse(SavedQuery)
    assert query.name == name
    assert query.sql == sql
    return query


def scratch_metric_identity(tag: str) -> tuple[str, str]:
    """A fresh, attributable `(metric_key, source_key)` for this run.

    Both carry `RUN_TAG` so a leak points back at the run that made it. The key
    is a dotted `family.name` and the source key is a simple snake_case name,
    each starting `[a-z]` — the shapes `validate_graph` requires.
    """
    slug = f"scratch_{RUN_TAG}_{tag}_{uuid.uuid4().hex[:8]}"
    return f"example.{slug}", slug


def custom_metric_body(
    metric_key: str,
    source_key: str,
    *,
    subject: str | None = None,
    tags: list[str] | None = None,
) -> dict[str, object]:
    """A minimal valid custom-metric graph: a sum over one measure.

    The create/import/update body. Callers mutate a copy for the rejection cases
    — a bad key, a non-single-select statement, or SQL that omits a contract
    column. `subject`/`tags` are omitted unless a caller asks for them, so the
    default body stays the smallest valid graph.
    """
    body: dict[str, object] = {
        "metric_key": metric_key,
        "label": "Scratch probe metric",
        "entity_type": "person",
        "format": "integer",
        "direction": "neutral",
        "computation": "sum",
        "source_key": source_key,
        "observation_sql": SCRATCH_OBSERVATION_SQL,
        "measures": ["events"],
        "dimensions": [],
        "inputs": [{"role": "value", "measure_key": "events"}],
    }
    if subject is not None:
        body["subject"] = subject
    if tags is not None:
        body["tags"] = tags
    return body


def create_custom_metric(client: ApiClient, tag: str) -> CustomMetric:
    """`POST /v1/metrics` → 201, validated. The caller hard-deletes it.

    The metric is registered for the survivor sweep by its `metric_key`: a custom
    metric is removed rather than temporal, so mere presence at session end is a
    leak.
    """
    metric_key, source_key = scratch_metric_identity(tag)
    response = client.post(
        analytics_path("/v1/metrics"),
        json_body=custom_metric_body(metric_key, source_key),
    )
    assert response.status_code == 201, (
        f"create custom metric: status={response.status_code} body={response.text[:300]}"
    )
    metric = response.parse(CustomMetric)
    assert metric.metric_key == metric_key
    track(analytics_path("/v1/metrics"), "metric_key", metric_key)
    return metric


SCRATCH_METRIC_COLUMN: Final[str] = "total"


def _searching(needle: str) -> QueryParams:
    return {"q": needle, "limit": 200}


def scratch_dataset_declaration(title: str) -> dict[str, object]:
    return {
        "title": title,
        "fields": [
            {"name": "day", "path": "day", "type": "datetime", "default_clock": True},
            {"name": "lines", "path": "lines", "type": "int"},
        ],
    }


def create_v3_dataset(client: ApiClient, tag: str) -> Dataset:
    name = scratch_name(tag)

    response = client.put(
        v3_path(f"/v1/datasets/{name}"), json_body=scratch_dataset_declaration(name)
    )
    assert response.status_code == 200, (
        f"declare dataset: status={response.status_code} body={response.text[:300]}"
    )
    track(v3_path("/v1/datasets"), "name", name, params=_searching(name))

    dataset = response.parse(Dataset)
    assert dataset.name == name
    return dataset


def _create_v3_definition(
    client: ApiClient, segment: str, name: str, body: dict[str, object]
) -> str:
    response = client.post(v3_path(f"/v1/{segment}"), json_body={"name": name, "body": body})
    assert response.status_code == 201, (
        f"create {segment}: status={response.status_code} body={response.text[:300]}"
    )

    created = response.parse(CreatedId)
    track(v3_path(f"/v1/{segment}"), "id", created.id, params=_searching(name))
    return created.id


def _read_v3[T: MetricDefinition | WidgetDefinition | DashboardDefinition](
    client: ApiClient, path: str, model: type[T]
) -> T:
    response = client.get(path)
    assert response.status_code == 200, (
        f"read {path}: status={response.status_code} body={response.text[:300]}"
    )
    return response.parse(model)


def create_v3_metric(client: ApiClient, tag: str, dataset: str) -> MetricDefinition:
    body: dict[str, object] = {
        "dataset": dataset,
        "fields": [{"agg": "count", "type": "int", "as_name": SCRATCH_METRIC_COLUMN}],
    }
    metric_id = _create_v3_definition(client, "metrics", scratch_name(tag), body)

    return _read_v3(client, v3_path(f"/v1/metrics/{metric_id}"), MetricDefinition)


def create_v3_widget(client: ApiClient, tag: str, metric: str) -> WidgetDefinition:
    body: dict[str, object] = {
        "type": "table",
        "metric": metric,
        "columns": [SCRATCH_METRIC_COLUMN],
    }
    widget_id = _create_v3_definition(client, "widgets", scratch_name(tag), body)

    return _read_v3(client, v3_path(f"/v1/widgets/{widget_id}"), WidgetDefinition)


def create_v3_dashboard(
    client: ApiClient, tag: str, widgets: Sequence[str] = ()
) -> DashboardDefinition:
    name = scratch_name(tag)
    body: dict[str, object] = {
        "title": name,
        "items": [{"widget": widget} for widget in widgets],
    }
    dashboard_id = _create_v3_definition(client, "dashboards", name, body)

    return _read_v3(client, v3_path(f"/v1/dashboards/{dashboard_id}"), DashboardDefinition)


def create_v3_folder(client: ApiClient, tag: str) -> Folder:
    name = scratch_name(tag)

    response = client.post(v3_path("/v1/folders"), json_body={"name": name})
    assert response.status_code == 201, (
        f"create folder: status={response.status_code} body={response.text[:300]}"
    )

    folder = response.parse(Folder)
    track(v3_path("/v1/folders"), "id", folder.id, key="folders")
    assert folder.name == name
    return folder


def create_v3_alert(client: ApiClient, tag: str, metric: str, destination: str) -> RuleResponse:
    name = scratch_name(tag)
    draft = {
        "name": name,
        "metric": metric,
        "destination": destination,
        "interval_secs": 3600,
        "enabled": False,
        "condition": {
            "rows": "any",
            "where": {"column": SCRATCH_METRIC_COLUMN, "op": ">", "value": 0},
        },
    }

    response = client.post(v3_path("/v1/alerts"), json_body=draft)
    assert response.status_code == 201, (
        f"create alert: status={response.status_code} body={response.text[:300]}"
    )

    rule = response.parse(RuleResponse)
    track(v3_path("/v1/alerts"), "id", rule.id, key="alerts", params=_searching(name))
    assert rule.name == name
    return rule


#: Named resources, and the listing that would still show a leaked one.
_NAMED_LISTINGS: Final[tuple[str, ...]] = (
    analytics_path("/v1/queries"),
    identity_path("/v1/roles"),
)


def _listing_items(
    client: ApiClient, path: str, key: str = "items", params: QueryParams | None = None
) -> list[dict[str, object]]:
    """Rows of one listing, or nothing when it cannot be read.

    A non-200 is swallowed on purpose. The sweep runs at the very end of a
    session, after failures, and a stand that has stopped answering should not
    turn into a confusing second failure about leaked rows.
    """
    response = client.get(path, params=params)
    if response.status_code != 200:
        return []

    body = response.json()
    rows = body.get(key) if isinstance(body, dict) else None
    assert isinstance(rows, list), f"{path} answered no `{key}` list: {response.text[:300]}"
    return [row for row in rows if isinstance(row, dict)]


def surviving_scratch_rows(*, analytics: ApiClient, identity: ApiClient) -> list[str]:
    """Any row this session created that is still in force on the stand.

    Two clients because the services have different callers: the identity admin
    listings, and every insight-v3-core listing, answer 403 to an ordinary
    persona, so only the admin operator can see whether such a row leaked.

    **Still in force, not still present.** The two resource families delete
    differently, and conflating them would make this either useless or a
    permanent false alarm:

    * Metrics, saved queries and roles are removed — soft-deleted or hard — and
      stop being listed. Any appearance is a leak.
    * Person-role assignments and visibility grants are TEMPORAL. `DELETE` sets
      `valid_to`; the row stays and the listing keeps returning it. Only a row
      still carrying `valid_to: null` was left in force.

    So a tracked row is judged by `valid_to` when the listing reports one, and
    by mere presence when it does not.
    """
    issued = issued_names()
    if not issued and not _CREATED_IDS:
        return []

    def client_for(path: str) -> ApiClient:
        return analytics if path.startswith(ANALYTICS_PREFIX) else identity

    leaked: list[str] = []

    for listing in _NAMED_LISTINGS:
        for item in _listing_items(client_for(listing), listing):
            name = item.get("name")
            if isinstance(name, str) and name in issued:
                leaked.append(f"{listing} -> {name}")

    for tracked in _CREATED_IDS:
        client = client_for(tracked.listing)
        for item in _listing_items(client, tracked.listing, tracked.key, tracked.params):
            if str(item.get(tracked.id_field)) != tracked.value:
                continue
            if item.get("valid_to") is not None:
                continue  # revoked, which is what a delete does to these
            leaked.append(f"{tracked.listing} -> {tracked.id_field}={tracked.value} still in force")

    return leaked


__all__: Sequence[str] = (
    "NON_UUID",
    "RUN_TAG",
    "SCRATCH_METRIC_COLUMN",
    "SCRATCH_OBSERVATION_SQL",
    "SCRATCH_PREFIX",
    "SCRATCH_QUERY_REF",
    "SCRATCH_SOURCE_ID",
    "SCRATCH_SOURCE_TYPE",
    "UNKNOWN_ID",
    "UNKNOWN_METRIC_KEY",
    "create_custom_metric",
    "create_saved_query",
    "create_v3_alert",
    "create_v3_dashboard",
    "create_v3_dataset",
    "create_v3_folder",
    "create_v3_metric",
    "create_v3_widget",
    "custom_metric_body",
    "issued_names",
    "scratch_dataset_declaration",
    "scratch_metric_identity",
    "scratch_name",
    "surviving_scratch_rows",
    "track",
    "tracked_ids",
)

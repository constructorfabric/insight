"""Two tenants that share a source id, issue ids and changelog ids stay apart.

A source id names a connector instance inside one tenant only, and Jira ids are
unique only inside one site, so two tenants can hold the very same ids with
different contents. Every model up to the class boundary keys, joins, groups and
ranks by tenant first; these seed the collision and read each model per tenant.
"""

from __future__ import annotations

from typing import Any

from conftest import Scenario
from helpers import SOURCE_ID, TENANT_ID, event, field, issue, item

OTHER_TENANT = "22222222-2222-2222-2222-222222222222"
POINTS = "customfield_10001"
LABEL = "customfield_10002"


def _as_other_tenant(records: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """The same ids under the other tenant, keyed the way its connector would key them."""
    return [{**r, "tenant_id": OTHER_TENANT, "unique_key": f"{OTHER_TENANT}-{r['unique_key']}"} for r in records]


def _seed(scenario: Scenario) -> None:
    mine = {
        "fields": [
            field(POINTS, name="Story Points", schema_type="number"),
            field(LABEL, name="Label", schema_type="string"),
        ],
        "issues": [issue("TST-1", fields={POINTS: 5, LABEL: "mine"})],
        "events": [
            event("TST-1", 101, "2026-01-06T10:00:00", [item(POINTS, frm="3", frm_str="3", to="5", to_str="5")])
        ],
    }
    theirs = {
        "fields": [field(POINTS, name="Story Points", schema_type="number")],
        "issues": [issue("TST-1", fields={POINTS: 9})],
        "events": [
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(POINTS, frm="7", frm_str="7", to="9", to_str="9"), item(LABEL, to="x", to_str="theirs")],
            )
        ],
    }
    scenario.seed(**mine)
    scenario.seed(**{name: _as_other_tenant(rows) for name, rows in theirs.items()})
    scenario.build()


def _rows(scenario: Scenario, sql: str) -> list[dict[str, Any]]:
    return scenario.warehouse.rows(sql, {"src": SOURCE_ID})


def test_each_tenant_keeps_its_own_changelog_items(scenario: Scenario) -> None:
    _seed(scenario)

    rows = _rows(
        scenario,
        "SELECT tenant_id, field_id, value_to_string FROM staging.jira_changelog_items"
        " WHERE insight_source_id = {src:String} ORDER BY tenant_id, field_id",
    )

    assert [(r["tenant_id"], r["field_id"], r["value_to_string"]) for r in rows] == [
        (TENANT_ID, POINTS, "5"),
        (OTHER_TENANT, POINTS, "9"),
        (OTHER_TENANT, LABEL, "theirs"),
    ]


def test_a_field_is_classified_only_where_its_tenant_catalogues_it(scenario: Scenario) -> None:
    _seed(scenario)

    kinds = _rows(
        scenario,
        "SELECT tenant_id, field_id FROM staging.jira__task_field_kind"
        " WHERE insight_source_id = {src:String} ORDER BY tenant_id, field_id",
    )
    unclassified = _rows(
        scenario,
        "SELECT tenant_id, field_id FROM staging.jira__task_field_unclassified WHERE insight_source_id = {src:String}",
    )

    assert [(r["tenant_id"], r["field_id"]) for r in kinds] == [
        (TENANT_ID, POINTS),
        (TENANT_ID, LABEL),
        (OTHER_TENANT, POINTS),
    ]
    assert [(r["tenant_id"], r["field_id"]) for r in unclassified] == [(OTHER_TENANT, LABEL)], (
        "the label is catalogued by the first tenant only; the other's change to it is unclassified there"
    )


def test_each_tenant_snapshots_its_own_issue(scenario: Scenario) -> None:
    _seed(scenario)

    rows = _rows(
        scenario,
        "SELECT tenant_id, value_ids FROM staging.jira_issue_field_snapshot"
        f" WHERE insight_source_id = {{src:String}} AND field_id = '{POINTS}' ORDER BY tenant_id",
    )

    assert [(r["tenant_id"], list(r["value_ids"])) for r in rows] == [(TENANT_ID, ["5"]), (OTHER_TENANT, ["9"])]

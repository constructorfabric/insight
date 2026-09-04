"""The invariants, and the selector the pipeline actually runs.

Every other module builds only the field-history models and their ancestors,
because that is what its scenario exercises. This module pays for the full
staging selector once: if the field-history models stop coexisting with the rest
of the Jira staging chain, the pipeline breaks and no narrow selector notices.
"""

from __future__ import annotations

from conftest import PROD_SELECTOR, Scenario
from helpers import event, field, issue, item

FIELDS = [
    field("assignee", name="Assignee", schema_type="user"),
    field("status", name="Status", schema_type="status"),
    field("components", name="Components", schema_type="array", schema_items="component"),
    field("labels", name="Labels", schema_type="array", schema_items="string"),
    field("duedate", name="Due date", schema_type="date"),
    field("description", name="Description", schema_type="string"),
    field("customfield_10001", name="Story Points", schema_type="number"),
    field(
        "customfield_10002",
        name="Incident Start",
        schema_type="datetime",
        schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:datetime",
    ),
    field(
        "customfield_10003",
        name="Severity",
        schema_type="option",
        schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:select",
    ),
    field(
        "customfield_10004",
        name="Products",
        schema_type="array",
        schema_items="option",
        schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:multiselect",
    ),
]

VALUES = {
    "assignee": {"accountId": "alice-acct", "displayName": "Alice Alpha"},
    "status": {"id": "6", "name": "Closed"},
    "components": [{"id": "501", "name": "api"}],
    "labels": ["alpha", "beta"],
    "duedate": "2026-03-15",
    "description": {"type": "doc", "version": 1, "content": []},
    "customfield_10001": 5,
    "customfield_10002": "2026-02-01T07:00:00.000+0000",
    "customfield_10003": {"id": "9001", "value": "High"},
    "customfield_10004": [{"id": "7001", "value": "Storage"}],
}

HISTORY = [
    event(
        "TST-1",
        101,
        "2026-01-06T10:00:00",
        [
            item("status", frm="1", frm_str="Open", to="3", to_str="In Progress"),
            item("customfield_10001", frm="3", frm_str="3", to="5", to_str="5"),
        ],
    ),
    event(
        "TST-1",
        102,
        "2026-01-07T10:00:00",
        [item("components", to="501", to_str="api"), item("labels", to_str="alpha beta")],
    ),
    event(
        "TST-1",
        103,
        "2026-01-08T10:00:00",
        [
            item("status", frm="3", frm_str="In Progress", to="6", to_str="Closed"),
            item("customfield_10004", to="[7001]", to_str="Storage"),
        ],
    ),
]


def _seed(scenario):
    scenario.seed(fields=FIELDS, issues=[issue("TST-1", fields=VALUES), issue("TST-2", fields=VALUES)], events=HISTORY)


def test_every_jira_invariant_holds_on_a_healthy_issue(scenario: Scenario) -> None:
    """Ten fields across every kind, two issues, one with history and one
    without. All of it must satisfy the contract tests at once — the parallel
    arrays, the single-value bound, the event-id conventions, the cardinality
    rules, traceability back to bronze, and the round trip.
    """
    _seed(scenario)
    scenario.build()

    assert scenario.invariants_hold(), (
        "a dbt singular test under tests/jira failed on a scenario that should "
        "satisfy all of them; run `dbt test --select tests/jira` to see which"
    )


def test_the_pipeline_selector_builds(scenario: Scenario) -> None:
    """`tag:jira,tag:staging` is the string the prod staging step runs.

    Selecting it here is the only thing that catches a field-history model that
    works in isolation but breaks the chain — a name collision, a missing
    `depends_on`, or a model the promote view has to precede.
    """
    _seed(scenario)
    scenario.build(PROD_SELECTOR)

    assert scenario.invariants_hold()

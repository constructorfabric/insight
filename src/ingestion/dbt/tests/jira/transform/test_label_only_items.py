"""System fields Jira names by label alone, with no `fieldId` in the item.

A link added or removed arrives as `"field": "Link"` and a change of parent as
`"IssueParentAssociation"`, neither carrying the field id the rest of the
changelog uses. They are still `issuelinks` and `parent`, and an item dropped
for lacking a field id is history the journal never had.
"""

from __future__ import annotations

from typing import Any

from conftest import Scenario, case
from helpers import event, field, issue

LINKS = "issuelinks"
LINKS_FIELD = field(LINKS, name="Linked Issues", schema_type="array", schema_items="issuelinks")
PARENT = "parent"
PARENT_FIELD = field(PARENT, name="Parent")


def _by_label(label: str, *, to: str, to_str: str) -> dict[str, Any]:
    return {"field": label, "fieldtype": "jira", "from": None, "fromString": None, "to": to, "toString": to_str}


@case(
    fields=[LINKS_FIELD],
    issues=[
        issue("TST-1", fields={LINKS: [{"id": "6", "type": {"name": "Relates"}, "outwardIssue": {"key": "TST-9"}}]})
    ],
    events=[
        event(
            "TST-1", 101, "2026-01-06T10:00:00", [_by_label("Link", to="TST-9", to_str="This issue relates to TST-9")]
        )
    ],
)
def test_a_link_named_by_label_is_an_issuelinks_event(scenario: Scenario) -> None:
    assert scenario.states(LINKS) == [[], ["TST-9"]]
    assert scenario.round_trip_holds()


@case(
    fields=[PARENT_FIELD],
    issues=[issue("TST-1", fields={PARENT: {"id": "10005", "key": "TST-5"}})],
    events=[
        event("TST-1", 101, "2026-01-06T10:00:00", [_by_label("IssueParentAssociation", to="10005", to_str="TST-5")])
    ],
)
def test_a_parent_association_is_a_parent_event(scenario: Scenario) -> None:
    assert scenario.states(PARENT) == [[], ["10005"]]
    assert scenario.round_trip_holds()

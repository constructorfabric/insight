"""Which issues an issue points at: `class_task_references` over links, keys
written in field values, and keys written in comments.

One issue of project APP points at issues of project DEF in every way Jira
allows, next to text that only looks like a key. Each rule below is one row the
model must produce, or must not.
"""

from __future__ import annotations

import json
import uuid
from collections.abc import Iterator
from dataclasses import dataclass
from typing import Any

import pytest
from conftest import LINKS_SELECTOR, Scenario, Warehouse, clear_stateful_link_inputs
from helpers import CREATED_AT, SOURCE_ID, TENANT_ID, event, field, issue, item

SELECTOR = f"{LINKS_SELECTOR} +jira__task_references"

POINTING_ID = "1001"
RICH_TEXT = "customfield_10100"
FOUND_IN_VERSION = "customfield_10200"
LINKS = "issuelinks"
BLOCKS = {"id": "10000", "name": "Blocks", "outward": "blocks", "inward": "is blocked by"}

EDITED_AT = "2026-01-06 10:00:00.000"
FIRST_READ = "2026-01-15T03:00:00"
LATER_READ = "2026-03-28T03:00:00"
LIVE_COMMENT = "9001"
DELETED_COMMENT = "9002"


def _text(text: str) -> dict[str, Any]:
    return {"type": "text", "text": text}


def _doc(*nodes: dict[str, Any]) -> dict[str, Any]:
    return {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": list(nodes)}]}


DESCRIPTION = _doc(
    _text("Regression of DEF-1, see APP-1 itself, encoded as UTF-8; moved: OLD-1; unsynced: NOPE-5; future DEF-999 "),
    {"type": "inlineCard", "attrs": {"url": "https://example.atlassian.net/browse/DEF-2"}},
    _text(" spec "),
    {
        "type": "text",
        "text": "here",
        "marks": [{"type": "link", "attrs": {"href": "https://example.atlassian.net/browse/DEF-5"}}],
    },
    {"type": "mention", "attrs": {"id": "acct-0001", "text": "@DEF-4", "accessLevel": ""}},
    _text("Next line:\nDEF-12, not a key: DEF-13x"),
)
RICH_TEXT_VALUE = _doc(_text("Root cause tracked in DEF-3."))
OLD_DESCRIPTION = "Old mention of DEF-7"
RENDERED_DESCRIPTION = "Regression of DEF-1"


def _def(n: int, *, key: str | None = None, extracted_at: str = "2026-03-01T12:00:00") -> dict[str, Any]:
    return issue(key or f"DEF-{n}", jira_id=str(2000 + n), extracted_at=extracted_at)


ISSUES = [
    issue(
        "APP-1",
        jira_id=POINTING_ID,
        fields={
            "summary": "Follow-up of DEF-6",
            "description": DESCRIPTION,
            RICH_TEXT: RICH_TEXT_VALUE,
            "labels": ["triage", "DEF-14"],
            FOUND_IN_VERSION: {"id": "10", "value": "Seen with DEF-15"},
            LINKS: [{"id": "1", "type": BLOCKS, "outwardIssue": {"id": "2008", "key": "DEF-8"}}],
        },
    ),
    *(_def(n) for n in (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13, 14, 15)),
    _def(11, key="OLD-1", extracted_at="2026-01-10T12:00:00"),
    _def(11),
]

FIELDS = [
    field("summary", name="Summary", schema_type="string"),
    field("description", name="Description", schema_type="string"),
    field(
        RICH_TEXT,
        name="Root cause",
        schema_type="string",
        schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:textarea",
    ),
    field(LINKS, name="Linked Issues", schema_type="array", schema_items="issuelinks"),
    field("labels", name="Labels", schema_type="array", schema_items="string"),
    field(FOUND_IN_VERSION, name="Found in", schema_type="option"),
]

EVENTS = [
    event(
        "APP-1",
        101,
        "2026-01-06T10:00:00",
        [item("description", frm_str=OLD_DESCRIPTION, to_str=RENDERED_DESCRIPTION)],
        jira_id=POINTING_ID,
    )
]


def _airbyte(extracted_at: str) -> dict[str, Any]:
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": extracted_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
    }


def _comment(comment_id: str, body: dict[str, Any], extracted_at: str) -> dict[str, Any]:
    return {
        **_airbyte(extracted_at),
        "id": comment_id,
        "tenant_id": TENANT_ID,
        "source_id": SOURCE_ID,
        "unique_key": f"{TENANT_ID}-{SOURCE_ID}-{comment_id}",
        "comment_id": int(comment_id),
        "id_readable": "APP-1",
        "jira_id": POINTING_ID,
        "author_account_id": "author-acct",
        "created": "2026-01-10T09:00:00.000+0000",
        "updated": "2026-01-10T09:00:00.000+0000",
        "body": json.dumps(body),
        "collected_at": extracted_at,
    }


# A later read of the issue's comments re-emitted the live one and left the
# other out, so the other is deleted.
COMMENTS = [
    _comment(LIVE_COMMENT, _doc(_text("Caused by DEF-9")), LATER_READ),
    _comment(DELETED_COMMENT, _doc(_text("Maybe DEF-10")), FIRST_READ),
]


def _project(project_id: int, key: str) -> dict[str, Any]:
    return {
        **_airbyte(FIRST_READ),
        "id": str(project_id),
        "tenant_id": TENANT_ID,
        "source_id": SOURCE_ID,
        "unique_key": f"{SOURCE_ID}-project-{project_id}",
        "project_id": project_id,
        "project_key": key,
        "name": f"Project {key}",
        "collected_at": FIRST_READ,
    }


PROJECTS = [_project(1, "APP"), _project(2, "DEF")]


@dataclass(frozen=True)
class Reference:
    found_in: str
    found_ref: str
    target_key: str
    target_id: str | None
    is_current: int
    found_kind: str


@dataclass(frozen=True)
class Rule:
    rule: str
    expected: Reference


RULES = [
    Rule("a structured link is a reference", Reference("link", "blocking", "DEF-8", "2008", 1, "link")),
    Rule("a key in the description text", Reference("field", "description", "DEF-1", "2001", 1, "long_text")),
    Rule("a key in an ADF smart link URL", Reference("field", "description", "DEF-2", "2002", 1, "long_text")),
    Rule("a key in the href of a link mark", Reference("field", "description", "DEF-5", "2005", 1, "long_text")),
    Rule("a key in a rich-text custom field", Reference("field", RICH_TEXT, "DEF-3", "2003", 1, "long_text")),
    Rule("a key in a short text field", Reference("field", "summary", "DEF-6", "2006", 1, "short_text")),
    Rule(
        "a key only an older body held is no longer current",
        Reference("field", "description", "DEF-7", "2007", 0, "long_text"),
    ),
    Rule(
        "a moved issue's old key resolves to its id and current key",
        Reference("field", "description", "DEF-11", "2011", 1, "long_text"),
    ),
    Rule(
        "an unknown key of a synced project is kept unresolved",
        Reference("field", "description", "DEF-999", None, 1, "long_text"),
    ),
    Rule(
        "a key after an escaped line break in ADF", Reference("field", "description", "DEF-12", "2012", 1, "long_text")
    ),
    Rule("a key in a label", Reference("field", "labels", "DEF-14", "2014", 1, "short_text")),
    Rule("a key in a select option's value", Reference("field", FOUND_IN_VERSION, "DEF-15", "2015", 1, "short_text")),
    Rule("a key in a live comment", Reference("comment", LIVE_COMMENT, "DEF-9", "2009", 1, "comment")),
]

NOT_REFERENCES = [
    ("a key-shaped word that names no issue or synced project", "UTF-8"),
    ("an unknown key of a project the source does not sync", "NOPE-5"),
    ("an issue naming its own key", "APP-1"),
    ("a key only a deleted comment held", "DEF-10"),
    ("a mention's display name", "DEF-4"),
    ("a key glued to the word after it", "DEF-13"),
]


@pytest.fixture(scope="module")
def references(warehouse: Warehouse) -> Iterator[list[Reference]]:
    scenario = Scenario(warehouse)
    _clear(warehouse)
    scenario.seed(fields=FIELDS, issues=ISSUES, events=EVENTS)
    warehouse.insert("bronze_jira.jira_comments", COMMENTS)
    warehouse.insert("bronze_jira.jira_projects", PROJECTS)
    scenario.build(SELECTOR)
    warehouse.dbt("run", "--select", "class_task_references")

    rows = warehouse.rows(
        "SELECT found_in, found_ref, target_key, target_id, is_current, found_kind"
        " FROM silver.class_task_references FINAL"
        " WHERE insight_source_id = {src:String} AND issue_id = {issue:String}"
        " ORDER BY found_in, found_ref, target_key",
        {"src": SOURCE_ID, "issue": POINTING_ID},
    )
    yield [Reference(**row) for row in rows]
    _clear(warehouse)


def _clear(warehouse: Warehouse) -> None:
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_fields")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_issue")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_issue_history")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_comments")
    warehouse.execute("TRUNCATE TABLE IF EXISTS bronze_jira.jira_projects")
    warehouse.execute("TRUNCATE TABLE IF EXISTS silver.class_task_references")
    clear_stateful_link_inputs(warehouse)
    warehouse.generation += 1


@pytest.mark.parametrize("case", RULES, ids=[r.rule for r in RULES])
def test_each_way_of_pointing_is_one_reference(references: list[Reference], case: Rule) -> None:
    assert case.expected in references, f"should hold: {case.rule}"


@pytest.mark.parametrize(("rule", "key"), NOT_REFERENCES, ids=[r for r, _ in NOT_REFERENCES])
def test_text_that_names_no_other_issue_is_no_reference(references: list[Reference], rule: str, key: str) -> None:
    assert key not in {r.target_key for r in references}, f"should not be a reference: {rule}"


def test_the_references_are_exactly_the_rules(references: list[Reference]) -> None:
    assert sorted(references, key=repr) == sorted((r.expected for r in RULES), key=repr)


def test_first_seen_is_the_earliest_value_that_held_the_key(warehouse: Warehouse, references: list[Reference]) -> None:
    rows = warehouse.rows(
        "SELECT target_key, toString(first_seen_at) AS first_seen_at"
        " FROM silver.class_task_references FINAL"
        " WHERE insight_source_id = {src:String} AND issue_id = {issue:String} AND found_ref = 'description'"
        "   AND target_key IN ('DEF-1', 'DEF-7')",
        {"src": SOURCE_ID, "issue": POINTING_ID},
    )
    assert {r["target_key"]: r["first_seen_at"] for r in rows} == {
        "DEF-7": CREATED_AT.replace("T", " ") + ".000",
        "DEF-1": EDITED_AT,
    }


def test_issues_of_one_project_pointing_at_another_is_one_query(
    warehouse: Warehouse, references: list[Reference]
) -> None:
    rows = warehouse.rows(
        "SELECT DISTINCT target_key"
        " FROM silver.class_task_references FINAL"
        " WHERE insight_source_id = {src:String} AND is_current = 1"
        "   AND splitByChar('-', id_readable)[1] = 'APP' AND splitByChar('-', target_key)[1] = 'DEF'"
        " ORDER BY target_key",
        {"src": SOURCE_ID},
    )
    assert [r["target_key"] for r in rows] == [
        "DEF-1",
        "DEF-11",
        "DEF-12",
        "DEF-14",
        "DEF-15",
        "DEF-2",
        "DEF-3",
        "DEF-5",
        "DEF-6",
        "DEF-8",
        "DEF-9",
        "DEF-999",
    ]

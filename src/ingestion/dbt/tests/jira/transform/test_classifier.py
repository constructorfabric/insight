"""The shape of a field comes from its catalogue row, never from its value.

Deciding it from the value is the root cause the whole change addresses, so
these tests feed the SAME value through fields whose only difference is
metadata, and require different results.
"""

from __future__ import annotations

from helpers import event, field, issue, item

TAGS = "customfield_10007"
PICKS = "customfield_10008"
CUSTOM_VERSIONS = "customfield_10009"
POINTS = "customfield_10001"


def test_identical_values_are_read_by_their_own_metadata(scenario):
    """Two fields, byte-identical items, different kinds.

    A labels-type field puts its whole list, space-separated, in the rendered
    side; a multi-select puts a bracketed id list in the id side. Given an item
    that carries only a rendered side, one is a two-element list and the other
    is a single value — and nothing in the item says which.
    """
    scenario.seed(
        fields=[
            field(
                TAGS,
                name="Tags",
                schema_type="array",
                schema_items="string",
                schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:labels",
            ),
            field(
                PICKS,
                name="Picks",
                schema_type="option",
                schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:select",
            ),
        ],
        issues=[issue("TST-1", fields={})],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(TAGS, to=None, to_str="alpha beta"), item(PICKS, to=None, to_str="alpha beta")],
            )
        ],
    )
    scenario.build()

    # The issue carries neither key, so both fields also get a withdrawal row
    # (§3.6); this scenario is about how the ITEM is read, so it looks at the
    # rows the event produced.
    assert scenario.changelog_states(TAGS) == [["alpha", "beta"]]
    assert scenario.changelog_states(PICKS) == [["alpha beta"]]


def test_a_custom_picker_over_a_system_item_type_is_a_full_list(scenario):
    """`fixVersions` and a custom multi-version picker are both `array` /
    `version`. Same structure, different changelog shape — the system field
    emits one item per element, the custom one the whole list — so whether
    `schema_custom` is populated is what separates them."""
    scenario.seed(
        fields=[
            field("fixVersions", name="Fix versions", schema_type="array", schema_items="version"),
            field(
                CUSTOM_VERSIONS,
                name="Affected builds",
                schema_type="array",
                schema_items="version",
                schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:multiversion",
            ),
        ],
        issues=[
            issue(
                "TST-1",
                fields={
                    "fixVersions": [{"id": "301", "name": "1.0"}, {"id": "302", "name": "1.1"}],
                    CUSTOM_VERSIONS: [{"id": "401", "name": "b1"}, {"id": "402", "name": "b2"}],
                },
            )
        ],
        events=[
            # system: one element per item
            event("TST-1", 101, "2026-01-06T10:00:00", [item("fixVersions", to="301", to_str="1.0")]),
            event("TST-1", 102, "2026-01-07T10:00:00", [item("fixVersions", to="302", to_str="1.1")]),
            # custom: the whole list in one item
            event(
                "TST-1",
                103,
                "2026-01-08T10:00:00",
                [item(CUSTOM_VERSIONS, frm="[401]", frm_str="b1", to="[401, 402]", to_str="b1,b2")],
            ),
        ],
    )
    scenario.build()

    assert scenario.states("fixVersions") == [[], ["301"], ["301", "302"]]
    assert scenario.states(CUSTOM_VERSIONS) == [["401"], ["401", "402"]]
    assert scenario.round_trip_holds()


def test_a_comment_item_produces_no_field_state(scenario):
    """A comment item carries the body in the rendered `from` side with every
    other side empty — byte-identical to a labels field cleared to empty. The
    only thing that tells them apart is that `comment` is not field state, and
    that comes from metadata."""
    scenario.seed(
        fields=[
            field("comment", name="Comment", schema_type="comments-page"),
            field(POINTS, name="Story Points", schema_type="number"),
        ],
        issues=[issue("TST-1", fields={POINTS: 5})],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [item("comment", frm_str="a deleted comment body")])],
    )
    scenario.build()

    assert [r["field_id"] for r in scenario.journal(issue="TST-1")] == ["created", POINTS]


def test_an_unclassifiable_field_stops_the_run(scenario):
    """`schema_type = 'any'` means an app owns the type, so structure says
    nothing. An unrecognised app key there must fail loudly: the two defects
    this design replaces were both silent, and a field with real changelog
    traffic disappearing quietly is exactly what must not happen again.
    """
    scenario.seed(
        fields=[
            field(
                "customfield_10010",
                name="Some app field",
                schema_type="any",
                schema_custom="com.example.newapp:some-new-type",
            )
        ],
        issues=[issue("TST-1", fields={"customfield_10010": "x"})],
    )
    code, output = scenario.warehouse.dbt_status("run", "--select", "tag:jira,tag:staging")

    assert code != 0, "an unmapped field kind must fail the run"
    # ClickHouse requires throwIf's message to be constant, so it names where to
    # look rather than which field.
    assert "jira__task_field_kind" in output


def test_the_newest_catalogue_row_decides(scenario):
    """A field's type can change, and bronze keeps every version it has seen.
    The classifier must read the current one — reading an older row would parse
    today's values by yesterday's rule."""
    scenario.seed(
        fields=[
            field(POINTS, name="Incident Start", schema_type="string", extracted_at="2026-01-01T00:00:00"),
            field(
                POINTS,
                name="Incident Start",
                schema_type="datetime",
                schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:datetime",
                extracted_at="2026-02-01T00:00:00",
            ),
        ],
        issues=[issue("TST-1", fields={POINTS: "2026-02-01T07:00:00.000+0000"})],
    )
    scenario.build()

    # Parsed as an instant, which only the newer row's type asks for.
    assert scenario.states(POINTS) == [["2026-02-01 07:00:00.000"]]


def test_the_catalogue_created_field_does_not_shadow_the_creation_marker(scenario):
    """Jira's catalogue contains a real `created` field, and `created` is also
    the sentinel field id of the per-issue creation marker. Emitting both
    produces two rows with the same unique key, and ReplacingMergeTree then
    keeps an arbitrary one."""
    scenario.seed(
        fields=[
            field("created", name="Created", schema_type="datetime"),
            field(POINTS, name="Story Points", schema_type="number"),
        ],
        issues=[issue("TST-1", fields={"created": "2026-01-05T09:00:00.000+0000", POINTS: 5})],
    )
    scenario.build()

    marker = scenario.journal(issue="TST-1", field="created")
    assert len(marker) == 1
    assert marker[0]["_seq"] == 0
    assert marker[0]["value_ids"] == []

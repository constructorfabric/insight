"""A changelog field the catalogue does not contain (spec §3.2).

Absence from `bronze_jira.jira_fields` cannot be classified even in principle:
without a catalogue row there is no `schema_type`, so no separator, no id side
and no cardinality. Before this the join to the classifier was an inner one and
the field's whole history disappeared without a trace — the same defect class as
the reported one, on the one input the design cannot resolve.

What it gets instead is one best-effort row per (issue, field) carrying the last
value as it arrived, plus a registry row so the exclusion is queryable, plus a
test that separates a field deleted long ago from one whose metadata simply has
not arrived yet.
"""

from __future__ import annotations

from helpers import event, field, issue, item

GHOST = "customfield_19001"
POINTS = "customfield_10001"
POINTS_FIELD = field(POINTS, name="Story Points", schema_type="number")


def _ghost_history(key: str = "TST-1") -> list[dict]:
    return [
        event(key, 101, "2026-01-06T10:00:00", [item(GHOST, frm=None, frm_str=None, to="7001", to_str="Kernel")]),
        event(
            key, 102, "2026-01-07T10:00:00", [item(GHOST, frm="7001", frm_str="Kernel", to="7002", to_str="Network")]
        ),
    ]


def test_an_uncatalogued_field_keeps_its_last_value(scenario):
    """Two events, no catalogue row: exactly one row, carrying the newest `to`
    side verbatim and stamped with that event's own time.

    Verbatim because the shape is unknowable — parsing a list or an id side here
    would be the guess this design replaces.
    """
    scenario.seed(fields=[POINTS_FIELD], issues=[issue("TST-1", fields={POINTS: 5})], events=_ghost_history())
    scenario.build()

    rows = scenario.journal(field=GHOST)
    assert len(rows) == 1
    assert rows[0]["event_kind"] == "unclassified_field"
    assert rows[0]["event_id"] == "unclassified:TST-1"
    assert rows[0]["value_ids"] == ["7002"]
    assert rows[0]["value_displays"] == ["Network"]
    assert rows[0]["event_at"].startswith("2026-01-07 10:00:00")
    # Nothing establishes that the value IS an identifier, so it does not claim
    # to be one.
    assert rows[0]["value_id_type"] == "none"


def test_the_field_keeps_the_name_the_changelog_gave_it(scenario):
    """The item's own display name is the only naming there is, and it is
    present even when the catalogue row is not."""
    scenario.seed(
        fields=[POINTS_FIELD],
        issues=[issue("TST-1", fields={POINTS: 5})],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [item(GHOST, to="1", to_str="x")])],
    )
    scenario.build()

    # `helpers.item` puts the field id in both `field` and `fieldId`, which is
    # what Jira does for a field it can still name.
    assert scenario.journal(field=GHOST)[0]["field_id"] == GHOST


def test_a_classified_field_never_lands_here(scenario):
    """The exclusion is keyed on the WHOLE catalogue. A field that is `ignored`
    or `UNKNOWN` has been looked at and decided, so it must not be swept in as
    unclassifiable."""
    scenario.seed(
        fields=[POINTS_FIELD, field("votes", name="Votes", schema_type="votes")],
        issues=[issue("TST-1", fields={POINTS: 5})],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [item("votes", to="3", to_str="3")])],
    )
    scenario.build()

    kinds = {r["event_kind"] for r in scenario.journal(issue="TST-1")}
    assert "unclassified_field" not in kinds
    assert [r["field_id"] for r in scenario.journal(issue="TST-1")] == ["created", POINTS]


def test_the_registry_records_the_exclusion(scenario):
    """The exclusion has to be queryable, or it is just a silent drop with extra
    steps."""
    scenario.seed(fields=[POINTS_FIELD], issues=[issue("TST-1", fields={POINTS: 5})], events=_ghost_history())
    scenario.build()

    rows = scenario.warehouse.rows(
        "SELECT field_id, field_name, changelog_items, issues_affected,"
        "       toString(newest_event) AS newest_event, metadata_is_missing"
        " FROM staging.jira__task_field_unclassified"
        f" WHERE field_id = '{GHOST}'"
    )
    assert len(rows) == 1
    assert rows[0]["changelog_items"] == 2
    assert rows[0]["issues_affected"] == 1


def test_a_field_whose_metadata_has_not_arrived_yet_fails_the_guard(scenario):
    """The dangerous half of §3.2.

    A field deleted before the connector's first field sync can only have OLD
    events, and there is nothing to do about it. A field created since the last
    sync has events NEWER than that first sync — its metadata is missing, not
    gone, and dropping it silently is the defect this design removes. The guard
    is the event timestamp, and it is the only thing that separates them.
    """
    scenario.seed(
        fields=[field(POINTS, name="Story Points", schema_type="number", extracted_at="2026-01-01T00:00:00")],
        issues=[issue("TST-1", fields={POINTS: 5})],
        events=[event("TST-1", 101, "2026-06-01T10:00:00", [item(GHOST, to="7001", to_str="Kernel")])],
    )
    scenario.build()

    assert (
        scenario.warehouse.rows(
            f"SELECT metadata_is_missing FROM staging.jira__task_field_unclassified WHERE field_id = '{GHOST}'"
        )[0]["metadata_is_missing"]
        == 1
    )
    assert not scenario.invariants_hold("assert_jira_unclassified_fields_are_old")


def test_an_ancient_deleted_field_passes_the_guard(scenario):
    """The other half: events older than the catalogue's first sync are what a
    long-deleted field looks like, and the run must not fail on it."""
    scenario.seed(
        fields=[field(POINTS, name="Story Points", schema_type="number", extracted_at="2026-06-01T00:00:00")],
        issues=[issue("TST-1", fields={POINTS: 5})],
        events=[event("TST-1", 101, "2020-01-06T10:00:00", [item(GHOST, to="7001", to_str="Kernel")])],
    )
    scenario.build()

    assert scenario.invariants_hold("assert_jira_unclassified_fields_are_old")

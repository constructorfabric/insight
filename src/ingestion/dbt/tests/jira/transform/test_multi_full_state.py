"""Multi-value fields whose changelog item carries the WHOLE list.

Three kinds share this shape and differ only in how the list is joined, and the
join is a property of the kind — not something to detect in the value:

  string_array   labels: no id side at all, the list space-separated in the
                 rendered side. The replaced pipeline looked for an id side,
                 found none, and discarded the event outright.
  option_array   multi-select and friends: a bracketed id list plus displays
                 joined by a bare comma. The replaced pipeline read the literal
                 `[7001, 7002]` as one id.
  legacy_list    the pre-2020 Sprint serialization, `", "` on both sides.

These are the defects the change exists for, so each test states both what the
journal holds and, where the two sides share an id space, that the round trip
accepts it.
"""

from __future__ import annotations

from helpers import event, field, issue, item

LABELS = "labels"
PRODUCTS = "customfield_10004"
SPRINT = "customfield_10005"
ASSISTING = "customfield_10006"

LABELS_FIELD = field(LABELS, name="Labels", schema_type="array", schema_items="string")
PRODUCTS_FIELD = field(
    PRODUCTS,
    name="Products",
    schema_type="array",
    schema_items="option",
    schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:multiselect",
)
SPRINT_FIELD = field(
    SPRINT,
    name="Sprint",
    schema_type="array",
    schema_items="json",
    schema_custom="com.pyxis.greenhopper.jira:gh-sprint",
)
ASSISTING_FIELD = field(
    ASSISTING,
    name="Assisting",
    schema_type="array",
    schema_items="user",
    schema_custom="com.atlassian.jira.plugin.system.customfieldtypes:people",
)


# ── string_array: the family with no id side ────────────────────────────────


def test_a_labels_event_is_not_discarded(scenario):
    """The item's id sides are both NULL and the whole list sits in the rendered
    sides, space-separated. Jira rejects whitespace inside a label, so a space
    is always a separator — there is nothing to detect."""
    scenario.seed(
        fields=[LABELS_FIELD],
        issues=[issue("TST-1", fields={LABELS: ["alpha", "beta"]})],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(LABELS, frm=None, frm_str="alpha", to=None, to_str="alpha beta")],
            )
        ],
    )
    scenario.build()

    assert scenario.states(LABELS) == [["alpha"], ["alpha", "beta"]]
    assert scenario.round_trip_holds()


def test_a_label_is_its_own_identifier(scenario):
    """A label has no id, so the string is both value and identifier — which is
    what `value_id_type` has to say, or a consumer treats it as an opaque key."""
    scenario.seed(fields=[LABELS_FIELD], issues=[issue("TST-1", fields={LABELS: ["alpha"]})])
    scenario.build()

    row = scenario.journal(field=LABELS)[0]
    assert row["value_ids"] == row["value_displays"] == ["alpha"]
    assert row["value_id_type"] == "string_literal"
    assert row["field_cardinality"] == "multi"


def test_a_comma_inside_a_label_is_content(scenario):
    """The separator is a space, so a comma is part of the label. Splitting on
    `", "` — which is how the shape used to be guessed — turns one label into
    two."""
    scenario.seed(
        fields=[LABELS_FIELD],
        issues=[issue("TST-1", fields={LABELS: ["needs-ux,design"]})],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(LABELS, frm=None, frm_str=None, to=None, to_str="needs-ux,design")],
            )
        ],
    )
    scenario.build()

    assert scenario.states(LABELS) == [[], ["needs-ux,design"]]
    assert scenario.round_trip_holds()


def test_labels_cleared_to_empty_is_an_ordinary_event(scenario):
    """An empty right side is a value, not a degenerate item: the left side
    still names what was removed."""
    scenario.seed(
        fields=[LABELS_FIELD],
        issues=[issue("TST-1", fields={LABELS: []})],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(LABELS, frm=None, frm_str="alpha beta", to=None, to_str=None)],
            )
        ],
    )
    scenario.build()

    assert scenario.states(LABELS) == [["alpha", "beta"], []]
    assert scenario.round_trip_holds()


# ── option_array: the bracketed-id family ──────────────────────────────────


def test_a_bracketed_id_list_parses_into_its_elements(scenario):
    """`[7001, 7002]` is two ids, not one. The id side is authoritative here
    because numeric ids inside brackets parse without ambiguity; displays are
    matched to them positionally."""
    scenario.seed(
        fields=[PRODUCTS_FIELD],
        issues=[
            issue("TST-1", fields={PRODUCTS: [{"id": "7001", "value": "Storage"}, {"id": "7002", "value": "Network"}]})
        ],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(PRODUCTS, frm="[7001]", frm_str="Storage", to="[7001, 7002]", to_str="Storage,Network")],
            )
        ],
    )
    scenario.build()

    rows = scenario.journal(field=PRODUCTS)
    assert [r["value_ids"] for r in rows] == [["7001"], ["7001", "7002"]]
    assert [r["value_displays"] for r in rows] == [["Storage"], ["Storage", "Network"]]
    assert scenario.round_trip_holds()


def test_a_display_containing_a_comma_falls_back_to_the_ids(scenario):
    """Displays are joined by a BARE comma, so a display that contains one
    cannot be split back. When the two sides disagree on length the ids stand
    in as displays rather than emitting a mismatched pair — the arrays must stay
    parallel, and a wrong display is recoverable while a broken pairing is not.
    """
    scenario.seed(
        fields=[PRODUCTS_FIELD],
        issues=[issue("TST-1", fields={PRODUCTS: [{"id": "7003", "value": "Example, Ltd"}]})],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(PRODUCTS, frm=None, frm_str=None, to="[7003]", to_str="Example, Ltd")],
            )
        ],
    )
    scenario.build()

    changed = scenario.journal(field=PRODUCTS)[-1]
    assert changed["value_ids"] == ["7003"]
    assert changed["value_displays"] == ["7003"]
    # The id side still reconciles, which is why the round trip compares ids.
    assert scenario.round_trip_holds()


def test_an_id_repeated_by_jira_counts_once(scenario):
    """Jira's own bracketed list can repeat an id. Deduplicating the (id,
    display) PAIRS is not enough — the same element can carry different
    displays on the two sides — so the dedup keys on the id."""
    scenario.seed(
        fields=[PRODUCTS_FIELD],
        issues=[issue("TST-1", fields={PRODUCTS: [{"id": "7001", "value": "Storage"}]})],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(PRODUCTS, frm=None, frm_str=None, to="[7001, 7001]", to_str="Storage,Storage")],
            )
        ],
    )
    scenario.build()

    assert scenario.states(PRODUCTS) == [[], ["7001"]]
    assert scenario.round_trip_holds()


def test_a_display_only_item_keeps_the_event(scenario):
    """Not every field of this family supplies ids: an app field can send items
    whose id side is empty and whose display holds the value. Falling back to
    the empty id side loses the event — the failure mode this design removes —
    so the displays serve as both value and identifier.

    The round trip is not asserted: the issue JSON identifies these elements by
    account id while the changelog names them, so the two sides are different id
    spaces by construction.
    """
    scenario.seed(
        fields=[ASSISTING_FIELD],
        issues=[issue("TST-1", fields={ASSISTING: [{"accountId": "alice-acct", "displayName": "Alice Alpha"}]})],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(ASSISTING, frm=None, frm_str=None, to=None, to_str="Alice Alpha,Bob Beta")],
            )
        ],
    )
    scenario.build()

    changed = scenario.journal(field=ASSISTING)[-1]
    assert changed["value_ids"] == ["Alice Alpha", "Bob Beta"]
    assert changed["value_displays"] == ["Alice Alpha", "Bob Beta"]


# ── legacy_list: the one named exception ───────────────────────────────────


def test_the_legacy_sprint_serialization_splits_on_comma_space(scenario):
    """Sprint is the one kind whose separator really is `", "`, on both sides.

    Its element ids also arrive UNQUOTED in the issue JSON — a bare number
    where an option id is a string — which is why every id goes through the
    unquote helper instead of `JSONExtractString`.
    """
    scenario.seed(
        fields=[SPRINT_FIELD],
        issues=[issue("TST-1", fields={SPRINT: [{"id": 2151, "name": "Sprint A"}, {"id": 2152, "name": "Sprint B"}]})],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(SPRINT, frm="2151", frm_str="Sprint A", to="2151, 2152", to_str="Sprint A, Sprint B")],
            )
        ],
    )
    scenario.build()

    rows = scenario.journal(field=SPRINT)
    assert [r["value_ids"] for r in rows] == [["2151"], ["2151", "2152"]]
    assert [r["value_displays"] for r in rows] == [["Sprint A"], ["Sprint A", "Sprint B"]]
    assert scenario.round_trip_holds()


def test_a_sprint_name_containing_comma_space_falls_back_to_the_ids(scenario):
    """A sprint NAME can itself contain `", "`, so splitting the display side by
    it over-splits. The id side is numeric and unambiguous, so it decides, and
    the displays follow only when the two agree on length."""
    scenario.seed(
        fields=[SPRINT_FIELD],
        issues=[issue("TST-1", fields={SPRINT: [{"id": 2153, "name": "Q1, week 3"}]})],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(SPRINT, frm=None, frm_str=None, to="2153", to_str="Q1, week 3")],
            )
        ],
    )
    scenario.build()

    changed = scenario.journal(field=SPRINT)[-1]
    assert changed["value_ids"] == ["2153"]
    assert changed["value_displays"] == ["2153"]
    assert scenario.round_trip_holds()

"""Multi-value fields whose changelog item carries ONE element.

Components, versions, issue links and attachments are system fields with
first-class changelog support: adding one emits an item with only a `to` side,
removing one an item with only a `from`. This is the single kind whose state
accumulates, so it is the only one where a mis-read event can corrupt a later
one — and the only one where the value at creation has to be reconstructed by
undoing the whole chain.

    initial      = (current ∪ every removal) \\ every addition
    state at k   = (initial ∪ additions up to k) \\ removals up to k
"""

from __future__ import annotations

from helpers import event, field, issue, item

COMPONENTS = "components"
COMPONENTS_FIELD = field(COMPONENTS, name="Components", schema_type="array", schema_items="component")


def _add(cid: str, name: str) -> dict:
    return item(COMPONENTS, to=cid, to_str=name)


def _remove(cid: str, name: str) -> dict:
    return item(COMPONENTS, frm=cid, frm_str=name)


def test_elements_present_with_no_events_are_the_initial_state(scenario):
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: [{"id": "501", "name": "api"}, {"id": "502", "name": "storage"}]})],
    )
    scenario.build()

    assert scenario.states(COMPONENTS) == [["501", "502"]]
    assert scenario.round_trip_holds()


def test_two_additions_accumulate(scenario):
    """Each row holds the state AFTER its event, not the element that changed —
    that is what lets a consumer read any point in time without folding."""
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: [{"id": "501", "name": "api"}, {"id": "502", "name": "storage"}]})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [_add("501", "api")]),
            event("TST-1", 102, "2026-01-07T10:00:00", [_add("502", "storage")]),
        ],
    )
    scenario.build()

    assert scenario.states(COMPONENTS) == [[], ["501"], ["501", "502"]]
    assert scenario.round_trip_holds()


def test_two_elements_added_in_one_entry_make_one_row(scenario):
    """One changelog entry can touch several elements: Jira emits one item per
    element under the same changelog id. The entry is one user action and the
    contract orders events by `event_id`, so the journal holds ONE row for it,
    carrying the state after every item — two rows with one id would be
    indistinguishable to a reader, and the ReplacingMergeTree would keep one
    of them anyway."""
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: [{"id": "501", "name": "api"}, {"id": "502", "name": "storage"}]})],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [_add("501", "api"), _add("502", "storage")])],
    )
    scenario.build()

    rows = scenario.journal(field=COMPONENTS)
    assert [r["event_id"] for r in rows if r["event_kind"] == "changelog"] == ["101"]
    assert scenario.states(COMPONENTS) == [[], ["501", "502"]]
    assert [r["delta_action"] for r in rows if r["event_kind"] == "changelog"] == ["add"]
    assert scenario.round_trip_holds()


def test_an_entry_that_removes_and_adds_is_a_replacement(scenario):
    """Swapping one component for another is one entry with a removal item and
    an addition item. Neither verb describes the whole entry, so the row says
    `set` — the contract's word for a multi-value state given in full."""
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: [{"id": "502", "name": "storage"}]})],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [_remove("501", "api"), _add("502", "storage")])],
    )
    scenario.build()

    rows = scenario.journal(field=COMPONENTS)
    assert scenario.states(COMPONENTS) == [["501"], ["502"]]
    assert [r["delta_action"] for r in rows if r["event_kind"] == "changelog"] == ["set"]
    assert scenario.round_trip_holds()


def test_additions_and_removals_interleave(scenario):
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: [{"id": "502", "name": "storage"}]})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [_add("501", "api")]),
            event("TST-1", 102, "2026-01-07T10:00:00", [_add("502", "storage")]),
            event("TST-1", 103, "2026-01-08T10:00:00", [_remove("501", "api")]),
        ],
    )
    scenario.build()

    assert scenario.states(COMPONENTS) == [[], ["501"], ["501", "502"], ["502"]]
    assert scenario.round_trip_holds()


def test_an_element_removed_then_a_different_one_added(scenario):
    """The reconstruction has to put back what was removed and take away what
    was added, or the issue looks as though it was created with today's set."""
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: [{"id": "503", "name": "billing"}]})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [_remove("501", "api")]),
            event("TST-1", 102, "2026-01-07T10:00:00", [_add("503", "billing")]),
        ],
    )
    scenario.build()

    assert scenario.states(COMPONENTS) == [["501"], [], ["503"]]
    assert scenario.round_trip_holds()


def test_a_field_emptied_by_removals_ends_empty(scenario):
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: []})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [_add("501", "api")]),
            event("TST-1", 102, "2026-01-07T10:00:00", [_remove("501", "api")]),
        ],
    )
    scenario.build()

    assert scenario.states(COMPONENTS) == [[], ["501"], []]
    assert scenario.round_trip_holds()


def test_the_same_element_added_twice_counts_once(scenario):
    """Jira can emit a redundant addition. Counting it twice would make a
    "components per issue" metric wrong without any array looking malformed."""
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: [{"id": "501", "name": "api"}]})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [_add("501", "api")]),
            event("TST-1", 102, "2026-01-07T10:00:00", [_add("501", "api")]),
        ],
    )
    scenario.build()

    assert scenario.states(COMPONENTS) == [[], ["501"], ["501"]]
    assert scenario.round_trip_holds()


def test_an_element_renamed_since_the_event_is_still_the_same_element(scenario):
    """A component renamed after it was added arrives with one name in the
    changelog and another in the issue JSON. It is one element, identified by
    its id, so the addition must still be undone when reconstructing the
    initial state.

    The round trip cannot see this: it compares ids, and the id set is right
    either way. Only the initial row is wrong — the issue would look as though
    it was created carrying a component that was added later.
    """
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: [{"id": "501", "name": "platform"}]})],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [_add("501", "api")])],
    )
    scenario.build()

    assert scenario.states(COMPONENTS) == [[], ["501"]]
    assert scenario.round_trip_holds()


# ── issue links: element-wise, but the two sides name the element differently ──

LINKS = "issuelinks"
LINKS_FIELD = field(LINKS, name="Linked Issues", schema_type="array", schema_items="issuelinks")


def _link(key: str, sentence: str, *, removed: bool = False) -> dict:
    """One changelog item for a link. Jira puts the LINKED ISSUE's key in the id
    side and a rendered sentence in the display side."""
    if removed:
        return item(LINKS, frm=key, frm_str=sentence)
    return item(LINKS, to=key, to_str=sentence)


def test_a_link_is_identified_by_the_linked_issue_key(scenario):
    """The issue resource names the LINK OBJECT by its own id, with the linked
    issue's key nested inside; the changelog names the LINKED ISSUE by key and
    never mentions the link's id at all.

    Identifying by the link id — which is what a component-shaped normalizer
    does — leaves the two sides with no id in common, so every issue holding a
    link disagrees with its own history. The key is the only identifier both
    sides carry.
    """
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[
            issue(
                "TST-1",
                fields={
                    LINKS: [
                        {
                            "id": "157916",
                            "type": {"name": "Duplicate"},
                            "outwardIssue": {"key": "TST-9", "fields": {"summary": "other"}},
                        }
                    ]
                },
            )
        ],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [_link("TST-9", "This issue duplicates TST-9")])],
    )
    scenario.build()

    rows = scenario.journal(field=LINKS)
    assert [r["value_ids"] for r in rows] == [[], ["TST-9"]]
    # The rendered sentence is the display where the changelog supplies one; the
    # resource has no equivalent text, so its row carries the key.
    assert rows[-1]["value_displays"] == ["This issue duplicates TST-9"]
    assert rows[-1]["value_id_type"] == "string_literal"
    assert scenario.round_trip_holds()


def test_an_inward_link_is_the_same_shape(scenario):
    """Direction is a property of the link, not of the element: an issue holds
    the link once either way, and only the rendered text says which side it is
    on."""
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[
            issue(
                "TST-1", fields={LINKS: [{"id": "158190", "type": {"name": "Blocks"}, "inwardIssue": {"key": "TST-8"}}]}
            )
        ],
        events=[event("TST-1", 101, "2026-01-06T10:00:00", [_link("TST-8", "This issue is blocked by TST-8")])],
    )
    scenario.build()

    assert scenario.states(LINKS) == [[], ["TST-8"]]
    assert scenario.round_trip_holds()


def test_links_added_and_removed_accumulate(scenario):
    """Links are element-wise like components, so they fold the same way."""
    scenario.seed(
        fields=[LINKS_FIELD],
        issues=[
            issue("TST-1", fields={LINKS: [{"id": "3", "type": {"name": "Relates"}, "outwardIssue": {"key": "TST-8"}}]})
        ],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [_link("TST-9", "This issue duplicates TST-9")]),
            event("TST-1", 102, "2026-01-07T10:00:00", [_link("TST-8", "This issue relates to TST-8")]),
            event("TST-1", 103, "2026-01-08T10:00:00", [_link("TST-9", "This issue duplicates TST-9", removed=True)]),
        ],
    )
    scenario.build()

    assert scenario.states(LINKS) == [[], ["TST-9"], ["TST-9", "TST-8"], ["TST-8"]]
    assert scenario.round_trip_holds()


SUBTASKS = "subtasks"
SUBTASKS_FIELD = field(SUBTASKS, name="Sub-tasks", schema_type="array", schema_items="issuelinks")


def test_subtasks_are_identified_by_the_child_key(scenario):
    """Jira reports `subtasks` with the same `schema_items` as `issuelinks`, so
    it lands in the same kind — but its element is the referenced issue itself,
    with the key at the top level and no link object around it.

    Probing only the link-object paths normalizes the whole field to empty
    strings: parallel arrays of nothing, which every array-shape test accepts
    and only the round trip catches.
    """
    scenario.seed(
        fields=[SUBTASKS_FIELD],
        issues=[
            issue(
                "TST-1",
                fields={
                    SUBTASKS: [
                        {"id": "1", "key": "TST-2", "fields": {"summary": "first child"}},
                        {"id": "2", "key": "TST-3", "fields": {"summary": "second child"}},
                    ]
                },
            )
        ],
    )
    scenario.build()

    rows = scenario.journal(field=SUBTASKS)
    assert [r["value_ids"] for r in rows] == [["TST-2", "TST-3"]]
    assert rows[0]["value_displays"] == ["TST-2", "TST-3"]
    assert scenario.round_trip_holds()


def test_an_element_removed_and_added_again_comes_back(scenario):
    """The case set arithmetic cannot express.

    `(initial ∪ additions up to k) \\ removals up to k` subtracts an element for
    good once it appears in "every removal", so a link or a component that was
    removed and later re-added is missing from every state after the removal —
    including the newest one, which then contradicts the issue. The fold applies
    the operations in order instead, so the re-add restores it.
    """
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: [{"id": "501", "name": "api"}, {"id": "502", "name": "storage"}]})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [_add("501", "api")]),
            event("TST-1", 102, "2026-01-07T10:00:00", [_add("502", "storage")]),
            event("TST-1", 103, "2026-01-08T10:00:00", [_remove("501", "api")]),
            event("TST-1", 104, "2026-01-09T10:00:00", [_add("501", "api")]),
        ],
    )
    scenario.build()

    assert scenario.states(COMPONENTS) == [[], ["501"], ["501", "502"], ["502"], ["502", "501"]]
    assert scenario.round_trip_holds()


def test_a_re_added_element_carries_the_later_display(scenario):
    """An `add` replaces the element's entry rather than appending a second one,
    so the display is the one the latest event rendered — and the element is
    still counted once."""
    scenario.seed(
        fields=[COMPONENTS_FIELD],
        issues=[issue("TST-1", fields={COMPONENTS: [{"id": "501", "name": "platform"}]})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [_add("501", "api")]),
            event("TST-1", 102, "2026-01-07T10:00:00", [_remove("501", "api")]),
            event("TST-1", 103, "2026-01-08T10:00:00", [_add("501", "platform")]),
        ],
    )
    scenario.build()

    rows = scenario.journal(field=COMPONENTS)
    assert [r["value_ids"] for r in rows] == [[], ["501"], [], ["501"]]
    assert rows[-1]["value_displays"] == ["platform"]
    assert scenario.round_trip_holds()

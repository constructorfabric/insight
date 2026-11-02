"""Pure rules for reporting new page views without folder data or duplicate routes."""

import pytest

from .navigation_report import NavigationPage, is_dashboard_folder, new_page_paths

pytestmark = pytest.mark.reliability


@pytest.mark.parametrize(
    ("href", "expected"),
    [
        (None, False),
        ("/portal/custom", False),
        ("/portal/custom?owned=me", False),
        ("/portal/custom?folder=unfiled", False),
        ("/portal/custom?folder=", False),
        ("/portal/custom?folder=example-folder", True),
        ("/portal/custom/?tag=example&folder=example%20folder", True),
        ("https://example.com/portal/custom?folder=example-folder", True),
        ("/portal/custom/metrics?folder=example-folder", False),
    ],
)
def test_folder_links_are_identified_by_the_dashboard_route_and_folder_parameter(
    href: str | None, expected: bool
) -> None:
    assert is_dashboard_folder(href) == expected, f"href: {href!r}"


@pytest.mark.parametrize(
    ("label", "page_id", "expected"),
    [
        ("Details", "page-example-parent", []),
        ("Overview", "page-example-child", ["Example > Parent > Overview"]),
    ],
)
def test_aliases_are_decided_by_the_rendered_page_id_instead_of_the_label(
    label: str, page_id: str, expected: list[str]
) -> None:
    page = NavigationPage(("Example", "Parent", label), frozenset({page_id}))

    assert new_page_paths([page], {"page-example-parent"}) == expected


def test_dashboard_filters_already_covered_by_another_route_are_not_reported() -> None:
    page = NavigationPage(("Dashboards", "Example filter"), frozenset({"page-custom-dashboards"}))

    assert new_page_paths([page], {"page-custom-dashboards"}) == []


def test_a_new_parent_page_is_reported_once_when_its_submenu_opens_the_same_page() -> None:
    page_ids = frozenset({"page-example-parent"})
    parent = NavigationPage(("Example", "Parent"), page_ids)
    child = NavigationPage(("Example", "Parent", "Details"), page_ids)

    assert new_page_paths([parent, child], ()) == ["Example > Parent"]


def test_discoveries_from_multiple_personas_are_merged_by_route_and_page_id() -> None:
    pages = [
        NavigationPage(("Example", "New page"), frozenset({"page-example-new"})),
        NavigationPage(("Example", "New page"), frozenset({"page-example-new"})),
        NavigationPage(("Other", "Alias"), frozenset({"page-example-new"})),
        NavigationPage(("Example", "Other page"), frozenset({"page-example-other"})),
    ]

    assert new_page_paths(pages, ()) == ["Example > New page", "Example > Other page"]


def test_a_new_visible_page_is_not_hidden_by_a_previously_covered_page_id() -> None:
    page = NavigationPage(
        ("Example", "New page"), frozenset({"page-example-known", "page-example-new"})
    )

    assert new_page_paths([page], {"page-example-known"}) == ["Example > New page"]


def test_unresolved_targets_are_reported_once_without_claiming_they_are_aliases() -> None:
    page = NavigationPage(("Example", "Unresolved"), frozenset())

    assert new_page_paths([page, page], ()) == ["Example > Unresolved"]

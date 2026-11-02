"""Read-only navigation orchestration and catalogue comparison."""

from playwright.sync_api import Error, expect

from .catalogue import PAGE_SCENARIOS, ZONES, PageScenario
from .checks import Checks, mark_page_requests, wait_for_quiet
from .navigation_report import NavigationPage, is_dashboard_folder
from .page_watch import PageWatch
from .pages.portal_shell import PortalShell, navigation_entries
from .steps import step


def expand_entry(portal: PortalShell, parent: str) -> None:
    entry = portal.entry(portal.pane_buttons, parent)
    if entry.get_attribute("aria-expanded") == "false":
        portal.open_pane_entry(parent)


def open_scenario(portal: PortalShell, watch: PageWatch, scenario: PageScenario) -> None:
    portal.go()
    portal.open_zone(scenario.zone)
    if len(scenario.path) == 1 and scenario.first is None:
        mark_page_requests(portal, watch)
    portal.open_pane_entry(scenario.path[0])

    if len(scenario.path) == 2:
        expand_entry(portal, scenario.path[0])
        mark_page_requests(portal, watch)
        portal.open_sub_entry(scenario.path[0], scenario.path[1])

    if scenario.first:
        with step(f"Find the first item in the {scenario.path[0]} list"):
            href = portal.first_list_href(scenario.first)
        mark_page_requests(portal, watch)
        portal.open_list_item(scenario.path[0], href)


def identify_page(portal: PortalShell, watch: PageWatch, path: tuple[str, ...]) -> NavigationPage:
    with step(f"Identify the page opened by {' > '.join(path)}"):
        try:
            settled = wait_for_quiet(portal, watch, loaded=True)
            page_ids = portal.visible_page_ids() if settled else frozenset()
        except Error:
            page_ids = frozenset()
        return NavigationPage(path=path, page_ids=page_ids)


def discover_entry(portal: PortalShell, watch: PageWatch, path: tuple[str, ...]) -> NavigationPage:
    try:
        portal.open_zone(path[0])
        if len(path) == 3:
            expand_entry(portal, path[1])
            portal.open_sub_entry(path[1], path[2])
        else:
            portal.open_pane_entry(path[1])
    except Error:
        return NavigationPage(path=path, page_ids=frozenset())
    return identify_page(portal, watch, path)


def verify_entries(
    portal: PortalShell,
    watch: PageWatch,
    zone: str,
    checks: Checks,
    scenarios: tuple[PageScenario, ...],
) -> list[NavigationPage]:
    paths = [scenario.path for scenario in scenarios if scenario.zone == zone]
    catalogue_paths = [scenario.path for scenario in PAGE_SCENARIOS if scenario.zone == zone]
    catalogue_labels = {path[0] for path in catalogue_paths}
    expected = list(dict.fromkeys(path[0] for path in paths))
    for label in expected:
        with checks.check(f'Verify "{label}" is visible in the {zone} pane'):
            expect(portal.entry(portal.pane_buttons, label)).to_be_visible(timeout=30_000)

    entries = [
        entry
        for entry in navigation_entries(portal.pane_buttons)
        if not is_dashboard_folder(entry.href)
    ]
    extras: list[NavigationPage] = []
    for entry in entries:
        if entry.label in catalogue_labels or entry.expanded is not None:
            continue
        extras.append(discover_entry(portal, watch, (zone, entry.label)))

    parents = dict.fromkeys(path[0] for path in paths if len(path) == 2)
    parents.update(dict.fromkeys(entry.label for entry in entries if entry.expanded is not None))
    for parent in parents:
        with checks.check(f'Open the sub-menu under "{parent}"'):
            portal.open_zone(zone)
            expand_entry(portal, parent)
        parent_page = identify_page(portal, watch, (zone, parent))
        if parent not in catalogue_labels:
            extras.append(parent_page)
        children = [path[1] for path in paths if len(path) == 2 and path[0] == parent]
        catalogue_children = {
            path[1] for path in catalogue_paths if len(path) == 2 and path[0] == parent
        }
        buttons = portal.sub_buttons(parent)
        for label in children:
            with checks.check(f'Verify "{label}" is visible under "{parent}"'):
                expect(portal.entry(buttons, label)).to_be_visible(timeout=30_000)
        for entry in navigation_entries(buttons):
            if entry.label in catalogue_children or is_dashboard_folder(entry.href):
                continue
            child_page = discover_entry(portal, watch, (zone, parent, entry.label))
            if child_page.page_ids and child_page.page_ids == parent_page.page_ids:
                continue
            extras.append(child_page)

    return extras


def verify_navigation(
    portal: PortalShell,
    watch: PageWatch,
    checks: Checks,
    scenarios: tuple[PageScenario, ...],
) -> list[NavigationPage]:
    portal.go()
    with checks.check("Verify the Insight portal opened"):
        expect(portal.rail.zones().first).to_be_visible(timeout=30_000)
    extras = [
        NavigationPage(path=(entry.label,), page_ids=frozenset())
        for entry in navigation_entries(portal.safe_navigation(portal.rail.zones()))
        if entry.label not in ZONES
    ]
    zones = dict.fromkeys(scenario.zone for scenario in scenarios)
    for zone in zones:
        entry = portal.entry(portal.rail.zones(), zone)
        with checks.check(f"Verify the {zone} zone is visible"):
            expect(entry).to_be_visible(timeout=30_000)
        if not entry.is_visible():
            continue
        with checks.check(f"Visit the {zone} navigation"):
            portal.open_zone(zone)
            extras.extend(verify_entries(portal, watch, zone, checks, scenarios))

    return extras

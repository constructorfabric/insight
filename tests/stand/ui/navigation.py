"""Read-only navigation orchestration and catalogue comparison."""

from playwright.sync_api import expect

from .catalogue import PAGE_SCENARIOS, ZONES, PageScenario
from .checks import Checks, mark_page_requests
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


def verify_entries(
    portal: PortalShell, zone: str, checks: Checks, scenarios: tuple[PageScenario, ...]
) -> list[str]:
    paths = [scenario.path for scenario in scenarios if scenario.zone == zone]
    catalogue_paths = [scenario.path for scenario in PAGE_SCENARIOS if scenario.zone == zone]
    catalogue_labels = {path[0] for path in catalogue_paths}
    expected = list(dict.fromkeys(path[0] for path in paths))
    for label in expected:
        with checks.check(f'Verify "{label}" is visible in the {zone} pane'):
            expect(portal.entry(portal.pane_buttons, label)).to_be_visible(timeout=30_000)

    entries = navigation_entries(portal.pane_buttons)
    extras = [f"{zone} > {entry.label}" for entry in entries if entry.label not in catalogue_labels]
    parents = dict.fromkeys(path[0] for path in paths if len(path) == 2)
    parents.update(dict.fromkeys(entry.label for entry in entries if entry.expanded is not None))
    for parent in parents:
        with checks.check(f'Open the sub-menu under "{parent}"'):
            expand_entry(portal, parent)
        children = [path[1] for path in paths if len(path) == 2 and path[0] == parent]
        catalogue_children = {
            path[1] for path in catalogue_paths if len(path) == 2 and path[0] == parent
        }
        buttons = portal.sub_buttons(parent)
        for label in children:
            with checks.check(f'Verify "{label}" is visible under "{parent}"'):
                expect(portal.entry(buttons, label)).to_be_visible(timeout=30_000)
        extras.extend(
            f"{zone} > {parent} > {entry.label}"
            for entry in navigation_entries(buttons)
            if entry.label not in catalogue_children
        )

    return extras


def verify_navigation(
    portal: PortalShell, checks: Checks, scenarios: tuple[PageScenario, ...]
) -> list[str]:
    portal.go()
    with checks.check("Verify the Insight portal opened"):
        expect(portal.rail.zones().first).to_be_visible(timeout=30_000)
    extras = [
        entry.label
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
            extras.extend(verify_entries(portal, zone, checks, scenarios))

    return extras

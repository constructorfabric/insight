"""Read-only navigation checks and discovery of additional page views."""

from playwright.sync_api import Error, expect

from .checks import Checks, wait_for_quiet
from .navigation_report import NavigationPage, is_dashboard_folder
from .page_watch import PageWatch
from .pages.portal_shell import PortalShell, navigation_entries
from .steps import step


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
            portal.expand_pane_entry(path[1])
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
    expected: dict[str, tuple[str, ...]],
) -> list[NavigationPage]:
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
        if entry.label in expected or entry.expanded is not None:
            continue
        extras.append(discover_entry(portal, watch, (zone, entry.label)))

    parents = dict.fromkeys(label for label, children in expected.items() if children)
    parents.update(dict.fromkeys(entry.label for entry in entries if entry.expanded is not None))
    for parent in parents:
        with checks.check(f'Open the sub-menu under "{parent}"'):
            portal.open_zone(zone)
            portal.expand_pane_entry(parent)
        parent_page = identify_page(portal, watch, (zone, parent))
        if parent not in expected:
            extras.append(parent_page)
        children = expected.get(parent, ())
        buttons = portal.sub_buttons(parent)
        for label in children:
            with checks.check(f'Verify "{label}" is visible under "{parent}"'):
                expect(portal.entry(buttons, label)).to_be_visible(timeout=30_000)
        for entry in navigation_entries(buttons):
            if entry.label in children or is_dashboard_folder(entry.href):
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
    expected: dict[str, dict[str, tuple[str, ...]]],
) -> list[NavigationPage]:
    with checks.check("Verify the Insight portal opened"):
        expect(portal.rail.zones().first).to_be_visible(timeout=30_000)
    extras = [
        NavigationPage(path=(entry.label,), page_ids=frozenset())
        for entry in navigation_entries(portal.safe_navigation(portal.rail.zones()))
        if entry.label not in expected
    ]
    for zone, entries in expected.items():
        entry = portal.entry(portal.rail.zones(), zone)
        with checks.check(f"Verify the {zone} zone is visible"):
            expect(entry).to_be_visible(timeout=30_000)
        if not entry.is_visible():
            continue
        with checks.check(f"Visit the {zone} navigation"):
            portal.open_zone(zone)
            extras.extend(verify_entries(portal, watch, zone, checks, entries))

    return extras

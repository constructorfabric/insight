"""Every catalogued portal page loads and renders without request or widget errors.

Why this needs a browser: SPA navigation changes the selected zone, direction
and lens in the client. The frontend renders page-specific test ids, loading
markers and widget error statuses; HTTP route responses cannot prove that
rendering completed. Request observation begins at the final navigation hop
so earlier zone and list traffic does not decide the page's verdict.
"""

from collections.abc import Callable

import pytest
from insight_stand import Manifest

from .catalogue import PAGE_SCENARIOS, PERSONAS, PageScenario, scenarios_for
from .checks import Checks, verify_page
from .navigation import open_scenario, verify_navigation
from .navigation_report import NavigationPage, new_page_paths
from .page_watch import PageWatch
from .pages.portal_shell import PortalShell
from .steps import report_metadata, step

pytestmark = [pytest.mark.reliability, pytest.mark.stand_smoke]


@pytest.mark.parametrize(
    "scenario",
    [
        pytest.param(
            scenario,
            id=scenario.title,
            marks=pytest.mark.requires_seed(scenario.persona),
        )
        for scenario in PAGE_SCENARIOS
    ],
)
def test_portal_page_loads_without_errors(
    portal_smoke_for: Callable[[str], tuple[PortalShell, PageWatch]], scenario: PageScenario
) -> None:
    report_metadata(scenario.title, feature="Portal pages", story=scenario.zone)
    portal, watch = portal_smoke_for(scenario.persona)
    open_scenario(portal, watch, scenario)
    verify_page(portal, watch, scenario)


@pytest.mark.requires_seed(*PERSONAS)
def test_navigation_shows_every_expected_zone_and_entry(
    portal_smoke_for: Callable[[str], tuple[PortalShell, PageWatch]],
    record_property: Callable[[str, object], None],
    navigation_extras: list[str],
    stand_manifest: Manifest,
) -> None:
    report_metadata(
        "Navigation shows every expected zone and entry",
        feature="Portal pages",
        story="Navigation",
    )
    checks = Checks()
    discoveries: list[NavigationPage] = []
    for persona in PERSONAS:
        scenarios = scenarios_for(persona, stand_manifest.capabilities)
        if not scenarios:
            continue
        with checks.check(f"Verify navigation for the {persona} persona"):
            portal, watch = portal_smoke_for(persona)
            discoveries.extend(verify_navigation(portal, watch, checks, scenarios))
    with step("Report navigation entries absent from the catalogue"):
        extras = new_page_paths(discoveries, (scenario.test_id for scenario in PAGE_SCENARIOS))
        navigation_extras.extend(extras)
        record_property("Navigation entries absent from the catalogue", extras)
    checks.finish()

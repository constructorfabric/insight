from collections.abc import Callable

import pytest

from .checks import Checks
from .navigation import verify_navigation
from .navigation_report import new_page_paths
from .portal import PortalPage
from .steps import step

pytestmark = [pytest.mark.reliability, pytest.mark.stand_smoke]


@pytest.mark.requires_seed("ceo")
def test_navigation_shows_every_expected_zone_and_entry(
    ceo_portal: PortalPage,
    record_property: Callable[[str, object], None],
    navigation_extras: list[str],
) -> None:
    expected: dict[str, dict[str, tuple[str, ...]]] = {
        "Home": {
            "At a glance": (),
            "By direction": (),
            "Attention needed": (),
            "Data coverage": (),
            "Contribution breakdown": (),
        },
        "Explore": {
            "Development": ("Git output", "Flow", "Repositories"),
            "Collaboration": (
                "Messaging",
                "Meetings",
                "Email",
                "Time outside meetings",
            ),
            "Overview": (),
            "AI adoption": ("By unit / role",),
        },
        "People": {
            "Me": (),
            "Task delivery": (),
            "Git output": (),
            "Collaboration": (),
            "AI adoption": (),
            "My team": (),
            "Roster · by role": (),
        },
        "Reports": {
            "Report builder": (),
        },
        "Manage": {
            "What's new": (),
        },
    }
    conditional: dict[str, dict[str, tuple[str, ...]]] = {
        "Home": {
            "My dashboards": (),
        },
        "Dashboards": {
            "All dashboards": (),
            "Shared with me": (),
            "Unfiled": (),
            "Metrics": (),
            "Widgets": (),
            "Datasets": (),
            "Alerts": (),
        },
        "Explore": {
            "Collaboration": ("Files & sharing",),
        },
        "Manage": {
            "Sources & connectors": (),
            "Identities · roles & taxonomy": (),
            "Platform usage": (),
        },
    }
    covered_page_ids = {
        "page-overview-at-a-glance",
        "page-overview-by-direction",
        "page-overview-attention",
        "page-overview-health",
        "page-overview-contribution",
        "page-custom-dashboards",
        "page-custom-dashboard",
        "page-custom-metrics",
        "page-custom-metric",
        "page-custom-widgets",
        "page-custom-widget",
        "page-custom-datasets",
        "page-custom-dataset",
        "page-custom-alerts",
        "page-directions-dev-overview",
        "page-directions-dev-git-output",
        "page-directions-dev-flow",
        "page-directions-dev-repositories",
        "page-directions-collab-overview",
        "page-directions-collab-messaging",
        "page-directions-collab-meetings",
        "page-directions-collab-email",
        "page-directions-collab-time-outside-meetings",
        "page-directions-collab-files-sharing",
        "page-aicost-overview",
        "page-aicost-adoption-funnel",
        "page-aicost-by-unit-role",
        "page-person-at-a-glance",
        "page-person-task-delivery",
        "page-person-git-output",
        "page-person-collaboration",
        "page-person-ai-adoption",
        "page-people-roster",
        "page-people-employees",
        "page-reports-report-builder",
        "page-manage-connector-health",
        "page-manage-identities",
        "page-manage-whats-new",
        "page-manage-platform-usage",
    }
    checks = Checks()
    discoveries = verify_navigation(
        ceo_portal.shell, ceo_portal.watch, checks, expected, conditional
    )
    with step("Report navigation entries absent from the expected list"):
        extras = new_page_paths(discoveries, covered_page_ids)
        navigation_extras.extend(extras)
        record_property("Navigation entries absent from the expected list", extras)
    checks.finish()

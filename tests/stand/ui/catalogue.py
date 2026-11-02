"""Read-only portal visits, independent of any stand's extra navigation entries."""

from dataclasses import dataclass
from typing import Literal

type PersonaName = Literal["ceo", "admin_operator"]


@dataclass(frozen=True)
class PageScenario:
    zone: str
    path: tuple[str, ...]
    title: str
    test_id: str
    first: str | None = None
    capability: str | None = None
    persona: PersonaName = "ceo"

    @property
    def page_name(self) -> str:
        if self.first:
            return self.path[0].removeprefix("All ").removesuffix("s").lower()
        return self.path[-1]


PAGE_SCENARIOS = (
    PageScenario(
        "Home",
        ("At a glance",),
        "At a glance loads without errors",
        "page-overview-at-a-glance",
    ),
    PageScenario(
        "Home",
        ("By direction",),
        "By direction loads without errors",
        "page-overview-by-direction",
    ),
    PageScenario(
        "Home",
        ("Trend",),
        "Trend loads without errors",
        "page-overview-trend",
    ),
    PageScenario(
        "Home",
        ("Attention needed",),
        "Attention needed loads without errors",
        "page-overview-attention",
    ),
    PageScenario(
        "Home",
        ("Data coverage",),
        "Data coverage loads without errors",
        "page-overview-health",
    ),
    PageScenario(
        "Home",
        ("Contribution breakdown",),
        "Contribution breakdown loads without errors",
        "page-overview-contribution",
    ),
    PageScenario(
        "Home",
        ("My dashboards",),
        "My dashboards from Home loads without errors",
        "page-custom-dashboards",
        persona="admin_operator",
    ),
    PageScenario(
        "Dashboards",
        ("All dashboards",),
        "All dashboards list loads without errors",
        "page-custom-dashboards",
    ),
    PageScenario(
        "Dashboards",
        ("All dashboards",),
        "First dashboard loads without errors",
        "page-custom-dashboard",
        first="/portal/custom/",
        capability="portal_dashboards",
    ),
    PageScenario(
        "Dashboards",
        ("Shared with me",),
        "Shared with me list loads without errors",
        "page-custom-dashboards",
    ),
    PageScenario(
        "Dashboards",
        ("Unfiled",),
        "Unfiled dashboards list loads without errors",
        "page-custom-dashboards",
    ),
    PageScenario(
        "Dashboards",
        ("Metrics",),
        "Metrics list loads without errors",
        "page-custom-metrics",
        persona="admin_operator",
    ),
    PageScenario(
        "Dashboards",
        ("Metrics",),
        "First metric loads without errors",
        "page-custom-metric",
        first="/portal/custom/metrics/",
        capability="portal_metrics",
        persona="admin_operator",
    ),
    PageScenario(
        "Dashboards",
        ("Widgets",),
        "Widgets list loads without errors",
        "page-custom-widgets",
        persona="admin_operator",
    ),
    PageScenario(
        "Dashboards",
        ("Widgets",),
        "First widget loads without errors",
        "page-custom-widget",
        first="/portal/custom/widgets/",
        capability="portal_widgets",
        persona="admin_operator",
    ),
    PageScenario(
        "Dashboards",
        ("Datasets",),
        "Datasets list loads without errors",
        "page-custom-datasets",
        persona="admin_operator",
    ),
    PageScenario(
        "Dashboards",
        ("Datasets",),
        "First dataset loads without errors",
        "page-custom-dataset",
        first="/portal/custom/datasets/",
        capability="portal_datasets",
        persona="admin_operator",
    ),
    PageScenario(
        "Dashboards",
        ("Alerts",),
        "Alerts list loads without errors",
        "page-custom-alerts",
        persona="admin_operator",
    ),
    PageScenario(
        "Explore",
        ("Development",),
        "Development overview loads without errors",
        "page-directions-dev-overview",
    ),
    PageScenario(
        "Explore",
        ("Development", "Git output"),
        "Development Git output loads without errors",
        "page-directions-dev-git-output",
    ),
    PageScenario(
        "Explore",
        ("Development", "Flow"),
        "Development Flow loads without errors",
        "page-directions-dev-flow",
    ),
    PageScenario(
        "Explore",
        ("Development", "Repositories"),
        "Development Repositories loads without errors",
        "page-directions-dev-repositories",
    ),
    PageScenario(
        "Explore",
        ("Collaboration",),
        "Collaboration overview loads without errors",
        "page-directions-collab-overview",
    ),
    PageScenario(
        "Explore",
        ("Collaboration", "Messaging"),
        "Collaboration Messaging loads without errors",
        "page-directions-collab-messaging",
    ),
    PageScenario(
        "Explore",
        ("Collaboration", "Meetings"),
        "Collaboration Meetings loads without errors",
        "page-directions-collab-meetings",
    ),
    PageScenario(
        "Explore",
        ("Collaboration", "Email"),
        "Collaboration Email loads without errors",
        "page-directions-collab-email",
    ),
    PageScenario(
        "Explore",
        ("Collaboration", "Time outside meetings"),
        "Collaboration Time outside meetings loads without errors",
        "page-directions-collab-time-outside-meetings",
    ),
    PageScenario(
        "Explore",
        ("Collaboration", "Files & sharing"),
        "Collaboration Files and sharing loads without errors",
        "page-directions-collab-files-sharing",
    ),
    PageScenario(
        "Explore",
        ("Overview",),
        "AI cost overview loads without errors",
        "page-aicost-overview",
    ),
    PageScenario(
        "Explore",
        ("AI adoption",),
        "AI adoption funnel loads without errors",
        "page-aicost-adoption-funnel",
    ),
    PageScenario(
        "Explore",
        ("AI adoption", "By unit / role"),
        "AI adoption by unit and role loads without errors",
        "page-aicost-by-unit-role",
    ),
    PageScenario(
        "People",
        ("Me",),
        "Me page loads without errors",
        "page-person-at-a-glance",
    ),
    PageScenario(
        "People",
        ("My team",),
        "My team loads without errors",
        "page-people-roster",
    ),
    PageScenario(
        "People",
        ("Roster · by role",),
        "Roster by role loads without errors",
        "page-people-employees",
    ),
    PageScenario(
        "Reports",
        ("Report builder",),
        "Report builder loads without errors",
        "page-reports-report-builder",
    ),
    PageScenario(
        "Manage",
        ("Sources & connectors",),
        "Sources and connectors load without errors",
        "page-manage-connector-health",
        persona="admin_operator",
    ),
    PageScenario(
        "Manage",
        ("Identities · roles & taxonomy",),
        "Identities, roles and taxonomy load without errors",
        "page-manage-identities",
        persona="admin_operator",
    ),
    PageScenario(
        "Manage",
        ("What's new",),
        "What's new loads without errors",
        "page-manage-whats-new",
    ),
    PageScenario(
        "Manage",
        ("Platform usage",),
        "Platform usage loads without errors",
        "page-manage-platform-usage",
        persona="admin_operator",
    ),
)

ZONES = tuple(dict.fromkeys(scenario.zone for scenario in PAGE_SCENARIOS))
PERSONAS = tuple(dict.fromkeys(scenario.persona for scenario in PAGE_SCENARIOS))


def scenarios_for(persona: PersonaName) -> tuple[PageScenario, ...]:
    return tuple(scenario for scenario in PAGE_SCENARIOS if scenario.persona == persona)

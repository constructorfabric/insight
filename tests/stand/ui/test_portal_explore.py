import pytest

from .portal import PortalPage

pytestmark = [pytest.mark.reliability, pytest.mark.stand_smoke]


@pytest.mark.requires_seed("ceo")
def test_development_overview_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("Development")
    ceo_portal.verify_page_works("page-directions-dev-overview")


@pytest.mark.requires_seed("ceo")
def test_development_git_output_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("Development")
    ceo_portal.open_sub_entry("Development", "Git output")
    ceo_portal.verify_page_works("page-directions-dev-git-output")


@pytest.mark.requires_seed("ceo")
def test_development_flow_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("Development")
    ceo_portal.open_sub_entry("Development", "Flow")
    ceo_portal.verify_page_works("page-directions-dev-flow")


@pytest.mark.requires_seed("ceo")
def test_development_repositories_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("Development")
    ceo_portal.open_sub_entry("Development", "Repositories")
    ceo_portal.verify_page_works("page-directions-dev-repositories")


@pytest.mark.requires_seed("ceo")
def test_collaboration_overview_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("Collaboration")
    ceo_portal.verify_page_works("page-directions-collab-overview")


@pytest.mark.requires_seed("ceo")
def test_collaboration_messaging_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("Collaboration")
    ceo_portal.open_sub_entry("Collaboration", "Messaging")
    ceo_portal.verify_page_works("page-directions-collab-messaging")


@pytest.mark.requires_seed("ceo")
def test_collaboration_meetings_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("Collaboration")
    ceo_portal.open_sub_entry("Collaboration", "Meetings")
    ceo_portal.verify_page_works("page-directions-collab-meetings")


@pytest.mark.requires_seed("ceo")
def test_collaboration_email_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("Collaboration")
    ceo_portal.open_sub_entry("Collaboration", "Email")
    ceo_portal.verify_page_works("page-directions-collab-email")


@pytest.mark.requires_seed("ceo")
def test_collaboration_time_outside_meetings_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("Collaboration")
    ceo_portal.open_sub_entry("Collaboration", "Time outside meetings")
    ceo_portal.verify_page_works("page-directions-collab-time-outside-meetings")


@pytest.mark.requires_seed("ceo")
@pytest.mark.requires_files_sharing
def test_collaboration_files_and_sharing_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("Collaboration")
    ceo_portal.open_sub_entry("Collaboration", "Files & sharing")
    ceo_portal.verify_page_works("page-directions-collab-files-sharing")


@pytest.mark.requires_seed("ceo")
def test_ai_cost_overview_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("Overview")
    ceo_portal.verify_page_works("page-aicost-overview")


@pytest.mark.requires_seed("ceo")
def test_ai_adoption_funnel_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("AI adoption")
    ceo_portal.verify_page_works("page-aicost-adoption-funnel")


@pytest.mark.requires_seed("ceo")
def test_ai_adoption_by_unit_and_role_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Explore")
    ceo_portal.open_pane_entry("AI adoption")
    ceo_portal.open_sub_entry("AI adoption", "By unit / role")
    ceo_portal.verify_page_works("page-aicost-by-unit-role")

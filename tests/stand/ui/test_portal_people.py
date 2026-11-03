import pytest

from .portal import PortalPage

pytestmark = [pytest.mark.reliability, pytest.mark.stand_smoke]


@pytest.mark.requires_seed("ceo")
def test_me_page_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("People")
    ceo_portal.open_pane_entry("Me")
    ceo_portal.verify_page_loaded("page-person-at-a-glance")


@pytest.mark.requires_seed("ceo")
def test_person_task_delivery_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("People")
    ceo_portal.open_pane_entry("Task delivery")
    ceo_portal.verify_page_loaded("page-person-task-delivery")


@pytest.mark.requires_seed("ceo")
def test_person_git_output_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("People")
    ceo_portal.open_pane_entry("Git output")
    ceo_portal.verify_page_loaded("page-person-git-output")


@pytest.mark.requires_seed("ceo")
def test_person_collaboration_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("People")
    ceo_portal.open_pane_entry("Collaboration")
    ceo_portal.verify_page_loaded("page-person-collaboration")


@pytest.mark.requires_seed("ceo")
def test_person_ai_adoption_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("People")
    ceo_portal.open_pane_entry("AI adoption")
    ceo_portal.verify_page_loaded("page-person-ai-adoption")


@pytest.mark.requires_seed("ceo")
def test_my_team_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("People")
    ceo_portal.open_pane_entry("My team")
    ceo_portal.verify_page_loaded("page-people-roster")


@pytest.mark.requires_seed("ceo")
def test_roster_by_role_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("People")
    ceo_portal.open_pane_entry("Roster · by role")
    ceo_portal.verify_page_loaded("page-people-employees")

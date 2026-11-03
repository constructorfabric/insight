import pytest

from .portal import PortalPage

pytestmark = [pytest.mark.reliability, pytest.mark.stand_smoke]


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.requires_ingestion
def test_sources_and_connectors_loads(admin_operator_portal: PortalPage) -> None:
    admin_operator_portal.open_zone("Manage")
    admin_operator_portal.open_pane_entry("Sources & connectors")
    admin_operator_portal.verify_page_works("page-manage-connector-health")


@pytest.mark.requires_seed("admin_operator")
def test_identities_roles_and_taxonomy_loads(admin_operator_portal: PortalPage) -> None:
    admin_operator_portal.open_zone("Manage")
    admin_operator_portal.open_pane_entry("Identities · roles & taxonomy")
    admin_operator_portal.verify_page_works("page-manage-identities")


@pytest.mark.requires_seed("ceo")
def test_whats_new_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Manage")
    ceo_portal.open_pane_entry("What's new")
    ceo_portal.verify_page_works("page-manage-whats-new")


@pytest.mark.requires_seed("admin_operator")
def test_platform_usage_loads(admin_operator_portal: PortalPage) -> None:
    admin_operator_portal.open_zone("Manage")
    admin_operator_portal.open_pane_entry("Platform usage")
    admin_operator_portal.verify_page_works("page-manage-platform-usage")

import pytest

from .portal import PortalPage

pytestmark = [pytest.mark.reliability, pytest.mark.stand_smoke]


@pytest.mark.requires_seed("ceo")
def test_at_a_glance_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Home")
    ceo_portal.open_pane_entry("At a glance")
    ceo_portal.verify_page_works("page-overview-at-a-glance")


@pytest.mark.requires_seed("ceo")
def test_by_direction_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Home")
    ceo_portal.open_pane_entry("By direction")
    ceo_portal.verify_page_works("page-overview-by-direction")


@pytest.mark.requires_seed("ceo")
def test_trend_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Home")
    ceo_portal.open_pane_entry("Trend")
    ceo_portal.verify_page_works("page-overview-trend")


@pytest.mark.requires_seed("ceo")
def test_attention_needed_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Home")
    ceo_portal.open_pane_entry("Attention needed")
    ceo_portal.verify_page_works("page-overview-attention")


@pytest.mark.requires_seed("ceo")
def test_data_coverage_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Home")
    ceo_portal.open_pane_entry("Data coverage")
    ceo_portal.verify_page_works("page-overview-health")


@pytest.mark.requires_seed("ceo")
def test_contribution_breakdown_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Home")
    ceo_portal.open_pane_entry("Contribution breakdown")
    ceo_portal.verify_page_works("page-overview-contribution")


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.requires_custom_catalogue
def test_my_dashboards_from_home_loads(admin_operator_portal: PortalPage) -> None:
    admin_operator_portal.open_zone("Home")
    admin_operator_portal.open_pane_entry("My dashboards")
    admin_operator_portal.verify_page_works("page-custom-dashboards")

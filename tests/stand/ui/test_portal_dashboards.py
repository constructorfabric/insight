import pytest

from .portal import PortalPage

pytestmark = [pytest.mark.reliability, pytest.mark.stand_smoke]


@pytest.mark.requires_seed("ceo")
@pytest.mark.requires_custom_catalogue
def test_all_dashboards_list_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Dashboards")
    ceo_portal.open_pane_entry("All dashboards")
    ceo_portal.verify_page_loaded("page-custom-dashboards")


@pytest.mark.requires_seed("ceo")
@pytest.mark.requires_custom_catalogue
@pytest.mark.requires_portal_dashboards
def test_first_dashboard_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Dashboards")
    ceo_portal.open_pane_entry("All dashboards")
    ceo_portal.open_first_item("All dashboards", "/portal/custom/")
    ceo_portal.verify_page_loaded("page-custom-dashboard")


@pytest.mark.requires_seed("ceo")
@pytest.mark.requires_custom_catalogue
def test_shared_with_me_list_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Dashboards")
    ceo_portal.open_pane_entry("Shared with me")
    ceo_portal.verify_page_loaded("page-custom-dashboards")


@pytest.mark.requires_seed("ceo")
@pytest.mark.requires_custom_catalogue
def test_unfiled_dashboards_list_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Dashboards")
    ceo_portal.open_pane_entry("Unfiled")
    ceo_portal.verify_page_loaded("page-custom-dashboards")


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.requires_custom_catalogue
def test_metrics_list_loads(admin_operator_portal: PortalPage) -> None:
    admin_operator_portal.open_zone("Dashboards")
    admin_operator_portal.open_pane_entry("Metrics")
    admin_operator_portal.verify_page_loaded("page-custom-metrics")


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.requires_custom_catalogue
@pytest.mark.requires_portal_metrics
def test_first_metric_loads(admin_operator_portal: PortalPage) -> None:
    admin_operator_portal.open_zone("Dashboards")
    admin_operator_portal.open_pane_entry("Metrics")
    admin_operator_portal.open_first_item("Metrics", "/portal/custom/metrics/")
    admin_operator_portal.verify_page_loaded("page-custom-metric")


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.requires_custom_catalogue
def test_widgets_list_loads(admin_operator_portal: PortalPage) -> None:
    admin_operator_portal.open_zone("Dashboards")
    admin_operator_portal.open_pane_entry("Widgets")
    admin_operator_portal.verify_page_loaded("page-custom-widgets")


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.requires_custom_catalogue
@pytest.mark.requires_portal_widgets
def test_first_widget_loads(admin_operator_portal: PortalPage) -> None:
    admin_operator_portal.open_zone("Dashboards")
    admin_operator_portal.open_pane_entry("Widgets")
    admin_operator_portal.open_first_item("Widgets", "/portal/custom/widgets/")
    admin_operator_portal.verify_page_loaded("page-custom-widget")


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.requires_custom_catalogue
def test_datasets_list_loads(admin_operator_portal: PortalPage) -> None:
    admin_operator_portal.open_zone("Dashboards")
    admin_operator_portal.open_pane_entry("Datasets")
    admin_operator_portal.verify_page_loaded("page-custom-datasets")


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.requires_custom_catalogue
@pytest.mark.requires_portal_datasets
def test_first_dataset_loads(admin_operator_portal: PortalPage) -> None:
    admin_operator_portal.open_zone("Dashboards")
    admin_operator_portal.open_pane_entry("Datasets")
    admin_operator_portal.open_first_item("Datasets", "/portal/custom/datasets/")
    admin_operator_portal.verify_page_loaded("page-custom-dataset")


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.requires_custom_catalogue
def test_alerts_list_loads(admin_operator_portal: PortalPage) -> None:
    admin_operator_portal.open_zone("Dashboards")
    admin_operator_portal.open_pane_entry("Alerts")
    admin_operator_portal.verify_page_loaded("page-custom-alerts")

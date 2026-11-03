import pytest

from .portal import PortalPage

pytestmark = [pytest.mark.reliability, pytest.mark.stand_smoke]


@pytest.mark.requires_seed("ceo")
def test_report_builder_loads(ceo_portal: PortalPage) -> None:
    ceo_portal.open_zone("Reports")
    ceo_portal.open_pane_entry("Report builder")
    ceo_portal.verify_page_works("page-reports-report-builder")

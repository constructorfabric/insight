from urllib.parse import quote

import pytest
from api.schemas import ConnectorHealthResponse
from insight_stand import Manifest
from playwright.sync_api import expect
from pydantic import BaseModel

from .portal import PortalPage

pytestmark = pytest.mark.reliability


class SavedCrons(BaseModel):
    crons: list[str]


class SavedSchedule(BaseModel):
    requested: SavedCrons | None = None


@pytest.mark.requires_seed("admin_operator")
def test_a_refused_schedule_shows_the_server_reason_and_preserves_the_saved_value(
    admin_operator_portal: PortalPage, stand_manifest: Manifest
) -> None:
    if stand_manifest.capabilities.ingestion:
        pytest.skip("The no-cluster schedule refusal is a compose scenario.")

    portal = admin_operator_portal
    page = portal.shell.page
    response = page.request.get("/api/connectors/v1/connector-health")
    assert response.status == 200
    summary = ConnectorHealthResponse.model_validate(response.json())
    instances = [row for row in summary.connectors if row.configured and row.source_id]
    assert instances, "The compose schedule fixture must include a configured instance."
    instance = instances[0]
    assert instance.source_id is not None
    path = (
        f"/api/connectors/v1/connectors/{quote(instance.connector, safe='')}/"
        f"{quote(instance.source_id, safe='')}/schedule"
    )
    original = page.request.get(path)
    assert original.status == 200
    saved = SavedSchedule.model_validate(original.json())

    portal.open_zone("Manage")
    portal.open_pane_entry("Connectors")
    portal.verify_page_loaded("page-manage-connector-health")
    row = page.get_by_role("row", expanded=False).filter(
        has=page.get_by_role("cell", name=instance.connector, exact=True)
    )
    row = row.filter(has=page.get_by_text(instance.source_id, exact=True))
    row.click()
    page.get_by_role("button", name="Edit", exact=True).click()
    page.get_by_role("combobox", name="Runs", exact=True).click()
    page.get_by_role("option", name="Custom cron (UTC)", exact=True).click()
    while page.get_by_role("button", name="Remove expression 2", exact=True).count():
        page.get_by_role("button", name="Remove expression 2", exact=True).click()
    cron = page.get_by_role("textbox", name="Cron expression 1", exact=True)
    requested = "0 7 * * *" if cron.input_value() == "0 6 * * *" else "0 6 * * *"
    cron.fill(requested)
    with page.expect_response(
        lambda reply: reply.url.endswith(path) and reply.request.method == "PUT"
    ) as write:
        page.get_by_role("button", name="Save", exact=True).click()
    assert write.value.status == 503
    refusal = page.get_by_role("alert").filter(has_text="Schedule not changed")
    expect(refusal).to_contain_text("schedule was not changed")
    expect(refusal).to_contain_text("cluster connection is unavailable")
    current = page.request.get(path)
    assert current.status == 200
    assert current.headers["etag"] == original.headers["etag"]
    assert SavedSchedule.model_validate(current.json()) == saved
    expect(page.get_by_role("button", name="Save", exact=True)).to_be_disabled()

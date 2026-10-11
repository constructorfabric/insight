"""M365 connector test config builder."""

from __future__ import annotations

import json
from urllib.parse import urlencode

from connector_tests import ConfigBuilder, HttpRequest, HttpResponse

GRAPH_URL = "https://graph.microsoft.com/v1.0"
TOKEN_URL = "https://login.microsoftonline.com/test-azure-tenant/oauth2/v2.0/token"

# With this clock the calendar window is the 27 finished UTC days before today.
FROZEN_NOW = "2026-07-01T12:00:00Z"
WINDOW_START = "2026-06-04T00:00:00Z"
WINDOW_END = "2026-07-01T00:00:00Z"

CALENDAR_SELECT = "id,iCalUId,type,start,end,isAllDay,isCancelled,isOrganizer,showAs,responseStatus,attendees"


class M365ConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update(
            {
                "azure_tenant_id": "test-azure-tenant",
                "azure_client_id": "test-client",
                "azure_client_secret": "test-secret",
            }
        )

    def with_calendar(self) -> M365ConfigBuilder:
        self._config["m365_calendar"] = "true"
        return self


def mock_token(http_mocker) -> None:
    """The client-credentials exchange every Graph call starts with."""
    body = urlencode(
        {
            "scope": "https://graph.microsoft.com/.default",
            "client_id": "test-client",
            "grant_type": "client_credentials",
            "client_secret": "test-secret",
        }
    )
    http_mocker.post(
        HttpRequest(TOKEN_URL, body=body),
        HttpResponse(
            body=json.dumps({"access_token": "test-bearer", "token_type": "Bearer", "expires_in": 3600}),
            status_code=200,
        ),
    )


def users_request() -> HttpRequest:
    return HttpRequest(
        f"{GRAPH_URL}/users",
        query_params={"$select": "id,userPrincipalName,mail", "$filter": "accountEnabled eq true", "$top": "999"},
    )


def calendar_request(user_id: str) -> HttpRequest:
    return HttpRequest(
        f"{GRAPH_URL}/users/{user_id}/calendarView",
        query_params={"startDateTime": WINDOW_START, "endDateTime": WINDOW_END, "$select": CALENDAR_SELECT},
    )


def page(values: list[dict], next_link: str | None = None) -> HttpResponse:
    body: dict = {"value": values}
    if next_link:
        body["@odata.nextLink"] = next_link
    return HttpResponse(body=json.dumps(body), status_code=200)


def graph_error(status: int, code: str, message: str) -> HttpResponse:
    return HttpResponse(body=json.dumps({"error": {"code": code, "message": message}}), status_code=status)

"""Who asks the instance for a spec's metrics.

The suite mints nothing. A spec's request is sent as a seeded persona, over the session
that persona won by driving the deployed login, through the gateway that fronts
analytics — so a spec proves not only the numbers but that a real caller may read them.

A spec writes the API path the product documents; the client requires the gateway
prefix, and the one place that knows both is here rather than in every module.
"""

from __future__ import annotations

from typing import Any

from insight_stand.api import ApiClient, analytics_path


class StandCaller:
    """Sends a spec's request as one persona and returns the status and payload."""

    def __init__(self, client: ApiClient) -> None:
        self._client = client

    def call_request(self, request: dict[str, Any]) -> tuple[int, Any]:
        method = str(request.get("method", "POST")).upper()
        if method != "POST":
            raise ValueError(f"a spec's request must be a POST; got {method}")
        response = self._client.post(
            analytics_path(str(request["url"])), json_body=request.get("body")
        )
        try:
            return response.status_code, response.json()
        except ValueError:
            return response.status_code, response.text

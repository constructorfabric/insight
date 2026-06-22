"""Copilot seats stream — full-refresh, offset-paginated.

Endpoint: GET /orgs/{org}/copilot/billing/seats?page={p}&per_page=100

API response shape:
    {
      "total_seats": 1234,
      "seats": [
        {
          "created_at": "...",
          "updated_at": "...",
          "pending_cancellation_date": null,
          "last_activity_at": "...",
          "last_activity_editor": "vscode/...",
          "last_authenticated_at": "...",
          "plan_type": "business" | "enterprise" | "unknown",
          "assignee": { "login": "user-x", "email": "user@example.com", ... }
        },
        ...
      ]
    }

Per spec: user_login and user_email come from the nested `assignee` object,
NOT top-level. We extract them explicitly here (and emit `user_login`,
`user_email` as flat top-level fields for Bronze convenience).

Pagination: page + per_page=100. Stop when API returns < 100 seats.
"""

import logging
from typing import Any, Iterable, Mapping, MutableMapping, Optional

import requests

from source_github_copilot.streams.base import CopilotRestStream

logger = logging.getLogger("airbyte")


class CopilotSeatsStream(CopilotRestStream):
    """Full-refresh seat roster."""

    name = "copilot_seats"
    use_cache = False  # Seat roster changes; don't cache across runs

    def path(self, **kwargs) -> str:
        return f"orgs/{self._org}/copilot/billing/seats"

    def request_params(
        self,
        stream_state: Optional[Mapping[str, Any]] = None,
        next_page_token: Optional[Mapping[str, Any]] = None,
        **kwargs,
    ) -> MutableMapping[str, Any]:
        params: MutableMapping[str, Any] = {"per_page": "100"}
        if next_page_token and "page" in next_page_token:
            params["page"] = str(next_page_token["page"])
        else:
            params["page"] = "1"
        return params

    def next_page_token(self, response: requests.Response) -> Optional[Mapping[str, Any]]:
        """Follow GitHub's `Link: rel="next"` header (authoritative); fall back to a
        count heuristic only when the header is absent.

        The previous count-only rule ("stop when < 100") wastes one empty request
        when total is an exact multiple of 100, and would truncate if the API ever
        returned a short non-final page. The Link header is the correct signal.
        """
        if not isinstance(response, requests.Response) or response.status_code != 200:
            return None
        from urllib.parse import parse_qs, urlparse

        next_url = (response.links or {}).get("next", {}).get("url")
        if next_url:
            page = (parse_qs(urlparse(next_url).query).get("page") or [None])[0]
            if page:
                # int for type-parity with the fallback branch below
                return {"page": int(page)}
        # Fallback: no Link header — stop when the page is under-full.
        try:
            seats = (response.json() or {}).get("seats") or []
        except ValueError:
            return None
        if len(seats) < 100:
            return None
        current_page = int((parse_qs(urlparse(response.url).query).get("page") or ["1"])[0])
        return {"page": current_page + 1}

    def parse_response(self, response: requests.Response, **kwargs) -> Iterable[Mapping[str, Any]]:
        if not self._guard_response(response):
            return
        try:
            payload = response.json()
        except ValueError:
            logger.error("Seats response is not valid JSON")
            return

        for seat in payload.get("seats") or []:
            assignee = seat.get("assignee") or {}
            user_login = assignee.get("login") or ""
            user_email = assignee.get("email")  # may be NULL — work email may be absent

            if not user_login:
                logger.warning("Seat without assignee.login — skipping")
                continue

            # Flatten the seat record: top-level identity fields + original seat fields.
            # Bronze keeps both `assignee` (raw) and the flat copies for staging convenience.
            record = {
                "user_login": user_login,
                "user_email": user_email,
                "plan_type": seat.get("plan_type"),
                "pending_cancellation_date": seat.get("pending_cancellation_date"),
                "last_activity_at": seat.get("last_activity_at"),
                "last_activity_editor": seat.get("last_activity_editor"),
                "last_authenticated_at": seat.get("last_authenticated_at"),
                "created_at": seat.get("created_at"),
                "updated_at": seat.get("updated_at"),
                "assignee": assignee,  # passthrough for additionalProperties
            }
            yield self._add_envelope(record, pk_parts=[user_login])

    def get_json_schema(self) -> Mapping[str, Any]:
        """JSON Schema for `bronze_github_copilot.copilot_seats`. additionalProperties=true so
        new fields from the API surface as nullable columns without a schema migration."""
        return {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": True,
            "properties": {
                # framework-injected
                "tenant_id": {"type": "string"},
                "source_id": {"type": "string"},
                "unique_key": {"type": "string"},
                "data_source": {"type": "string"},
                "collected_at": {"type": "string"},
                # identity (flat, extracted from `assignee`)
                "user_login": {"type": "string"},
                "user_email": {"type": ["null", "string"]},
                # seat metadata
                "plan_type": {"type": ["null", "string"]},
                "pending_cancellation_date": {"type": ["null", "string"]},
                "last_activity_at": {"type": ["null", "string"]},
                "last_activity_editor": {"type": ["null", "string"]},
                "last_authenticated_at": {"type": ["null", "string"]},
                "created_at": {"type": ["null", "string"]},
                "updated_at": {"type": ["null", "string"]},
                # raw assignee object (passthrough)
                "assignee": {"type": ["null", "object"]},
            },
        }

"""Reading the data mover's job listing.

One endpoint, four query parameters, one response field. The mover exposes far
more; depending on it would tie the page's history to details that are not
contract-stable across mover upgrades, and the whole point of copying this
account is to keep a durable record of what a changing source said.

Two of those parameters do the work that would otherwise be ours: the listing is
asked for in ascending creation order and filtered from a watermark. Ascending
order is what makes a capped read safe — whatever the cap leaves unread is NEWER
than everything collected, so the next tick resumes at that edge instead of the
watermark jumping over a gap.
"""

from __future__ import annotations

import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request
from typing import Any

_TIMEOUT_SECS = 60

#: One page. Transport shape, not policy — it changes how many round trips cover
#: the same jobs, never which jobs are covered.
PAGE_SIZE = 100

#: A termination backstop, not a coverage bound. Ascending order makes a stop
#: resumable, so this caps one tick and never what the ledger ends up holding.
MAX_PAGES = 50

#: The whole listing read's budget, across every page.
#:
#: INVARIANT: must stay well under the workflow pod's own deadline. `MAX_PAGES`
#: alone bounds nothing in time — fifty pages at the per-request timeout is
#: longer than the tick is allowed to live, and a tick killed part-way never
#: reaches its own summary line, so the run reads as aborted and the next one is
#: skipped. `sweep_run` returning 0 on every path does not protect a tick from a
#: sweep that simply takes too long.
BUDGET_SECS = 300


class MoverError(RuntimeError):
    """Anything that stopped the sweep from reading the mover."""


class Mover:
    def __init__(self, url: str, token: str) -> None:
        self._url = url.rstrip("/")
        self._token = token

    @classmethod
    def from_env(cls, env: dict[str, str] | None = None) -> Mover:
        source = os.environ if env is None else env
        url = source.get("AIRBYTE_URL")
        # SAFETY: the token is minted by the shell and handed over in the
        # environment, never in argv — argv is world-readable inside the pod.
        token = source.get("AIRBYTE_TOKEN")
        if not url:
            raise MoverError("AIRBYTE_URL is not set")
        # SAFETY: urllib honours `file://`, so the scheme is pinned rather than
        # taken on trust from the environment.
        if not url.startswith(("http://", "https://")):
            raise MoverError("AIRBYTE_URL must be an http or https URL")
        if not token:
            raise MoverError("AIRBYTE_TOKEN is not set")
        return cls(url, token)

    def sync_jobs(self, created_at_start: str | None) -> tuple[list[dict[str, Any]], bool]:
        """Every sync job created at or after the watermark, oldest first.

        Returns the entries and whether the read stopped short — of the page cap
        or of the time budget. Either way what was collected is contiguous from
        the watermark, so a short read is a delay rather than a gap.
        """
        deadline = time.monotonic() + BUDGET_SECS
        collected: list[dict[str, Any]] = []
        for page in range(MAX_PAGES):
            # Before the request, not after it: a page started with a second of
            # budget left still takes the full request timeout, so checking
            # afterwards lets the read overshoot by one timeout and delay the
            # seal that dates the page.
            if page > 0 and time.monotonic() >= deadline:
                return collected, True
            entries, served = self._page(page * PAGE_SIZE, created_at_start)
            collected.extend(entries)
            # INVARIANT: measured against what the server SERVED, not what
            # survived filtering. A full page carrying one unusable element
            # would otherwise look like a short page, and a read that stopped
            # half way would report itself complete.
            if served < PAGE_SIZE:
                return collected, False
        return collected, True

    def _page(
        self, offset: int, created_at_start: str | None
    ) -> tuple[list[dict[str, Any]], int]:
        query = {
            "jobType": "sync",
            "orderBy": "createdAt|ASC",
            "limit": str(PAGE_SIZE),
            "offset": str(offset),
        }
        if created_at_start:
            query["createdAtStart"] = created_at_start
        path = "/api/public/v1/jobs?" + urllib.parse.urlencode(query)

        payload = self._get(path)
        entries = payload.get("data")
        if not isinstance(entries, list):
            raise MoverError("job listing carries no data array")
        usable = [entry for entry in entries if isinstance(entry, dict)]
        return usable, len(entries)

    def _get(self, path: str) -> dict[str, Any]:
        request = urllib.request.Request(self._url + path, method="GET")
        request.add_header("Accept", "application/json")
        request.add_header("Authorization", f"Bearer {self._token}")
        try:
            # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
            with urllib.request.urlopen(request, timeout=_TIMEOUT_SECS) as response:
                decoded = json.load(response)
        except urllib.error.HTTPError as error:
            detail = error.read().decode(errors="replace").strip()[:400]
            raise MoverError(f"mover rejected the listing: {error.code} {detail}") from error
        except (OSError, json.JSONDecodeError) as error:
            raise MoverError(f"mover unreadable: {error}") from error
        if not isinstance(decoded, dict):
            raise MoverError("mover returned no object")
        return decoded

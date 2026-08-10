"""What the SPA actually asked for when an evidence dialog opened.

A dialog with rows in it proves the request succeeded, not that it was the right
request: the same table renders whether the selection carried the person whose
cell was clicked, the metric that was chosen, or the period the reader was
looking at. The selection is only visible on the wire, so a journey that cares
which one was sent reads it from the request the browser made.

`/export` is a different path, so it never satisfies this predicate.
"""

from __future__ import annotations

from collections.abc import Iterator
from contextlib import contextmanager
from typing import Any

from playwright.sync_api import Page, Request

DRILLDOWN_PATH = "/api/analytics/v1/metric-drilldown"


def is_drilldown(request: Request) -> bool:
    return request.method == "POST" and request.url.endswith(DRILLDOWN_PATH)


@contextmanager
def evidence_selection(page: Page) -> Iterator[dict[str, Any]]:
    """The drilldown selection sent while the block runs, readable after it."""
    selection: dict[str, Any] = {}
    with page.expect_request(is_drilldown) as request_info:
        yield selection
    body = request_info.value.post_data_json
    assert isinstance(body, dict), f"drilldown request carried {body!r}, not a selection"
    selection.update(body)

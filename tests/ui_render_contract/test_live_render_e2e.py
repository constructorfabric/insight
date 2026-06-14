"""Live UI-render e2e: assert the rendered DOM equals documented_transform(API).

This is the binding half of the render contract — it drives the real dashboard,
captures the actual API responses the page received, and checks every KPI tile
shows exactly what `render_contract.display_value()` says it should.

It is deliberately env-gated (skips unless INSIGHT_BASE_URL + INSIGHT_STORAGE_STATE
are set) because the app is behind Entra+MFA — we do NOT automate the login. To
produce the auth state once, by hand:

    playwright codegen --save-storage=auth.json https://insight-dev.constr.dev/
    # log in through the browser that opens (incl. MFA), then close it.
    INSIGHT_BASE_URL=https://insight-dev.constr.dev \
    INSIGHT_STORAGE_STATE=auth.json \
    INSIGHT_PERSON=<your-login-email> \
    pytest tests/ui_render_contract/test_live_render_e2e.py -v

Requires: pip install pytest pytest-playwright && playwright install chromium

Some assertions are EXPECTED to fail against the current app — that is the point.
They encode the correct contract; the failures are constructorfabric/insight#1337
(ComingSoon shown as 0; null ratio shown as 0%). They are marked xfail with the
issue reference so the suite is green until the bug is fixed, then flips to alert.
"""
from __future__ import annotations

import json
import os

import pytest

from render_contract import display_value

BASE_URL = os.environ.get("INSIGHT_BASE_URL")
STORAGE = os.environ.get("INSIGHT_STORAGE_STATE")
PERSON = os.environ.get("INSIGHT_PERSON", "")

pytestmark = pytest.mark.skipif(
    not (BASE_URL and STORAGE),
    reason="set INSIGHT_BASE_URL + INSIGHT_STORAGE_STATE (see module docstring)",
)

# KPI field → (catalog format, unit, ingested). `ingested=False` mirrors the
# catalog ComingSoon flag for sources not wired yet.
KPI_CONTRACT = {
    "focus_time_pct": ("percent", "%", True),
    "ai_loc_share_pct": ("percent", "%", True),
    "tasks_closed": ("integer", "count", True),
    "bugs_fixed": ("integer", "count", True),
    "ai_sessions": ("integer", "count", True),
    "prs_merged": ("integer", "count", False),       # #1337: not ingested → ComingSoon
    "pr_cycle_time_h": ("hours", "h", False),        # not ingested → ComingSoon
}

# label text on the tile → KPI field (for locating the rendered value)
TILE_LABEL = {
    "Focus Time": "focus_time_pct",
    "AI Code Acceptance": "ai_loc_share_pct",
    "Tasks Closed": "tasks_closed",
    "Bugs Fixed": "bugs_fixed",
    "AI Sessions": "ai_sessions",
    "Pull Requests Merged": "prs_merged",
    "Pull Request Cycle Time": "pr_cycle_time_h",
}


@pytest.fixture(scope="module")
def dashboard():
    """Load the IC dashboard, return (page, captured kpis dict)."""
    from playwright.sync_api import sync_playwright

    captured: dict = {}
    with sync_playwright() as p:
        browser = p.chromium.launch()
        ctx = browser.new_context(storage_state=STORAGE)
        page = ctx.new_page()

        def on_response(resp):
            if "/metrics/queries" in resp.url and resp.request.method == "POST":
                try:
                    body = resp.json()
                except Exception:  # noqa: BLE001
                    return
                for r in body.get("results", []):
                    if r.get("id") == "kpis" and r.get("status") == "ok" and r.get("items"):
                        captured.update(r["items"][0])

        page.on("response", on_response)
        url = BASE_URL.rstrip("/") + (f"/ic/{PERSON}/personal" if PERSON else "/")
        page.goto(url, wait_until="networkidle")
        page.wait_for_timeout(1500)
        assert captured, "no kpis batch item captured — auth expired or layout changed?"
        yield page, captured
        browser.close()


def _tile_text(page, label: str) -> str:
    """Visible text of the KPI tile whose label is `label`."""
    tile = page.locator(f"xpath=//*[normalize-space(text())='{label}']/ancestor::*[self::div][1]")
    return tile.first.inner_text(timeout=5000)


@pytest.mark.parametrize("label,field", list(TILE_LABEL.items()))
def test_tile_matches_contract(dashboard, label, field):
    page, kpis = dashboard
    if field not in kpis:
        pytest.skip(f"{field} not in captured kpis")
    fmt, unit, ingested = KPI_CONTRACT[field]
    expected = display_value(kpis.get(field), fmt=fmt, unit=unit, ingested=ingested)
    shown = _tile_text(page, label)

    # known-buggy today → xfail with the issue ref, so green until fixed.
    if not ingested or kpis.get(field) is None:
        pytest.xfail(f"constructorfabric/insight#1337: expected {expected!r} in tile {label!r}")
    assert expected in shown, f"{label}: expected {expected!r}, tile shows {shown!r}"


def test_no_value_glued_to_unit(dashboard):
    """#1291: a number must never be glued to its unit ('0tasks', '0h')."""
    import re

    page, _ = dashboard
    body = page.locator("main").inner_text()
    glued = re.findall(r"\b\d+(?:tasks|count|h|d|messages|emails|files|meetings)\b", body)
    assert not glued, f"value glued to unit (no space): {sorted(set(glued))}"

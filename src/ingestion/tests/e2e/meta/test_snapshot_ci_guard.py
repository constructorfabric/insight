"""CI=true guard for --update-snapshots.

The guard lives in `fixtures/test_fixtures.py`; the framework MUST refuse to
overwrite expected/response.csv when running under CI, so a CI run cannot
silently rubber-stamp regressions.

This test inspects the runner source for the guard rather than spawning a
nested pytest — the latter would force a full session boot (compose, CH,
analytics-api binary) for what's a one-line check.
"""

from __future__ import annotations

from pathlib import Path

import pytest


pytestmark = pytest.mark.smoke


_RUNNER_PATH = Path(__file__).resolve().parents[1] / "fixtures" / "test_fixtures.py"


def test_runner_source_contains_ci_guard() -> None:
    """The runner refuses --update-snapshots when CI=true (per feature-snapshot-update DoD)."""
    source = _RUNNER_PATH.read_text(encoding="utf-8")
    assert 'os.environ.get("CI")' in source, (
        f"{_RUNNER_PATH} must check CI env var before running update_snapshot"
    )
    assert "update_snapshots" in source
    assert "pytest.fail" in source or "raise" in source, (
        "the CI guard must hard-fail, not silently fall through"
    )

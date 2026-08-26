"""Pytest fixtures for the Keycloak Helm contract tests.

The render helpers live in `theme_harness` — see the note there on why they
are not in this file.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
from theme_harness import UMBRELLA


@pytest.fixture(scope="session")
def umbrella_deps() -> Path:
    """Vendor the subcharts once per session.

    Session-scoped on purpose: every module that renders the umbrella needs
    this, and each run rewrites `charts/insight/charts/` — so this suite and
    the other helm suites must not share a working tree concurrently.
    """
    proc = subprocess.run(
        ["helm", "dependency", "update", str(UMBRELLA)],
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    return UMBRELLA

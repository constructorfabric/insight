from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
from theme_harness import UMBRELLA


@pytest.fixture(scope="session")
def umbrella_deps() -> Path:
    proc = subprocess.run(
        ["helm", "dependency", "update", str(UMBRELLA)],
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    return UMBRELLA

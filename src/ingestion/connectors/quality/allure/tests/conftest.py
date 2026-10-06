import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

sys.path.insert(0, str(Path(__file__).parent))

from airbyte_cdk.sources.streams.http import rate_limiting  # noqa: E402
from connector_tests.plugin import *  # noqa: E402,F401,F403


@pytest.fixture
def slept(monkeypatch: pytest.MonkeyPatch) -> list[float]:
    waits: list[float] = []
    monkeypatch.setattr(rate_limiting, "time", SimpleNamespace(sleep=waits.append))

    return waits

from __future__ import annotations

import sys
from pathlib import Path
from unittest.mock import MagicMock

import pymysql
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))


@pytest.fixture
def database(monkeypatch: pytest.MonkeyPatch) -> MagicMock:
    connection = MagicMock()
    connection.__enter__.return_value = connection
    cursor = connection.cursor.return_value.__enter__.return_value
    cursor.__iter__.side_effect = lambda: iter([("example", "main", 7, '["0 3 * * *"]')])
    monkeypatch.setattr(pymysql, "connect", MagicMock(return_value=connection))
    return cursor

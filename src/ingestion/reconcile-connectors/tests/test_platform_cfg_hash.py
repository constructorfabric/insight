"""The cfg-hash of a source with injected platform fields moves with those fields.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "python" / "platform_cfg_hash.py"

SECRET_HASH = "a" * 64
PROXY = {"git_proxy_url": "https://proxy.example.com", "git_proxy_token": "token-1"}


def digest(secret_hash: str, injected: dict | str) -> subprocess.CompletedProcess[str]:
    injected_json = injected if isinstance(injected, str) else json.dumps(injected)
    return subprocess.run(
        [sys.executable, str(SCRIPT), secret_hash, injected_json], capture_output=True, text=True, check=False
    )


def test_the_same_inputs_give_the_same_digest() -> None:
    first = digest(SECRET_HASH, PROXY)
    second = digest(SECRET_HASH, dict(reversed(list(PROXY.items()))))

    assert first.returncode == 0, first.stderr
    assert first.stdout == second.stdout, "key order must not change the digest"
    assert len(first.stdout) == 64 and first.stdout != SECRET_HASH, f"not a fresh digest: {first.stdout!r}"


@pytest.mark.parametrize("injected", ["", "null", "{}"])
def test_no_injected_fields_is_an_empty_object(injected: str) -> None:
    result = digest(SECRET_HASH, injected)

    assert result.returncode == 0, result.stderr
    assert result.stdout == digest(SECRET_HASH, {}).stdout, f"should hash as {{}}: {injected!r}"


@pytest.mark.parametrize(
    ("secret_hash", "injected"),
    [
        pytest.param("b" * 64, PROXY, id="rotated-secret"),
        pytest.param(SECRET_HASH, {**PROXY, "git_proxy_token": "token-2"}, id="rotated-proxy-token"),
        pytest.param(SECRET_HASH, {**PROXY, "git_proxy_url": "https://other.example.com"}, id="moved-proxy"),
    ],
)
def test_a_rotated_secret_or_platform_field_moves_the_digest(secret_hash: str, injected: dict) -> None:
    baseline = digest(SECRET_HASH, PROXY).stdout

    moved = digest(secret_hash, injected)

    assert moved.returncode == 0, moved.stderr
    assert moved.stdout != baseline, f"should drift: {injected!r} / {secret_hash[:4]}"


@pytest.mark.parametrize(
    ("injected", "reason"),
    [
        pytest.param("{not json", "bad JSON", id="malformed"),
        pytest.param("[1, 2]", "must be a JSON object", id="not-an-object"),
    ],
)
def test_injected_fields_that_are_not_an_object_are_refused(injected: str, reason: str) -> None:
    result = digest(SECRET_HASH, injected)

    assert result.returncode == 1, f"should refuse: {injected!r}"
    assert reason in result.stderr, result.stderr

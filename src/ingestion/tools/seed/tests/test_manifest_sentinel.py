"""Tests for getting the manifest out of a pod through its log.

The document is printed after every database write, in a Job with
`backoffLimit: 0` — so refusing an oversized one would report a correctly
seeded stand as a failure with nothing left to retry. It switches transport
instead, and both forms have to survive a real log.

Run against the installed package (see the README's develop section):

    uv run --extra dev pytest tests
"""

from __future__ import annotations

import contextlib
import io
import json
import shutil
import subprocess
from pathlib import Path

import pytest

from insight_seed import config, manifest
from insight_seed import manifest_sentinel as sentinel

_EMAIL = "dev@company.nonpresent"
_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"

_SEED_DIR = Path(__file__).resolve().parent.parent
_REASSEMBLER = _SEED_DIR / "manifest-from-log.sh"

_needs_reassembler = pytest.mark.skipif(
    not (_REASSEMBLER.is_file() and shutil.which("bash")),
    reason="manifest-from-log.sh needs bash",
)

_GROWN = 250
_ENV = config.ORG_HEADCOUNT_ENV


def _env(headcount: str | None = None) -> dict[str, str]:
    env = {
        "DEV_USER_EMAIL": _EMAIL,
        "TENANT_DEFAULT_ID": _TENANT,
        "SEED_ANCHOR_DATE": "2026-06-30",
        "SEED_DAYS": "60",
        config.CROSS_TENANT_FIXTURE_ENV: "0",
    }
    if headcount is not None:
        env[_ENV] = headcount
    return env


def _emit(doc: manifest.Manifest) -> list[str]:
    buffer = io.StringIO()
    with contextlib.redirect_stdout(buffer):
        sentinel.emit_manifest_sentinel(doc)
    return buffer.getvalue().splitlines()


@pytest.fixture(scope="module")
def default_manifest() -> manifest.Manifest:
    return manifest.build_manifest(_env())


@pytest.fixture(scope="module")
def grown_manifest() -> manifest.Manifest:
    return manifest.build_manifest(_env(str(_GROWN)))


def test_the_default_roster_still_emits_one_plain_line(
    default_manifest: manifest.Manifest,
) -> None:
    """The manifest is printed after every write, in a Job with backoffLimit 0.

    Refusing an oversized document there would report a correctly seeded stand
    as a failure, with nothing left to retry.
    """
    lines = _emit(default_manifest)

    assert len(lines) == 1
    assert lines[0].startswith(sentinel.SENTINEL_PREFIX)
    assert len(lines[0].encode("utf-8")) <= sentinel._SENTINEL_MAX_BYTES
    assert json.loads(lines[0][len(sentinel.SENTINEL_PREFIX) :]) == default_manifest
    assert sentinel.decode_manifest_sentinel(lines) == default_manifest


def test_a_grown_roster_round_trips_through_the_chunked_form(
    grown_manifest: manifest.Manifest,
) -> None:
    assert len(grown_manifest["personas"]) == _GROWN

    lines = _emit(grown_manifest)

    assert lines
    for line in lines:
        assert line.startswith(sentinel.GZ_SENTINEL_PREFIX), line[:40]
        assert len(line.encode("utf-8")) <= sentinel._SENTINEL_MAX_BYTES
    assert sentinel.decode_manifest_sentinel(lines) == grown_manifest


def test_chunks_survive_reordering_and_surrounding_log_noise(
    grown_manifest: manifest.Manifest,
) -> None:
    lines = [
        "2026-06-30 INFO seed.silver generating rows",
        *reversed(_emit(grown_manifest)),
        "done",
    ]

    assert sentinel.decode_manifest_sentinel(lines) == grown_manifest


def test_a_missing_chunk_is_an_error_not_a_partial_document() -> None:
    doc = manifest.build_manifest(_env(str(config.MAX_ORG_HEADCOUNT)))
    lines = _emit(doc)
    assert len(lines) > 1, "the largest roster should need several chunks"

    with pytest.raises(ValueError):
        sentinel.decode_manifest_sentinel(lines[:-1])


def test_input_without_a_sentinel_is_an_error() -> None:
    with pytest.raises(ValueError):
        sentinel.decode_manifest_sentinel(["nothing to see here"])


@pytest.mark.parametrize("payload", ["null", "[]", "42", '"a string"'])
def test_a_sentinel_carrying_a_non_object_is_refused(payload: str) -> None:
    """`json.loads` returns whatever the text was; only an object is a manifest."""
    with pytest.raises(ValueError, match="not a JSON object"):
        sentinel.decode_manifest_sentinel([sentinel.SENTINEL_PREFIX + payload])


def test_an_identical_line_read_twice_is_tolerated(grown_manifest: manifest.Manifest) -> None:
    """A re-read log repeats lines; the same bytes are the same chunk."""
    lines = _emit(grown_manifest)

    assert sentinel.decode_manifest_sentinel(lines + lines) == grown_manifest


def test_conflicting_totals_are_an_error_not_a_splice(grown_manifest: manifest.Manifest) -> None:
    """Two emissions in one log must fail, not decode whichever came last."""
    lines = _emit(grown_manifest)
    foreign = f"{sentinel.GZ_SENTINEL_PREFIX}1/{len(lines) + 1} AAAA"

    with pytest.raises(ValueError):
        sentinel.decode_manifest_sentinel([foreign, *lines])


def test_a_differing_duplicate_chunk_is_an_error(grown_manifest: manifest.Manifest) -> None:
    lines = _emit(grown_manifest)
    forged = f"{sentinel.GZ_SENTINEL_PREFIX}1/{len(lines)} AAAA"

    with pytest.raises(ValueError):
        sentinel.decode_manifest_sentinel([*lines, forged])


@_needs_reassembler
@pytest.mark.parametrize("manifest_fixture", ["default_manifest", "grown_manifest"])
def test_the_shell_reassembler_agrees_with_the_python_one(
    manifest_fixture: str, request: pytest.FixtureRequest
) -> None:
    doc = request.getfixturevalue(manifest_fixture)
    log = "\n".join(["starting", *_emit(doc), "done"]) + "\n"

    result = subprocess.run(
        ["bash", str(_REASSEMBLER)],
        input=log,
        capture_output=True,
        text=True,
        check=True,
    )

    out = result.stdout.strip()
    assert out.startswith(sentinel.SENTINEL_PREFIX), out[:60]
    assert json.loads(out[len(sentinel.SENTINEL_PREFIX) :]) == doc


@_needs_reassembler
def test_the_shell_reassembler_refuses_a_corrupt_payload() -> None:
    """A failed decode must not print a bare sentinel — it matches the grep.

    Consumers select the manifest with `grep -m1 '^SEED_MANIFEST_JSON: '`, so
    an empty line under that prefix reads as a valid, empty manifest.
    """
    result = subprocess.run(
        ["bash", str(_REASSEMBLER)],
        input=f"{sentinel.GZ_SENTINEL_PREFIX}1/1 !!!!notbase64!!!!\n",
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode != 0
    assert sentinel.SENTINEL_PREFIX not in result.stdout

"""The shared Python logger emits the agreed line shape at the install's level.

The shape is the gears services' JSON envelope (insight#2488 AC-1): exactly
{timestamp, level, fields, target}, message nested under fields, level spelled
the Rust way. The level comes from INSIGHT_LOG_LEVEL and nowhere else (AC-2).

Run: pytest src/ingestion/scripts/tests
"""

from __future__ import annotations

import json
import logging
import sys
from datetime import UTC, datetime
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import insight_logging


def emitted_lines(capsys) -> list[dict]:
    err = capsys.readouterr().err
    return [json.loads(line) for line in err.splitlines() if line]


def one_line(capsys) -> dict:
    lines = emitted_lines(capsys)
    assert len(lines) == 1, f"expected one line, got {lines!r}"
    return lines[0]


def test_a_line_carries_exactly_the_agreed_field_set(capsys) -> None:
    log = insight_logging.configure("job-under-test")

    log.info("hello")

    line = one_line(capsys)
    assert set(line) == {"timestamp", "level", "fields", "target"}
    assert line["level"] == "INFO"
    assert line["fields"]["message"] == "hello"
    assert line["target"] == "job-under-test"


def test_the_timestamp_is_rfc3339_utc(capsys) -> None:
    insight_logging.configure("job").info("now")

    stamp = one_line(capsys)["timestamp"]
    assert stamp.endswith("Z")
    parsed = datetime.strptime(stamp, "%Y-%m-%dT%H:%M:%S.%fZ").replace(tzinfo=UTC)
    assert abs((datetime.now(UTC) - parsed).total_seconds()) < 60


def test_extra_values_land_inside_fields(capsys) -> None:
    log = insight_logging.configure("job")

    log.info("sync finished", extra={"event": "sync.completed", "duration_ms": 42})

    fields = one_line(capsys)["fields"]
    assert fields == {"message": "sync finished", "event": "sync.completed", "duration_ms": 42}


def test_an_unserializable_extra_never_kills_the_job(capsys) -> None:
    insight_logging.configure("job").info("odd", extra={"path": object()})

    assert isinstance(one_line(capsys)["fields"]["path"], str)


def test_an_exception_is_carried_not_dumped_as_text(capsys) -> None:
    log = insight_logging.configure("job")
    try:
        raise ValueError("boom")
    except ValueError:
        log.exception("it broke")

    line = one_line(capsys)
    assert line["level"] == "ERROR"
    assert "ValueError: boom" in line["fields"]["exception"]


@pytest.mark.parametrize(
    ("python_level", "expected"),
    [(logging.WARNING, "WARN"), (logging.ERROR, "ERROR"), (logging.CRITICAL, "ERROR"), (logging.DEBUG, "DEBUG")],
)
def test_levels_are_spelled_the_rust_way(capsys, monkeypatch, python_level, expected) -> None:
    monkeypatch.setenv("INSIGHT_LOG_LEVEL", "debug")
    insight_logging.configure("job").log(python_level, "line")

    assert one_line(capsys)["level"] == expected, f"should spell: {python_level!r}"


@pytest.mark.parametrize(
    ("knob", "info_expected", "debug_expected", "warn_expected"),
    [
        ("debug", True, True, True),
        ("info", True, False, True),
        ("warn", False, False, True),
        ("error", False, False, False),
    ],
)
def test_the_install_level_decides_what_is_emitted(
    capsys, monkeypatch, knob, info_expected, debug_expected, warn_expected
) -> None:
    monkeypatch.setenv("INSIGHT_LOG_LEVEL", knob)
    log = insight_logging.configure("job")

    log.debug("at debug")
    log.info("at info")
    log.warning("at warn")

    messages = [line["fields"]["message"] for line in emitted_lines(capsys)]
    assert ("at info" in messages) == info_expected, f"should honour: {knob!r}"
    assert ("at debug" in messages) == debug_expected, f"should honour: {knob!r}"
    assert ("at warn" in messages) == warn_expected, f"should honour: {knob!r}"


@pytest.mark.parametrize("knob", [None, "", "verbose", "TRACE"])
def test_an_absent_or_unknown_knob_means_info(capsys, monkeypatch, knob) -> None:
    if knob is None:
        monkeypatch.delenv("INSIGHT_LOG_LEVEL", raising=False)
    else:
        monkeypatch.setenv("INSIGHT_LOG_LEVEL", knob)
    log = insight_logging.configure("job")

    log.debug("at debug")
    log.info("at info")

    messages = [line["fields"]["message"] for line in emitted_lines(capsys)]
    assert messages == ["at info"], f"should default to info: {knob!r}"


def test_bound_fields_ride_every_line_and_yield_to_extras(capsys) -> None:
    log = insight_logging.configure("job", run_id="tick-1")

    log.info("first")
    log.info("second", extra={"run_id": "override"})

    lines = emitted_lines(capsys)
    assert lines[0]["fields"]["run_id"] == "tick-1"
    assert lines[1]["fields"]["run_id"] == "override"


def test_reconfiguring_never_doubles_a_line(capsys) -> None:
    insight_logging.configure("first")
    log = insight_logging.configure("second")

    log.info("once")

    assert len(emitted_lines(capsys)) == 1

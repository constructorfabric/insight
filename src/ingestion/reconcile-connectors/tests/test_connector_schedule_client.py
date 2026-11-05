from __future__ import annotations

import json
import sys
from pathlib import Path
from unittest.mock import MagicMock

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

import pymysql
import read_connector_schedules as client

GOOD = {
    "status": "ok",
    "version": 1,
    "schedules": [{"connector": "example", "source_id": "main", "revision": 7, "crons": ["0 3 * * *", "0 15 * * *"]}],
}
ENV = {
    "CONNECTOR_SCHEDULES_DB_HOST": "example-db",
    "CONNECTOR_SCHEDULES_DB_NAME": "analytics",
    "CONNECTOR_SCHEDULES_DB_USER": "example_reader",
    "CONNECTOR_SCHEDULES_DB_PASSWORD": "synthetic-secret",
}


def test_reads_the_complete_snapshot_with_parameterized_bounded_sql(database: MagicMock) -> None:
    database.__iter__.side_effect = lambda: iter(
        [("example", "main", 7, '["0 3 * * *"]'), ("example", "second", 8, "[]")]
    )
    result = client.read_schedules(ENV)
    assert result == {
        "status": "ok",
        "schedules": [
            {"connector": "example", "source_id": "main", "revision": 7, "crons": ("0 3 * * *",)},
            {"connector": "example", "source_id": "second", "revision": 8, "crons": ()},
        ],
    }
    sql, parameters = database.execute.call_args.args
    assert "FROM connector_schedule_requests_v1" in sql
    assert sql.count("%s") == 2 and parameters == (4096, 10001)
    options = pymysql.connect.call_args.kwargs
    assert options["connect_timeout"] == 5 and options["read_timeout"] == options["write_timeout"] == 15
    assert options["init_command"] == "SET SESSION max_statement_time=10"


@pytest.mark.parametrize(
    "bad",
    [
        ("example", "second", 0, "[]"),
        ("example", "second", -1, "[]"),
        ("example", "second", 8, "null"),
        ("example", "second", 8, '[""]'),
        ("example", "second", 8, json.dumps(["* * * * *"] * 11)),
        ("example", "second", 8, json.dumps(["x" * 257])),
        ("example", "bad id", 8, "[]"),
        ("x" * 65, "second", 8, "[]"),
        ("example", "second", 8, "invalid"),
        ("example", "main", 7, "[]"),
    ],
)
def test_one_invalid_row_refuses_the_entire_snapshot(database: MagicMock, bad: tuple) -> None:
    database.__iter__.side_effect = lambda: iter([("example", "main", 7, '["0 3 * * *"]'), bad])
    assert client.read_schedules(ENV) == {"status": "unavailable", "reason": "schedule database read failed"}


@pytest.mark.parametrize("failure", [pymysql.OperationalError, pymysql.InterfaceError])
def test_connection_and_auth_failures_never_disclose_credentials(database: MagicMock, failure: type[Exception]) -> None:
    pymysql.connect.side_effect = failure("synthetic-secret example_reader example-db")
    assert client.read_schedules(ENV) == {"status": "unavailable", "reason": "schedule database read failed"}


def test_credentials_can_come_from_files(database: MagicMock, tmp_path: Path) -> None:
    password = tmp_path / "password"
    password.write_text("synthetic-file-secret\n")
    assert client.read_schedules({**ENV, "CONNECTOR_SCHEDULES_DB_PASSWORD_FILE": str(password)})["status"] == "ok"
    assert pymysql.connect.call_args.kwargs["password"] == "synthetic-file-secret"


def test_oversized_snapshot_is_unavailable(database: MagicMock, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(client, "MAX_ROWS", 0)
    assert client.read_schedules(ENV)["status"] == "unavailable"


GOOD_ENTRY = GOOD["schedules"][0]


@pytest.mark.parametrize("entries", [[], [GOOD_ENTRY], [GOOD_ENTRY, {**GOOD_ENTRY, "source_id": "second"}]])
def test_the_whole_valid_response_is_accepted(entries: list[dict[str, object]]) -> None:
    result = client.validate_response({"version": 1, "schedules": entries})
    assert result == tuple(
        client.Schedule(e["connector"], e["source_id"], tuple(e["crons"]), e["revision"]) for e in entries
    )


BAD_RESPONSES = [None, [], {}, {"version": 1}, {"version": True, "schedules": []}, {"version": 1.0, "schedules": []}]
BAD_RESPONSES += [{"version": v, "schedules": []} for v in [0, 2, "1", None]]
BAD_RESPONSES += [{"version": 1, "schedules": value} for value in [None, {}, "schedules", 1]]
BAD_RESPONSES += [{"version": 1, "schedules": [GOOD_ENTRY, value]} for value in [None, [], "entry", 1, {}]]
for field in ["connector", "source_id"]:
    for value in [None, 1, "", "two words", "a\tb", "a\nb", "a\rb", "a\x00b", "a\x7fb", "a\u0085b", "a\u200bb"]:
        BAD_RESPONSES.append({"version": 1, "schedules": [GOOD_ENTRY, {**GOOD_ENTRY, field: value}]})
for value in [
    None,
    "0 3 * * *",
    ["0 3 * * *"] * 11,
    [1],
    [None],
    [""],
    ["   "],
    ["a\nb"],
    ["a\rb"],
    ["a\tb"],
    ["a\x00b"],
    ["a\u2028b"],
]:
    BAD_RESPONSES.append(
        {"version": 1, "schedules": [GOOD_ENTRY, {**GOOD_ENTRY, "source_id": "second", "crons": value}]}
    )
for value in [None, True, 1.5, "3"]:
    BAD_RESPONSES.append(
        {"version": 1, "schedules": [GOOD_ENTRY, {**GOOD_ENTRY, "source_id": "second", "revision": value}]}
    )
for field in GOOD_ENTRY:
    BAD_RESPONSES.append(
        {"version": 1, "schedules": [{key: value for key, value in GOOD_ENTRY.items() if key != field}]}
    )
BAD_RESPONSES.append({"version": 1, "schedules": [GOOD_ENTRY, GOOD_ENTRY]})


@pytest.mark.parametrize("body", BAD_RESPONSES)
def test_any_bad_entry_refuses_the_whole_response(body: object) -> None:
    with pytest.raises(client.ScheduleReadError):
        client.validate_response(body)


@pytest.mark.parametrize(
    "name",
    [
        "CONNECTOR_SCHEDULES_DB_HOST",
        "CONNECTOR_SCHEDULES_DB_NAME",
        "CONNECTOR_SCHEDULES_DB_USER",
        "CONNECTOR_SCHEDULES_DB_PASSWORD",
    ],
)
@pytest.mark.parametrize("value", [None, ""])
def test_reads_require_every_environment_value(database: MagicMock, name: str, value: str | None) -> None:
    env = dict(ENV)
    if value is None:
        del env[name]
    else:
        env[name] = value
    assert client.read_schedules(env) == {"status": "unavailable", "reason": "schedule database read failed"}


def test_clear_tombstones_are_valid_requests() -> None:
    result = client.validate_response({"version": 1, "schedules": [{**GOOD_ENTRY, "crons": []}]})
    assert result == (client.Schedule("example", "main", (), 7),)

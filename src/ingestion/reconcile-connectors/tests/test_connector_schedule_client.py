from __future__ import annotations

import base64
import errno
import json
import os
import subprocess
import sys
import threading
import time
from collections.abc import Iterator
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from urllib.parse import parse_qs

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from read_connector_schedules import (
    REQUIRED_ENV,
    Schedule,
    ScheduleReadError,
    read_schedules,
    sign_assertion,
    validate_response,
)

GOOD_ENTRY = {"connector": "example", "source_id": "main", "crons": ["0 3 * * *", "0 15 * * *"], "revision": 3}
GOOD = {"version": 1, "schedules": [GOOD_ENTRY]}


@pytest.fixture(params=["pkcs8", "sec1"])
def signing_env(tmp_path: Path, request: pytest.FixtureRequest) -> dict[str, str]:
    key = tmp_path / "key.pem"
    public = tmp_path / "public.pem"
    subprocess.run(
        [
            "node",
            "-e",
            """
            const {generateKeyPairSync} = require('node:crypto');
            const {writeFileSync} = require('node:fs');
            const {privateKey, publicKey} = generateKeyPairSync('ec', {namedCurve: 'prime256v1'});
            writeFileSync(process.argv[1], privateKey.export({format: 'pem', type: process.argv[3]}));
            writeFileSync(process.argv[2], publicKey.export({format: 'pem', type: 'spki'}));
            """,
            str(key),
            str(public),
            request.param,
        ],
        check=True,
        capture_output=True,
    )
    return {
        **os.environ,
        "CONNECTOR_SCHEDULES_ENABLED": "true",
        "CONNECTOR_SCHEDULES_URL": "http://127.0.0.1:1/schedules",
        "SERVICE_TOKEN_URL": "http://127.0.0.1:1/token",
        "SERVICE_TOKEN_NAME": "example-service",
        "SERVICE_TOKEN_KEY_FILE": str(key),
        "SERVICE_TOKEN_TENANT_ID": "00000000-0000-4000-8000-000000000001",
    }


def test_assertions_prove_the_service_identity_with_a_short_unique_lifetime(signing_env: dict[str, str]) -> None:
    before = int(time.time())
    assertion = sign_assertion(signing_env)
    second = sign_assertion(signing_env)
    header, payload, _ = assertion.split(".")
    claims = json.loads(base64.urlsafe_b64decode(payload + "=="))
    assert json.loads(base64.urlsafe_b64decode(header + "==")) == {"alg": "ES256", "typ": "JWT"}
    assert claims["iss"] == claims["sub"] == signing_env["SERVICE_TOKEN_NAME"]
    assert claims["aud"] == signing_env["SERVICE_TOKEN_URL"]
    assert before <= claims["iat"] <= int(time.time())
    assert 0 < claims["exp"] - claims["iat"] <= 60
    second_claims = json.loads(base64.urlsafe_b64decode(second.split(".")[1] + "=="))
    assert claims["jti"] and claims["jti"] != second_claims["jti"]
    verified = subprocess.run(
        [
            "node",
            "-e",
            """
            const {verify} = require('node:crypto');
            const {readFileSync} = require('node:fs');
            const [header, payload, signature] = readFileSync(0, 'utf8').split('.');
            const valid = verify('sha256', Buffer.from(`${header}.${payload}`),
              {key: readFileSync(process.argv[1]), dsaEncoding: 'ieee-p1363'}, Buffer.from(signature, 'base64url'));
            process.exit(valid ? 0 : 1);
            """,
            str(Path(signing_env["SERVICE_TOKEN_KEY_FILE"]).with_name("public.pem")),
        ],
        input=assertion,
        capture_output=True,
        text=True,
        check=False,
    )
    assert verified.returncode == 0, verified.stderr


@pytest.mark.parametrize("entries", [[], [GOOD_ENTRY], [GOOD_ENTRY, {**GOOD_ENTRY, "source_id": "second"}]])
def test_the_whole_valid_response_is_accepted(entries: list[dict[str, object]]) -> None:
    result = validate_response({"version": 1, "schedules": entries})
    assert result == tuple(Schedule(e["connector"], e["source_id"], tuple(e["crons"]), e["revision"]) for e in entries)


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
    [],
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
    with pytest.raises(ScheduleReadError):
        validate_response(body)


@pytest.mark.parametrize("name", REQUIRED_ENV)
@pytest.mark.parametrize("value", [None, ""])
def test_enabled_reads_require_every_environment_value(name: str, value: str | None) -> None:
    env = {key: "example" for key in REQUIRED_ENV}
    env["CONNECTOR_SCHEDULES_ENABLED"] = "true"
    if value is None:
        del env[name]
    else:
        env[name] = value
    assert read_schedules(env) == {"status": "unavailable", "reason": f"missing {name}"}


@pytest.mark.parametrize("enabled", [None, "", "false", "TRUE", "1"])
def test_only_literal_true_enables_the_tier(enabled: str | None) -> None:
    env = {} if enabled is None else {"CONNECTOR_SCHEDULES_ENABLED": enabled}
    assert read_schedules(env) == {"status": "off"}


@pytest.fixture
def stub_server() -> Iterator[tuple[HTTPServer, list[tuple[str, str, str]]]]:
    calls = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            body = self.rfile.read(int(self.headers["Content-Length"])).decode()
            calls.append(("POST", self.path, body))
            status = 401 if self.path == "/refused" else 200
            self.send_response(status)
            self.end_headers()
            self.wfile.write(json.dumps({"access_token": "example-token"}).encode())

        def do_GET(self) -> None:
            calls.append(("GET", self.path, self.headers["Authorization"]))
            self.send_response(503 if self.path == "/unavailable" else 200)
            self.end_headers()
            self.wfile.write(b"malformed" if self.path == "/malformed" else json.dumps(GOOD).encode())

        def log_message(self, format: str, *args: object) -> None:
            pass

    try:
        server = HTTPServer(("127.0.0.1", 0), Handler)
    except OSError as exc:
        if exc.errno in {errno.EPERM, errno.EACCES, errno.EAFNOSUPPORT, errno.EADDRNOTAVAIL}:
            pytest.skip(f"local sockets unavailable: {exc.strerror}")
        raise
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server, calls
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


@pytest.mark.parametrize(
    ("token_path", "schedules_path", "status", "reason"),
    [
        ("/token", "/schedules", "ok", None),
        ("/refused", "/schedules", "unavailable", "token HTTP 401"),
        ("/token", "/unavailable", "unavailable", "schedules HTTP 503"),
        ("/token", "/malformed", "unavailable", "schedules returned invalid JSON"),
    ],
)
def test_http_exchange_is_all_or_nothing(
    signing_env: dict[str, str],
    stub_server: tuple[HTTPServer, list[tuple[str, str, str]]],
    token_path: str,
    schedules_path: str,
    status: str,
    reason: str | None,
) -> None:
    server, calls = stub_server
    base = f"http://127.0.0.1:{server.server_port}"
    signing_env.update(SERVICE_TOKEN_URL=base + token_path, CONNECTOR_SCHEDULES_URL=base + schedules_path)
    result = read_schedules(signing_env)
    assert result["status"] == status
    form = parse_qs(calls[0][2])
    assert calls[0][:2] == ("POST", token_path)
    assert form["grant_type"] == ["client_credentials"]
    assert form["client_assertion_type"] == ["urn:ietf:params:oauth:client-assertion-type:jwt-bearer"]
    assert form["tenant_id"] == [signing_env["SERVICE_TOKEN_TENANT_ID"]]
    assert len(form["client_assertion"][0].split(".")) == 3
    assert len(calls) == (1 if token_path == "/refused" else 2)
    if len(calls) == 2:
        assert calls[1] == ("GET", schedules_path, "Bearer example-token")
    if reason:
        assert result == {"status": status, "reason": reason}
    else:
        assert json.loads(json.dumps(result["schedules"])) == GOOD["schedules"]


@pytest.mark.parametrize(
    ("token_reply", "schedule_reply", "expected"),
    [
        ('{"access_token":"example-token"}\n200', json.dumps(GOOD) + "\n200", "ok"),
        ("refused\n401", "", "token HTTP 401"),
        ('{"access_token":"example-token"}\n200', "refused\n503", "schedules HTTP 503"),
        ('{"access_token":"example-token"}\n200', "malformed\n200", "schedules returned invalid JSON"),
        ("malformed\n200", "", "token returned invalid JSON"),
        ('{"access_token":""}\n200', "", "token response has no valid access_token"),
        ('{"access_token":"a\\nb"}\n200', "", "token response has no valid access_token"),
        (
            '{"access_token":"example-token"}\n200',
            json.dumps({"version": 2, "schedules": []}) + "\n200",
            "invalid schedules response version",
        ),
    ],
)
def test_transport_uses_bounded_calls_and_never_exposes_credentials(
    signing_env: dict[str, str], monkeypatch: pytest.MonkeyPatch, token_reply: str, schedule_reply: str, expected: str
) -> None:
    import read_connector_schedules as client

    real_run = subprocess.run
    calls = []
    replies = iter([token_reply, schedule_reply])

    def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        if command[0] == "node":
            return real_run(command, **kwargs)
        calls.append((command, kwargs))
        return subprocess.CompletedProcess(command, 0, stdout=next(replies), stderr="example sensitive failure detail")

    monkeypatch.setattr(client.subprocess, "run", run)
    result = client.read_schedules(signing_env)
    if expected == "ok":
        assert result["status"] == "ok"
        assert json.loads(json.dumps(result["schedules"])) == GOOD["schedules"]
    else:
        assert result == {"status": "unavailable", "reason": expected}
    assert len(calls) == (2 if token_reply == '{"access_token":"example-token"}\n200' else 1)
    for command, kwargs in calls:
        assert command[command.index("--connect-timeout") + 1] == "5"
        assert command[command.index("--max-time") + 1] == "15"
        assert kwargs["timeout"] == 17
        assert "example-token" not in " ".join(command)
        assert "client_assertion=" not in " ".join(command)
        assert "--location" not in command
    form = parse_qs(calls[0][1]["input"])
    assert form["tenant_id"] == [signing_env["SERVICE_TOKEN_TENANT_ID"]]
    assert form["grant_type"] == ["client_credentials"]
    assert form["client_assertion_type"] == ["urn:ietf:params:oauth:client-assertion-type:jwt-bearer"]
    assert form["client_assertion"][0].count(".") == 2
    if len(calls) == 2:
        assert calls[1][1]["input"] == "Authorization: Bearer example-token\n"


@pytest.mark.parametrize("failure", [OSError("example"), subprocess.TimeoutExpired("curl", 17)])
def test_transport_failures_are_unavailable_without_sensitive_details(
    monkeypatch: pytest.MonkeyPatch, failure: Exception
) -> None:
    import read_connector_schedules as client

    def run(*args: object, **kwargs: object) -> subprocess.CompletedProcess[str]:
        raise failure

    monkeypatch.setattr(client.subprocess, "run", run)
    with pytest.raises(ScheduleReadError, match="^schedules request failed$"):
        client.request_json("http://example.invalid/schedules", "schedules", "", post=False)


def test_a_signing_failure_is_unavailable_without_key_details(signing_env: dict[str, str]) -> None:
    Path(signing_env["SERVICE_TOKEN_KEY_FILE"]).write_text("example invalid key content")
    assert read_schedules(signing_env) == {"status": "unavailable", "reason": "service assertion signing failed"}


@pytest.mark.parametrize("name", ["version", "missing-field", "wrong-type", "duplicate"])
def test_shared_negative_fixtures_refuse_the_whole_response(name: str) -> None:
    fixture = Path(__file__).with_name("fixtures") / f"connector-schedules-invalid-{name}.json"
    with pytest.raises(ScheduleReadError):
        validate_response(json.loads(fixture.read_bytes()))


def test_analytics_fixture_preserves_each_instance_and_ordered_crons() -> None:
    fixture = Path(__file__).with_name("fixtures") / "connector-schedules.json"
    assert validate_response(json.loads(fixture.read_bytes())) == (
        Schedule("example-connector", "main-east", ("0 3 * * *", "0 15 * * *"), 3),
        Schedule("example-connector", "second-west", ("30 6 * * 1-5",), 7),
        Schedule("example-other", "main-east", ("0 9 * * *",), 2),
    )

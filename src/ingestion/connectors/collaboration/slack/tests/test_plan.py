"""The plan and the token's scopes decide which streams a check and a sync read.

`requests_mock` raises NoMockAddress on any unregistered URL, so a test that
registers no day file proves the stream was not read.
"""

from __future__ import annotations

import logging
from typing import Any

import pytest
import requests
from airbyte_cdk.models import AirbyteErrorTraceMessage, FailureType
from airbyte_cdk.test.entrypoint_wrapper import EntrypointOutput
from airbyte_cdk.utils.traced_exception import AirbyteTracedException
from conftest import CONFIG, day_url, member_file, refusal, run, slack
from source_slack import plan as plan_module
from source_slack.client import BILLING_URL
from source_slack.plan import StreamRule
from source_slack.source import SourceSlack

_LOG = logging.getLogger("test")


def _errors(output: EntrypointOutput) -> list[AirbyteErrorTraceMessage]:
    return [e.trace.error for e in output.errors]


@pytest.mark.parametrize("plan", ["plus", "enterprise", "compliance"])
def test_plans_with_member_analytics_read_activity(plan: str) -> None:
    with slack(plan) as http:
        for day in ("2026-10-01", "2026-10-02"):
            http.get(day_url(day), **member_file(day))

        output = run()

    assert output.records, f"should read users_details on {plan}"
    assert http.request_history[0].url == BILLING_URL


@pytest.mark.parametrize("plan", ["free", "std"])
def test_plans_without_member_analytics_sync_without_reading_activity(plan: str) -> None:
    with slack(plan, scopes=("team.billing:read",)) as http:
        output = run()

    assert not output.errors, f"should succeed on {plan}"
    assert not output.records
    assert [r.url for r in http.request_history] == [BILLING_URL]


@pytest.mark.parametrize(
    ("plan", "report"),
    [
        ("plus", "Plan Business+ (plus). Streams to read: users_details. Missing scopes: none."),
        ("compliance", "Plan Enterprise (compliance). Streams to read: users_details. Missing scopes: none."),
        ("free", "Plan Free (free). Streams to read: none. Missing scopes: none."),
        ("std", "Plan Pro (std). Streams to read: none. Missing scopes: none."),
    ],
)
def test_check_reports_the_plan_and_the_streams_to_read(plan: str, report: str) -> None:
    with slack(plan) as http:
        http.get(day_url("2026-10-02"), **member_file("2026-10-02"))

        status = SourceSlack().check(_LOG, CONFIG)

    assert (status.status.value, status.message) == ("SUCCEEDED", report)


@pytest.mark.parametrize(
    ("code", "hint"),
    [
        ("missing_scope", "team.billing:read"),
        ("enterprise_is_restricted", "enterprise_is_restricted"),
        ("invalid_auth", "invalid_auth"),
        ("not_allowed_token_type", "not_allowed_token_type"),
    ],
)
def test_an_unreadable_plan_fails_the_sync_as_a_config_error_naming_the_code(code: str, hint: str) -> None:
    with slack() as http:
        http.post(BILLING_URL, json={"ok": False, "error": code})

        output = run(expecting_exception=True)
        ok, message = SourceSlack().check_connection(_LOG, CONFIG)

    assert not output.records
    errors = _errors(output)
    assert errors and errors[0].failure_type == FailureType.config_error, f"should be config_error: {code}"
    assert code in errors[0].message and hint in errors[0].message
    assert not ok and code in message and hint in message


@pytest.mark.parametrize("code", ["invalid_arguments", "team_added_to_org", "ratelimited"])
def test_a_plan_refusal_that_is_not_a_token_scope_or_plan_code_fails_as_a_system_error(code: str) -> None:
    with slack() as http:
        http.post(BILLING_URL, json={"ok": False, "error": code})

        output = run(expecting_exception=True)

    errors = _errors(output)
    assert errors[0].failure_type == FailureType.system_error, f"should be system_error: {code}"
    assert code in errors[0].message


def test_an_unknown_plan_fails_the_sync_as_a_config_error() -> None:
    with slack("platinum"):
        output = run(expecting_exception=True)

    errors = _errors(output)
    assert errors[0].failure_type == FailureType.config_error
    assert "platinum" in errors[0].message


def test_a_main_stream_without_its_scope_fails_the_sync_naming_the_scope() -> None:
    with slack("plus", scopes=("team.billing:read", "users:read")):
        output = run(expecting_exception=True)
        ok, message = SourceSlack().check_connection(_LOG, CONFIG)

    errors = _errors(output)
    assert errors[0].failure_type == FailureType.config_error
    assert "admin.analytics:read" in errors[0].message
    assert not ok and "admin.analytics:read" in message


def test_an_optional_stream_without_its_scope_is_skipped_with_a_warning(monkeypatch: pytest.MonkeyPatch) -> None:
    optional = StreamRule("users", frozenset({"plus"}), frozenset({"users:read", "users:read.email"}))
    monkeypatch.setattr(plan_module, "STREAM_RULES", (*plan_module.STREAM_RULES, optional))

    with slack("plus", scopes=("team.billing:read", "admin.analytics:read", "users:read")) as http:
        for day in ("2026-10-01", "2026-10-02"):
            http.get(day_url(day), **member_file(day))

        output = run()
        status = SourceSlack().check(_LOG, CONFIG)

    assert not output.errors
    assert output.records
    warnings = [log.log.message for log in output.logs if log.log.level.value == "WARN"]
    assert any("users" in w and "users:read.email" in w for w in warnings)
    assert status.message == (
        "Plan Business+ (plus). Streams to read: users_details. Missing scopes: users:read.email."
    )


def test_a_temporary_billing_refusal_is_retried(sleeps: list[float]) -> None:
    with slack() as http:
        http.post(
            BILLING_URL,
            [
                {"json": {"ok": False, "error": "ratelimited"}, "headers": {"Retry-After": "7"}},
                {"json": {"ok": True, "plan": "plus"}, "headers": {"x-oauth-scopes": "admin.analytics:read"}},
            ],
        )
        for day in ("2026-10-01", "2026-10-02"):
            http.get(day_url(day), **member_file(day))

        output = run()

    assert output.records
    assert sleeps == [7.0]


@pytest.mark.parametrize(
    ("answer", "kind"),
    [
        ({"json": {"ok": True, "plan": "plus"}}, FailureType.system_error),
        ({"text": "<html>"}, FailureType.system_error),
        ({"status_code": 403}, FailureType.system_error),
        ({"exc": requests.ConnectionError("no route")}, FailureType.transient_error),
    ],
)
def test_a_broken_billing_answer_fails_the_sync(answer: dict[str, Any], kind: FailureType) -> None:
    with slack() as http:
        http.post(BILLING_URL, **answer)

        output = run(expecting_exception=True)

    assert not output.records
    assert _errors(output)[0].failure_type == kind, f"should fail as {kind}: {answer}"


@pytest.mark.parametrize(
    "answer",
    [
        {"json": {"ok": False, "error": "ratelimited"}},
        {"status_code": 503},
        {"exc": requests.ConnectionError("no route")},
    ],
)
def test_check_raises_when_slack_fails_rather_than_the_config(answer: dict[str, Any]) -> None:
    with slack() as http:
        http.post(BILLING_URL, **answer)

        with pytest.raises(AirbyteTracedException) as raised:
            SourceSlack().check_connection(_LOG, CONFIG)

    assert raised.value.failure_type != FailureType.config_error, f"should not be config_error: {answer}"


def test_check_names_a_refused_day_on_a_readable_plan() -> None:
    with slack() as http:
        http.get(day_url("2026-10-02"), **refusal("member_analytics_disabled"))

        ok, message = SourceSlack().check_connection(_LOG, CONFIG)

    assert not ok and "member_analytics_disabled" in message

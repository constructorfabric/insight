from __future__ import annotations

import io
import json
import sys
from collections.abc import Callable
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import airbyte_sync_poll as poll

PollRunner = Callable[[list[object]], tuple[int, int]]


@pytest.fixture
def run_poll(monkeypatch: pytest.MonkeyPatch) -> PollRunner:
    monkeypatch.setenv("JOB_ID", "123")
    monkeypatch.setenv("AIRBYTE_URL", "https://airbyte.example.com")
    monkeypatch.setenv("POLL_INTERVAL_SECONDS", "1801")
    monkeypatch.setenv("IDLE_THRESHOLD_SECONDS", "3600")
    monkeypatch.setenv("STATUS_UNREADABLE_THRESHOLD_SECONDS", "3600")
    monkeypatch.setattr(poll, "oauth_token", lambda: "test-token")

    def run(responses: list[object]) -> tuple[int, int]:
        elapsed = 0
        remaining = iter(responses)

        def sleep(seconds: int) -> None:
            nonlocal elapsed
            elapsed += seconds
            assert elapsed < 300000, "polling must terminate on the supplied terminal status"

        def urlopen(*args: object, **kwargs: object) -> io.StringIO:
            response = next(remaining)
            if isinstance(response, Exception):
                raise response
            return io.StringIO(json.dumps(response))

        monkeypatch.setattr(poll.time, "time", lambda: elapsed)
        monkeypatch.setattr(poll.time, "monotonic", lambda: elapsed)
        monkeypatch.setattr(poll.time, "sleep", sleep)
        monkeypatch.setattr(poll.urllib.request, "urlopen", urlopen)
        return poll.main(), elapsed

    return run


def job(status: str, records: int = 0) -> dict[str, object]:
    return {
        "job": {"status": status},
        "attempts": [{"attempt": {"status": "running", "totalStats": {"recordsEmitted": records}}}],
    }


@pytest.mark.parametrize("status", ["pending", "running", "incomplete"])
def test_readable_nonterminal_jobs_can_wait_beyond_idle_threshold(run_poll: PollRunner, status: str) -> None:
    result, elapsed = run_poll([job(status)] * 4 + [job("succeeded")])
    assert result == 0, f"should keep waiting: {status}"
    assert elapsed > 3600


def test_rate_limit_wait_can_resume_and_complete(run_poll: PollRunner) -> None:
    result, _ = run_poll([job("running", 10)] * 4 + [job("running", 20), job("succeeded", 20)])
    assert result == 0


def test_readable_status_resets_outage_timer_without_record_progress(run_poll: PollRunner) -> None:
    result, _ = run_poll([job("running"), OSError("offline"), job("running"), OSError("offline"), job("succeeded")])
    assert result == 0


@pytest.mark.parametrize("unreadable", [OSError("offline"), {}, {"job": {"status": "unexpected"}}])
def test_continuous_unreadability_stops_monitoring(
    run_poll: PollRunner, unreadable: object, capsys: pytest.CaptureFixture[str]
) -> None:
    result, elapsed = run_poll([unreadable] * 4)
    assert result == 2
    assert 3600 <= elapsed <= 5403
    output = capsys.readouterr().err
    assert '"event": "sync.poll_failed"' in output
    assert '"event": "sync.completed"' not in output


@pytest.mark.parametrize(("status", "expected"), [("succeeded", 0), ("failed", 1), ("cancelled", 1)])
def test_terminal_status_wins_over_stale_counters(run_poll: PollRunner, status: str, expected: int) -> None:
    result, _ = run_poll([job("running"), job("running"), job(status)])
    assert result == expected, f"should honor terminal status: {status}"


def test_terminal_status_does_not_require_attempt_statistics(run_poll: PollRunner) -> None:
    result, _ = run_poll(
        [{"job": {"status": "succeeded"}, "attempts": [{"attempt": {"totalStats": {"recordsEmitted": "bad"}}}]}]
    )
    assert result == 0


def test_readable_job_can_complete_after_48_hours(run_poll: PollRunner) -> None:
    result, elapsed = run_poll([job("running")] * 100 + [job("succeeded")])
    assert result == 0
    assert elapsed > 172800


def test_retry_with_unchanged_totals_can_complete(run_poll: PollRunner) -> None:
    retry = job("running")
    retry["attempts"] = [{"attempt": {"id": 1, "status": "running"}}]
    result, _ = run_poll([job("running"), job("incomplete"), retry, retry, job("succeeded")])
    assert result == 0


def test_permanent_auth_failure_does_not_claim_sync_completion(
    run_poll: PollRunner, capsys: pytest.CaptureFixture[str]
) -> None:
    result, _ = run_poll([poll.FatalError("credentials unavailable")])
    assert result == 1
    output = capsys.readouterr().err
    assert '"event": "sync.poll_failed"' in output
    assert '"event": "sync.completed"' not in output

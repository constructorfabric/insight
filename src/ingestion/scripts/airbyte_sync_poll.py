"""Mirror an Airbyte job's status until completion or sustained unreadability."""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.request
from enum import StrEnum
from typing import NamedTuple

import insight_logging
from airbyte_auth import FatalError, airbyte_api_url, oauth_token

Progress = tuple[str | None, int, int, int]

FAILURE_MESSAGE_MAX_CHARS = 500


class JobStatus(StrEnum):
    PENDING = "pending"
    QUEUED = "queued"
    RUNNING = "running"
    INCOMPLETE = "incomplete"
    SUCCEEDED = "succeeded"
    FAILED = "failed"
    CANCELLED = "cancelled"


class SyncFailure(NamedTuple):
    failure_type: str
    origin: str
    external_message: str
    internal_message: str


def attempt_failures(resp: object) -> list[SyncFailure]:
    if not isinstance(resp, dict):
        return []
    attempts = resp.get("attempts")
    if not isinstance(attempts, list) or not attempts:
        return []

    attempt = attempts[-1].get("attempt") if isinstance(attempts[-1], dict) else None
    summary = attempt.get("failureSummary") if isinstance(attempt, dict) else None
    entries = summary.get("failures") if isinstance(summary, dict) else None
    if not isinstance(entries, list):
        return []

    return [_read_failure(entry) for entry in entries if isinstance(entry, dict)]


def _read_failure(entry: dict) -> SyncFailure:
    return SyncFailure(
        failure_type=_read_text(entry.get("failureType")) or "unknown",
        origin=_read_text(entry.get("failureOrigin")) or "unknown",
        external_message=_read_text(entry.get("externalMessage")),
        internal_message=_read_text(entry.get("internalMessage")),
    )


def _read_text(value: object) -> str:
    if not isinstance(value, str):
        return ""
    return value[:FAILURE_MESSAGE_MAX_CHARS]


def attempt_progress(resp: dict) -> Progress | None:
    attempts = resp.get("attempts") or []
    if not attempts:
        return None

    attempt = attempts[-1].get("attempt", {})
    stats = attempt.get("totalStats") or {}
    return (
        attempt.get("status"),
        int(stats.get("bytesEmitted") or attempt.get("bytesSynced") or 0),
        int(stats.get("recordsEmitted") or attempt.get("recordsSynced") or 0),
        int(stats.get("stateMessagesEmitted") or 0),
    )


def job_status(resp: object) -> JobStatus | None:
    if not isinstance(resp, dict):
        return None
    job = resp.get("job")
    if not isinstance(job, dict):
        return None
    status = job.get("status")
    if not isinstance(status, str):
        return None
    try:
        return JobStatus(status)
    except ValueError:
        return None


def main() -> int:
    log = insight_logging.configure("airbyte-sync")

    job_id = int(os.environ["JOB_ID"])
    try:
        airbyte_url = airbyte_api_url()
    except FatalError as error:
        log.error(f"FAIL: {error}")
        return 1
    poll_interval = int(os.environ["POLL_INTERVAL_SECONDS"])
    unreadable_threshold = int(os.environ["STATUS_UNREADABLE_THRESHOLD_SECONDS"])

    t0 = time.monotonic()
    last_progress: Progress | None = None
    last_readable_at = t0

    def emit_completed(status: str, exit_code: int) -> None:
        progress = last_progress or (None, 0, 0, 0)
        (log.info if exit_code == 0 else log.error)(
            "sync finished",
            extra={
                "event": "sync.completed",
                "job_id": job_id,
                "status": status,
                "duration_ms": int((time.monotonic() - t0) * 1000),
                "bytes": progress[1],
                "records": progress[2],
            },
        )

    def retry_or_give_up(reason: str) -> bool:
        unreadable_for = time.monotonic() - last_readable_at
        if unreadable_for >= unreadable_threshold:
            log.error(
                "Cannot read Airbyte job status; sync outcome is unknown",
                extra={"event": "sync.poll_failed", "job_id": job_id, "unreadable_for_s": int(unreadable_for)},
            )
            return False
        log.warning(f"poll: {reason}; retrying in {poll_interval}s")
        time.sleep(poll_interval)
        return True

    while True:
        try:
            token = oauth_token()
            headers = {"Content-Type": "application/json", "Authorization": f"Bearer {token}"}
            request = urllib.request.Request(
                f"{airbyte_url}/api/v1/jobs/get", data=json.dumps({"id": job_id}).encode(), headers=headers
            )
            # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
            resp = json.load(urllib.request.urlopen(request, timeout=30))
        except FatalError as error:
            log.error(f"FAIL: {error}", extra={"event": "sync.poll_failed", "job_id": job_id})
            return 1
        except Exception as error:  # noqa: BLE001
            if not retry_or_give_up(f"transient error fetching job status: {error!r}"):
                return 2
            continue

        status = job_status(resp)
        if status is None:
            if not retry_or_give_up("unusable job status"):
                return 2
            continue
        last_readable_at = time.monotonic()
        try:
            progress = attempt_progress(resp)
        except (AttributeError, IndexError, KeyError, TypeError, ValueError, OverflowError):
            progress = None
            log.warning("Unreadable progress counters", extra={"job_id": job_id})

        if progress is not None and progress != last_progress:
            last_progress = progress
            log.info(
                "sync progress",
                extra={
                    "job_id": job_id,
                    "status": status,
                    "bytes": progress[1],
                    "records": progress[2],
                    "states": progress[3],
                },
            )
        else:
            log.info("waiting for sync completion", extra={"job_id": job_id, "status": status})

        if status == JobStatus.SUCCEEDED:
            log.info("Sync complete")
            emit_completed("succeeded", 0)
            return 0
        if status in (JobStatus.FAILED, JobStatus.CANCELLED):
            log.error(f"Sync failed: {status}")
            for failure in attempt_failures(resp):
                log.error(
                    "sync failure reason",
                    extra={
                        "event": "sync.failure_reason",
                        "job_id": job_id,
                        "failure_type": failure.failure_type,
                        "failure_origin": failure.origin,
                        "external_message": failure.external_message,
                        "internal_message": failure.internal_message,
                    },
                )
            emit_completed(status, 1)
            return 1

        time.sleep(poll_interval)


if __name__ == "__main__":
    sys.exit(main())

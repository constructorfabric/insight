"""Watch one Airbyte sync job until it finishes, stalls, or fails.

Runs as the airbyte-sync workflow's poll step, on the toolbox image. The
watchdog is progress-based rather than wall-clock: a legitimate long sync may
run as long as it keeps emitting records/bytes/state, and no change for
IDLE_THRESHOLD_SECONDS means stuck — exit 2, distinct from the SIGTERM that
the workflow's activeDeadlineSeconds backstop produces.
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.request

import insight_logging
from airbyte_auth import FatalError, airbyte_api_url, oauth_token

Progress = tuple[str | None, int, int, int]


def attempt_progress(resp: dict) -> Progress | None:
    # Latest attempt's totalStats — emitted-by-source counters tick on every
    # record/state message Airbyte sees, so they are the cheapest reliable
    # progress signal (cheaper than streamStats, which is per-stream and grows).
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


def job_status(resp: object) -> str | None:
    """The job's status from a jobs/get payload; None when the shape is unusable."""
    if not isinstance(resp, dict):
        return None
    job = resp.get("job")
    if not isinstance(job, dict):
        return None
    status = job.get("status")
    return status if isinstance(status, str) else None


def main() -> int:
    log = insight_logging.configure("airbyte-sync")

    job_id = int(os.environ["JOB_ID"])
    try:
        airbyte_url = airbyte_api_url()
    except FatalError as error:
        log.error(f"FAIL: {error}")
        return 1
    poll_interval = int(os.environ["POLL_INTERVAL_SECONDS"])
    idle_threshold = int(os.environ["IDLE_THRESHOLD_SECONDS"])

    t0 = time.time()
    last_progress: Progress | None = None
    last_progress_at = time.time()

    def emit_completed(status: str, exit_code: int) -> None:
        # One structured terminal event per sync. duration_ms counts from poll
        # start — the trigger step runs seconds earlier, so this tracks the
        # sync wall-clock closely.
        progress = last_progress or (None, 0, 0, 0)
        (log.info if exit_code == 0 else log.error)(
            "sync finished",
            extra={
                "event": "sync.completed",
                "job_id": job_id,
                "status": status,
                "duration_ms": int((time.time() - t0) * 1000),
                "bytes": progress[1],
                "records": progress[2],
            },
        )

    def retry_or_give_up(reason: str) -> bool:
        # The idle threshold bounds unreadability the same way it bounds a
        # stalled sync: a poll that cannot learn anything for that long must
        # still emit its terminal event before activeDeadlineSeconds SIGTERMs
        # the pod without one. Returns False when the loop should give up.
        idle_for = int(time.time() - last_progress_at)
        if idle_for > idle_threshold:
            log.error(f"FAIL: no readable job status for {idle_for}s (threshold {idle_threshold}s): {reason}")
            emit_completed("unreachable", 2)
            return False
        log.warning(f"poll: {reason}; retrying in {poll_interval}s")
        time.sleep(poll_interval)
        return True

    while True:
        # Token TTL is short (~minutes); re-mint per iteration. A transient
        # blip (token endpoint hiccup, timeout, 5xx) must retry the loop, not
        # crash the watchdog — an unhandled exception here would kill the
        # idle-stuck-job detector below and leave a hung job "running" forever.
        try:
            token = oauth_token()
            headers = {"Content-Type": "application/json", "Authorization": f"Bearer {token}"}
            request = urllib.request.Request(
                f"{airbyte_url}/api/v1/jobs/get", data=json.dumps({"id": job_id}).encode(), headers=headers
            )
            # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
            resp = json.load(urllib.request.urlopen(request, timeout=30))
        except FatalError as error:
            log.error(f"FAIL: {error}")
            emit_completed("error", 1)
            return 1
        except Exception as error:  # noqa: BLE001
            if not retry_or_give_up(f"transient error fetching job status: {error!r}"):
                return 2
            continue

        status = job_status(resp)
        if status is None:
            if not retry_or_give_up(f"unusable job payload: {str(resp)[:200]!r}"):
                return 2
            continue
        progress = attempt_progress(resp)
        now = time.time()
        if progress is not None and progress != last_progress:
            last_progress = progress
            last_progress_at = now
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
            idle_for = int(now - last_progress_at)
            log.info(
                "no sync progress",
                extra={"job_id": job_id, "status": status, "idle_for_s": idle_for, "threshold_s": idle_threshold},
            )
            if idle_for > idle_threshold:
                log.error(
                    f"FAIL: Airbyte job has not emitted any record/byte/state for "
                    f"{idle_for}s (threshold {idle_threshold}s). Treating as stuck."
                )
                emit_completed("stuck", 2)
                return 2

        if status == "succeeded":
            log.info("Sync complete")
            emit_completed("succeeded", 0)
            return 0
        if status in ("failed", "cancelled"):
            log.error(f"Sync failed: {status}")
            emit_completed(status, 1)
            return 1

        time.sleep(poll_interval)


if __name__ == "__main__":
    sys.exit(main())

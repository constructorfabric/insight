"""Trigger one Airbyte connection sync and hand the job id to the poll step.

Runs as the airbyte-sync workflow's trigger step, on the toolbox image; every
workflow-time parameter arrives via the environment (see the SECURITY note in
charts/insight/templates/ingestion/airbyte-sync.yaml).
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.request

import insight_logging
from airbyte_auth import FatalError, airbyte_api_url, oauth_token


def triggered_job_id(resp: object) -> int | str | None:
    """The job id from a connections/sync payload; None when the shape is unusable."""
    if not isinstance(resp, dict):
        return None
    job = resp.get("job")
    if not isinstance(job, dict):
        return None
    job_id = job.get("id")
    return job_id if isinstance(job_id, int | str) else None


def main() -> int:
    log = insight_logging.configure("airbyte-sync")

    try:
        airbyte_url = airbyte_api_url()
    except FatalError as error:
        log.error(f"cannot trigger the sync: {error}")
        return 1

    # retryPolicy OnError only re-runs pod-level errors, not script exits, so
    # transient token-mint blips (K8s apiserver or Airbyte hiccup) are retried
    # here; FatalError aborts immediately.
    token = ""
    for attempt in range(3):
        try:
            token = oauth_token()
            break
        except FatalError as error:
            log.error(f"cannot mint an Airbyte token: {error}")
            return 1
        except Exception as error:
            if attempt == 2:
                log.exception("cannot mint an Airbyte token after retries")
                return 1
            log.warning(f"transient error minting token: {error!r}; retrying in 10s")
            time.sleep(10)

    headers = {"Content-Type": "application/json", "Authorization": f"Bearer {token}"}
    data = json.dumps({"connectionId": os.environ["CONNECTION_ID"]}).encode()
    request = urllib.request.Request(f"{airbyte_url}/api/v1/connections/sync", data=data, headers=headers)
    try:
        # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected
        resp = json.load(urllib.request.urlopen(request, timeout=120))
    except urllib.error.HTTPError as error:
        log.error(f"Airbyte API error {error.code}: {error.read().decode()}")
        return 1
    except Exception:
        # DNS/connection/timeout/bad-JSON failures stay in the log envelope
        # rather than escaping as a bare traceback.
        log.exception("cannot trigger the sync")
        return 1

    job_id = triggered_job_id(resp)
    if job_id is None:
        log.error(f"Airbyte answered the trigger without a job id: {str(resp)[:200]!r}")
        return 1

    # INVARIANT: stdout is this step's Argo outputs.result — the bare job id
    # consumed by poll-job; lifecycle lines go through the shared logger to stderr.
    log.info(
        "sync triggered",
        extra={"event": "sync.triggered", "connection_id": os.environ["CONNECTION_ID"], "job_id": job_id},
    )
    print(job_id)  # noqa: T201 -- stdout IS the step's Argo outputs.result
    return 0


if __name__ == "__main__":
    sys.exit(main())

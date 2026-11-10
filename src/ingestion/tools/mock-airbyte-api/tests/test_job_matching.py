from __future__ import annotations

import json
from concurrent.futures import ThreadPoolExecutor

import pytest
from proxy_client import control, request, rule, running, wait_event
from proxy_rules import request_ids


@pytest.mark.parametrize(
    "path,body",
    [
        ("/api/v1/jobs/get", {"id": 42}),
        ("/api/v1/jobs/get", {"id": "42"}),
        ("/api/v1/jobs/get?synthetic=1", {"id": 42}),
        ("/api/v1/jobs/get", {"jobId": 42}),
        ("/api/v1/jobs/get", {"job_id": 42}),
        ("/api/v1/jobs/get", {"job": {"id": 42}}),
        ("/api/v1/jobs/get", {"id": 42, "jobId": 42}),
        ("/api/public/v1/jobs/42", {}),
    ],
)
def test_job_filters_match_poll_ids_and_documented_keys(path: str, body: object) -> None:
    assert request_ids(path, body)["job_id"] == ["42"], f"path={path}, body={body!r}"


@pytest.mark.parametrize(
    "path,body", [("/api/v1/sources/get", {"id": 42}), ("/api/v1/jobs/get", {"source": {"id": 42}})]
)
def test_generic_ids_are_not_jobs_outside_the_poll_body(path: str, body: object) -> None:
    assert request_ids(path, body)["job_id"] == [], f"path={path}, body={body!r}"


def test_a_single_jobs_second_poll_waits_at_a_barrier_while_other_jobs_pass() -> None:
    with running() as (proxy, upstream), ThreadPoolExecutor() as pool:
        path = "/api/v1/jobs/get"
        poll_rule = rule({"type": "barrier", "barrier": "poll"}, method="POST", path=path, job_id="42", call=2)
        status, configured = control(proxy, "PUT", "/control/scenario", {"rules": [poll_rule]})
        assert status == 200
        for job in [41, 42, 41]:
            assert request(proxy, "POST", path, json.dumps({"id": job}).encode())[0] == 201, job
        pending = pool.submit(request, proxy, "POST", path, b'{"id":42}')
        try:
            wait_event(proxy.state, "barrier_entered")
            assert not pending.done()
            assert request(proxy, "POST", path, b'{"id":43}')[0] == 201
            assert [json.loads(row[-1])["id"] for row in upstream.received] == [41, 42, 41, 43]
        finally:
            status, _ = control(
                proxy, "POST", "/control/release", {"revision": configured["revision"], "barrier": "poll"}
            )
            assert status == 200
        assert pending.result()[0] == 201
        assert [json.loads(row[-1])["id"] for row in upstream.received] == [41, 42, 41, 43, 42]

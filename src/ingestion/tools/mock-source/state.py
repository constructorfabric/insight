from __future__ import annotations

import asyncio
import hashlib
import json
import logging
import math
import time
import uuid
from collections import deque
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta

from configuration import STREAMS, Scenario, timestamp

LOG = logging.getLogger("mock-source")
SESSION_TTL_SECONDS = 300
MAX_SESSIONS = 256
MAX_INSTANCES = 32


class Refusal(Exception):
    def __init__(self, status: int) -> None:
        self.status = status


@dataclass
class Revision:
    number: int
    scenario: Scenario
    failures: dict[str, int] = field(default_factory=dict)
    gates: dict[str, asyncio.Event] = field(default_factory=dict)


@dataclass
class Session:
    instance: str
    stream: str
    revision: Revision
    start: int
    delay: float
    touched: float
    finished: bool = False
    active: int = 0


@dataclass(frozen=True)
class Response:
    status: int
    body: dict[str, object]
    retry_after: int | None = None


def records(
    scenario: Scenario, instance: str, stream: str, start: int, end: int, revision: int
) -> list[dict[str, object]]:
    result = []
    for index in range(start, end):
        digest = hashlib.sha256(f"{scenario.seed}:{instance}:{stream}:{index}".encode()).hexdigest()
        result.append(
            {
                "id": f"{stream}-{index}",
                "value": int(digest[:7], 16),
                "checksum": digest,
                "last_updated_at": (scenario.epoch + timedelta(seconds=index)).strftime("%Y-%m-%dT%H:%M:%SZ"),
                "scenario_revision": revision,
            }
        )
    return result


class State:
    def __init__(self, retention: int) -> None:
        self.generation = uuid.uuid4().hex
        self.instances: dict[str, Revision] = {}
        self.sessions: dict[str, Session] = {}
        self.journal: deque[dict[str, object]] = deque(maxlen=retention)
        self.sequence = 0
        self.revision = 0

    def event(self, **fields: object) -> None:
        self.sequence += 1
        entry = {
            "generation": self.generation,
            "sequence": self.sequence,
            "timestamp": datetime.now(UTC).isoformat(),
            **fields,
        }
        self.journal.append(entry)
        LOG.info(json.dumps(entry, separators=(",", ":")))

    def observations(self, after: int, instance: str | None) -> dict[str, object]:
        entries = [row for row in self.journal if row["sequence"] > after]
        if instance is not None:
            entries = [row for row in entries if row.get("instance") == instance]
        oldest = self.journal[0]["sequence"] if self.journal else self.sequence + 1
        active = [
            {
                "read_id": key,
                "instance": session.instance,
                "stream": session.stream,
                "revision": session.revision.number,
                "requests": session.active,
            }
            for key, session in self.sessions.items()
            if session.active
        ]
        if instance is not None:
            active = [row for row in active if row["instance"] == instance]
        return {
            "generation": self.generation,
            "events": entries,
            "oldest_sequence": oldest,
            "latest_sequence": self.sequence,
            "truncated": after < oldest - 1,
            "active_requests": active,
        }

    def configure(self, instance: str, scenario: Scenario) -> int:
        if instance not in self.instances and len(self.instances) >= MAX_INSTANCES:
            raise Refusal(409)
        self.revision += 1
        self.instances[instance] = Revision(self.revision, scenario)
        self.event(event="configured", instance=instance, revision=self.revision)
        return self.revision

    def prune(self) -> None:
        now = time.monotonic()
        expired = [
            key
            for key, session in self.sessions.items()
            if not session.active and now - session.touched >= SESSION_TTL_SECONDS
        ]
        for key in expired:
            del self.sessions[key]

    def reset(self, instance: str) -> None:
        self.prune()
        owned = [session for session in self.sessions.values() if session.instance == instance]
        if any(session.active or not session.finished for session in owned):
            raise Refusal(409)
        self.sessions = {key: session for key, session in self.sessions.items() if session.instance != instance}
        self.instances.pop(instance, None)
        retained = [row for row in self.journal if row.get("instance") != instance]
        self.journal.clear()
        self.journal.extend(retained)
        self.event(event="reset", instance=instance)

    def release(self, instance: str, revision: int, stream: str) -> None:
        candidates = [session.revision for session in self.sessions.values() if session.instance == instance]
        if instance in self.instances:
            candidates.append(self.instances[instance])
        for candidate in candidates:
            if candidate.number == revision:
                control = candidate.scenario.stream(stream)
                candidate.gates.setdefault(stream, asyncio.Event()).set()
                self.event(
                    event="gate_released", instance=instance, stream=stream, revision=revision, page=control.gate_page
                )
                return
        raise Refusal(404)

    def session(self, instance: str, stream: str, query: dict[str, str]) -> tuple[str, Session, int]:
        self.prune()
        if "page_token" in query:
            key, offset = query["page_token"].split(":")
            session = self.sessions.get(key)
            if session is None:
                raise Refusal(410)
            if (session.instance, session.stream) != (instance, stream):
                raise Refusal(400)
            start = int(offset)
            control = session.revision.scenario.stream(stream)
            if start < session.start or (start - session.start) % control.page_size:
                raise Refusal(400)
            if start >= control.record_count and start != session.start:
                raise Refusal(400)
            return key, session, start
        if len(self.sessions) >= MAX_SESSIONS:
            raise Refusal(503)
        revision = self.instances[instance]
        scenario = revision.scenario
        control = scenario.stream(stream)
        start = 0
        if "cursor" in query and stream == STREAMS[1]:
            cursor = timestamp(query["cursor"])
            start = max(0, min(control.record_count, math.floor((cursor - scenario.epoch).total_seconds()) + 1))
        delay = control.delay(control.record_count - start)
        session = Session(instance, stream, revision, start, delay, time.monotonic())
        key = uuid.uuid4().hex
        self.sessions[key] = session
        return key, session, start

    async def read(self, instance: str, stream: str, query: dict[str, str], authorized: bool) -> Response:
        if instance not in self.instances or stream not in STREAMS:
            raise Refusal(404)
        key, session, start = self.session(instance, stream, query)
        control = session.revision.scenario.stream(stream)
        page = (start - session.start) // control.page_size
        fields = {
            "instance": instance,
            "stream": stream,
            "read_id": key,
            "revision": session.revision.number,
            "page": page,
            "cursor": query.get("cursor"),
            "traffic": "data",
        }
        session.active += 1
        self.event(event="request_started", **fields)
        status = 499
        try:
            response = await self.respond(session, key, start, page, authorized, fields)
            status = response.status
            return response
        finally:
            session.active -= 1
            session.touched = time.monotonic()
            self.event(event="request_finished", status=status, **fields)

    async def respond(
        self, session: Session, key: str, start: int, page: int, authorized: bool, fields: dict[str, object]
    ) -> Response:
        revision = session.revision
        control = revision.scenario.stream(session.stream)
        if not authorized or control.auth_reject:
            session.finished = True
            return Response(401, {"error": "unauthorized"})
        failures = revision.failures.setdefault(session.stream, 0)
        if failures < control.fail_requests:
            revision.failures[session.stream] += 1
            session.finished = True
            return Response(control.failure_status, {"error": "injected"}, control.retry_after_seconds)
        if control.fail_after_pages is not None and page >= control.fail_after_pages:
            session.finished = True
            return Response(control.failure_status, {"error": "injected"}, control.retry_after_seconds)
        if control.gate_page == page:
            gate = revision.gates.setdefault(session.stream, asyncio.Event())
            self.event(event="gate_entered", **fields)
            try:
                await asyncio.wait_for(gate.wait(), timeout=20)
            except TimeoutError:
                return Response(503, {"error": "gate waiting"}, 1)
            self.event(event="gate_passed", **fields)
        await asyncio.sleep(session.delay)
        end = min(start + control.page_size, control.record_count)
        session.finished = end >= control.record_count
        token = None if session.finished else f"{key}:{end}"
        return Response(
            200,
            {
                "records": records(revision.scenario, session.instance, session.stream, start, end, revision.number),
                "next_page_token": token,
                "revision": revision.number,
                "read_id": key,
                "generation": self.generation,
            },
        )

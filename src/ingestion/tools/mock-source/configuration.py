from __future__ import annotations

import math
from dataclasses import dataclass
from datetime import UTC, datetime

STREAMS = ("mock_events", "mock_updates")


def object_map(value: object) -> dict[str, object]:
    if not isinstance(value, dict):
        raise ValueError("expected object")
    return value


def integer(value: object, minimum: int, maximum: int) -> int:
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError("integer outside range")
    return value


def seconds(value: object, maximum: float) -> float:
    if type(value) not in {int, float} or not math.isfinite(value) or not 0 <= value <= maximum:
        raise ValueError("duration outside range")
    return float(value)


def timestamp(value: object) -> datetime:
    if not isinstance(value, str):
        raise ValueError("expected UTC timestamp")
    parsed = datetime.fromisoformat(value)
    if parsed.tzinfo is None or parsed.utcoffset().total_seconds() != 0:
        raise ValueError("expected UTC timestamp")
    return parsed.astimezone(UTC)


@dataclass(frozen=True)
class Stream:
    record_count: int
    page_size: int
    latency_seconds: float
    duration_minutes: float
    fail_after_pages: int | None
    fail_requests: int
    failure_status: int
    retry_after_seconds: int
    auth_reject: bool
    gate_page: int | None

    @classmethod
    def parse(cls, value: object) -> Stream:
        data = object_map(value)
        if set(data) != set(cls.__dataclass_fields__):
            raise ValueError("expected complete stream controls")
        count = integer(data["record_count"], 0, 1_000_000)
        size = integer(data["page_size"], 1, 1000)
        latency = seconds(data["latency_seconds"], 10)
        duration = seconds(data["duration_minutes"], 60)
        pages = max(1, math.ceil(count / size))
        if duration * 60 / pages > 10:
            raise ValueError("duration requires more pages (maximum ten seconds per page)")
        fail_after = data["fail_after_pages"]
        gate = data["gate_page"]
        if fail_after is not None:
            fail_after = integer(fail_after, 0, 1_000_000)
        if gate is not None:
            gate = integer(gate, 0, pages - 1)
        if type(data["auth_reject"]) is not bool:
            raise ValueError("expected boolean")
        return cls(
            count,
            size,
            latency,
            duration,
            fail_after,
            integer(data["fail_requests"], 0, 100_000),
            integer(data["failure_status"], 400, 599),
            integer(data["retry_after_seconds"], 0, 60),
            data["auth_reject"],
            gate,
        )

    def delay(self, remaining_records: int) -> float:
        pages = max(1, math.ceil(remaining_records / self.page_size))
        delay = max(self.latency_seconds, self.duration_minutes * 60 / pages)
        if delay > 10:
            raise ValueError("remaining cursor window needs more pages for duration")
        return delay


@dataclass(frozen=True)
class Scenario:
    seed: int
    epoch: datetime
    events: Stream
    updates: Stream

    @classmethod
    def parse(cls, value: object) -> Scenario:
        data = object_map(value)
        if set(data) != {"seed", "epoch", "streams"}:
            raise ValueError("expected seed, epoch and streams")
        streams = object_map(data["streams"])
        if set(streams) != set(STREAMS):
            raise ValueError("expected both streams")
        epoch = timestamp(data["epoch"])
        if epoch.microsecond or epoch.year > 9998:
            raise ValueError("epoch must have whole seconds and leave room for records")
        return cls(
            integer(data["seed"], 0, 2**32 - 1),
            epoch,
            Stream.parse(streams[STREAMS[0]]),
            Stream.parse(streams[STREAMS[1]]),
        )

    def stream(self, name: str) -> Stream:
        if name == STREAMS[0]:
            return self.events
        if name == STREAMS[1]:
            return self.updates
        raise ValueError("unknown stream")

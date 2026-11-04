from __future__ import annotations

import fnmatch
import math
import re
from dataclasses import dataclass
from typing import ClassVar, Literal
from urllib.parse import urlsplit

ID = re.compile(r"(?:[0-9]{1,20}|[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12})\Z")
NAME = re.compile(r"[a-z][a-z0-9-]{0,62}\Z")
MAX_BODY_BYTES = 1024 * 1024
MAX_REWRITE_BYTES = 4 * 1024 * 1024


def object_map(value: object) -> dict[str, object]:
    if not isinstance(value, dict):
        raise ValueError("expected object")
    return value


def integer(value: object, low: int, high: int) -> int:
    if type(value) is not int or not low <= value <= high:
        raise ValueError("integer outside range")
    return value


def identifier(value: object) -> str:
    if not isinstance(value, (str, int)) or isinstance(value, bool) or not ID.fullmatch(str(value)):
        raise ValueError("invalid id")
    return str(value)


def identities(value: object) -> dict[str, list[str]]:
    found: dict[str, list[str]] = {"connection_id": [], "job_id": []}
    seen: dict[str, set[str]] = {"connection_id": set(), "job_id": set()}
    pending = [value]
    while pending:
        item = pending.pop()
        if isinstance(item, list):
            pending.extend(item)
        if not isinstance(item, dict):
            continue
        for key, entry in item.items():
            kind = {
                "connectionId": "connection_id",
                "connection_id": "connection_id",
                "jobId": "job_id",
                "job_id": "job_id",
            }.get(key)
            if key == "job" and isinstance(entry, dict) and "id" in entry:
                pending.append({"jobId": entry["id"]})
            if kind is not None:
                try:
                    candidate = identifier(entry)
                except ValueError:
                    continue
                if candidate not in seen[kind]:
                    found[kind].append(candidate)
                    seen[kind].add(candidate)
            if isinstance(entry, (dict, list)):
                pending.append(entry)
    return found


def request_ids(path: str, body: object) -> dict[str, list[str]]:
    found = identities(body)
    parts = urlsplit(path).path.split("/")
    for previous, current in zip(parts, parts[1:]):
        if previous not in {"connections", "jobs"} or not ID.fullmatch(current):
            continue
        kind = "connection_id" if previous == "connections" else "job_id"
        if current not in found[kind]:
            found[kind].append(current)
    return found


def encoded_body(value: object) -> bytes:
    if not isinstance(value, str) or len(value.encode()) > MAX_BODY_BYTES:
        raise ValueError("invalid body")
    return value.encode()


def seconds(value: object) -> float:
    if type(value) not in {float, int} or not math.isfinite(value) or not 0 <= value <= 60:
        raise ValueError("invalid delay")
    return float(value)


def field_path(value: object) -> tuple[str, ...]:
    if (
        not isinstance(value, list)
        or not 1 <= len(value) <= 32
        or any(not isinstance(part, str) or not 1 <= len(part) <= 128 for part in value)
    ):
        raise ValueError("invalid field")
    return tuple(value)


class Action:
    @classmethod
    def parse(cls, raw: object) -> Fault:
        data = object_map(raw)
        kind = data["type"]
        fields = {
            "status": ({"type", "status"}, {"body"}),
            "malformed": ({"type", "body"}, set()),
            "truncated": ({"type", "body"}, set()),
            "delay": ({"type", "seconds"}, set()),
            "barrier": ({"type", "barrier"}, set()),
            "drop_response": ({"type"}, set()),
            "delay_response": ({"type", "seconds"}, set()),
            "rewrite": ({"type", "field", "value"}, set()),
        }
        if not isinstance(kind, str) or kind not in fields:
            raise ValueError("unknown action")
        required, optional = fields[kind]
        if not required <= data.keys() or data.keys() - required - optional:
            raise ValueError("invalid action fields")
        if kind == "status":
            return Status(integer(data["status"], 200, 599), encoded_body(data["body"]) if "body" in data else b"")
        if kind == "malformed":
            return Malformed(encoded_body(data["body"]))
        if kind == "truncated":
            return Truncated(encoded_body(data["body"]))
        if kind == "delay":
            return Delay(seconds(data["seconds"]))
        if kind == "barrier":
            barrier = data["barrier"]
            if not isinstance(barrier, str) or not NAME.fullmatch(barrier):
                raise ValueError("invalid barrier")
            return Barrier(barrier)
        if kind == "drop_response":
            return DropResponse()
        if kind == "delay_response":
            return DelayResponse(seconds(data["seconds"]))
        return Rewrite(field_path(data["field"]), data["value"])


@dataclass(frozen=True)
class Status(Action):
    status: int
    body: bytes
    kind: ClassVar[Literal["status"]] = "status"


@dataclass(frozen=True)
class Malformed(Action):
    body: bytes
    kind: ClassVar[Literal["malformed"]] = "malformed"


@dataclass(frozen=True)
class Truncated(Action):
    body: bytes
    kind: ClassVar[Literal["truncated"]] = "truncated"


@dataclass(frozen=True)
class Delay(Action):
    seconds: float
    kind: ClassVar[Literal["delay"]] = "delay"


@dataclass(frozen=True)
class Barrier(Action):
    barrier: str
    kind: ClassVar[Literal["barrier"]] = "barrier"


@dataclass(frozen=True)
class DropResponse(Action):
    kind: ClassVar[Literal["drop_response"]] = "drop_response"


@dataclass(frozen=True)
class DelayResponse(Action):
    seconds: float
    kind: ClassVar[Literal["delay_response"]] = "delay_response"


@dataclass(frozen=True)
class Rewrite(Action):
    field: tuple[str, ...]
    value: object
    kind: ClassVar[Literal["rewrite"]] = "rewrite"


type Fault = Status | Malformed | Truncated | Delay | Barrier | DropResponse | DelayResponse | Rewrite


@dataclass(frozen=True)
class Rule:
    name: str
    method: str
    path: str
    connection_id: str | None
    job_id: str | None
    call: int | None
    action: Fault

    @classmethod
    def parse(cls, raw: object) -> Rule:
        data = object_map(raw)
        if set(data) != {"name", "method", "path", "connection_id", "job_id", "call", "action"}:
            raise ValueError("expected complete rule")
        name, method, path = data["name"], data["method"], data["path"]
        if not isinstance(name, str) or not NAME.fullmatch(name):
            raise ValueError("invalid name")
        if not isinstance(method, str) or not re.fullmatch(r"[A-Z]{1,20}|\*", method):
            raise ValueError("invalid method")
        if not isinstance(path, str) or not path.startswith("/") or len(path) > 512:
            raise ValueError("invalid path")
        return cls(
            name,
            method,
            path,
            identifier(data["connection_id"]) if data["connection_id"] is not None else None,
            identifier(data["job_id"]) if data["job_id"] is not None else None,
            integer(data["call"], 1, 1000000) if data["call"] is not None else None,
            Action.parse(data["action"]),
        )

    def matches(self, method: str, path: str, ids: dict[str, list[str]]) -> bool:
        return (
            self.method in {"*", method}
            and fnmatch.fnmatchcase(urlsplit(path).path, self.path)
            and (self.connection_id is None or self.connection_id in ids["connection_id"])
            and (self.job_id is None or self.job_id in ids["job_id"])
        )


def rewrite(value: object, field: tuple[str, ...], replacement: object) -> object:
    parent = value
    for part in field[:-1]:
        parent = parent[int(part)] if isinstance(parent, list) else object_map(parent)[part]
    if isinstance(parent, list):
        parent[int(field[-1])] = replacement
    else:
        target = object_map(parent)
        if field[-1] not in target:
            raise ValueError("missing field")
        target[field[-1]] = replacement
    return value

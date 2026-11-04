from __future__ import annotations

import json
import logging
import threading
import uuid
from collections import deque
from dataclasses import dataclass, field
from datetime import UTC, datetime
from urllib.parse import urlsplit

from proxy_rules import Barrier, Rule, integer, object_map


class Refusal(Exception):
    def __init__(self, status: int) -> None:
        self.status = status


@dataclass
class Revision:
    number: int
    rules: tuple[Rule, ...]
    calls: dict[str, int] = field(default_factory=dict)
    barriers: dict[str, threading.Event] = field(default_factory=dict)


class State:
    def __init__(self, retention: int) -> None:
        self.lock = threading.RLock()
        self.generation = str(uuid.uuid4())
        self.sequence = 0
        self.revision = 0
        self.current = Revision(0, ())
        self.revisions = {0: self.current}
        self.active: dict[int, dict[str, object]] = {}
        self.journal: deque[dict[str, object]] = deque(maxlen=integer(retention, 1, 10000))

    def event(self, event: str, **fields: object) -> int:
        fields = {
            key: value[:16] if key in {"connection_ids", "job_ids"} and isinstance(value, list) else value
            for key, value in fields.items()
        }
        with self.lock:
            self.sequence += 1
            row = {
                "generation": self.generation,
                "sequence": self.sequence,
                "timestamp": datetime.now(UTC).isoformat(),
                "event": event,
                **fields,
            }
            self.journal.append(row)
            logging.info(json.dumps(row, separators=(",", ":")))
            return self.sequence

    def configure(self, raw: object) -> int:
        data = object_map(raw)
        if set(data) != {"rules"} or not isinstance(data["rules"], list) or len(data["rules"]) > 64:
            raise ValueError("expected rules")
        rules = tuple(Rule.parse(item) for item in data["rules"])
        if len({rule.name for rule in rules}) != len(rules):
            raise ValueError("duplicate rule name")
        with self.lock:
            if len(self.revisions) >= 64:
                raise Refusal(503)
            self.revision += 1
            revision = Revision(self.revision, rules)
            revision.barriers = {
                rule.action.barrier: threading.Event() for rule in rules if isinstance(rule.action, Barrier)
            }
            self.current = revision
            self.revisions[revision.number] = revision
            self.event("configured", revision=revision.number)
            return revision.number

    def begin(self, method: str, path: str, ids: dict[str, list[str]]) -> tuple[int, Revision, Rule | None]:
        with self.lock:
            revision = self.current
            selected = None
            for rule in revision.rules:
                if not rule.matches(method, path, ids):
                    continue
                count = revision.calls.get(rule.name, 0) + 1
                revision.calls[rule.name] = count
                if selected is None and (rule.call is None or rule.call == count):
                    selected = rule
            parts = urlsplit(path).path.split("/")
            endpoint = "/".join(
                part
                if part
                in {
                    "",
                    "api",
                    "public",
                    "v1",
                    "jobs",
                    "connections",
                    "sync",
                    "get",
                    "list",
                    "cancel",
                    "applications",
                    "token",
                    "sources",
                    "destinations",
                    "workspaces",
                    "source_definitions",
                    "create",
                    "update",
                    "delete",
                    "check_connection",
                    "discover_schema",
                    "declarative_source_definitions",
                }
                else "{redacted}"
                for part in parts
            )
            fields = {
                "method": method if method in {"GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"} else "OTHER",
                "endpoint": endpoint[:512],
                "revision": revision.number,
                "rule": selected.name if selected else None,
                "connection_ids": ids["connection_id"][:16],
                "job_ids": ids["job_id"][:16],
            }
            request = self.event("request_started", **fields)
            self.active[request] = fields
            return request, revision, selected

    def release(self, revision: int, barrier: str) -> None:
        with self.lock:
            if revision not in self.revisions or barrier not in self.revisions[revision].barriers:
                raise Refusal(404)
            self.revisions[revision].barriers[barrier].set()
            self.event("barrier_released", revision=revision, barrier=barrier)

    def reset(self) -> None:
        with self.lock:
            if self.active:
                raise Refusal(409)
            self.current = Revision(self.revision, ())
            self.revisions = {self.revision: self.current}
            self.journal.clear()
            self.event("reset", revision=self.revision)

    def observations(self, after: int) -> dict[str, object]:
        with self.lock:
            oldest = self.journal[0]["sequence"] if self.journal else self.sequence + 1
            return {
                "generation": self.generation,
                "events": [row for row in self.journal if row["sequence"] > after],
                "oldest_sequence": oldest,
                "latest_sequence": self.sequence,
                "truncated": after < oldest - 1,
                "active_requests": [{"request": request, **fields} for request, fields in self.active.items()],
            }

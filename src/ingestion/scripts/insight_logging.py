"""One logging setup for every Python job in the ingestion tree (insight#3191).

Emits the JSON line shape the gears services write — tracing-subscriber's
envelope — so one collector pipeline parses every service alike:

    {"timestamp": "2026-01-01T12:00:00.000000Z",
     "level": "INFO",
     "fields": {"message": "...", "...extra": "..."},
     "target": "<job name>"}

`level` is uppercase with WARN spelled the Rust way; extra key/values passed
via ``logger.info(msg, extra={...})`` land as siblings of ``message`` inside
``fields``. Lines go to stderr, matching the gears services, so a step's
stdout stays free for data (Argo outputs, JSON contracts between steps).

The level comes from INSIGHT_LOG_LEVEL — the install-wide knob the platform
ConfigMap publishes (debug|info|warn|error, default info). No job reads a
level knob of its own.
"""

from __future__ import annotations

import json
import logging
import os
import sys
from datetime import UTC, datetime

_LEVELS = {
    "debug": logging.DEBUG,
    "info": logging.INFO,
    "warn": logging.WARNING,
    "warning": logging.WARNING,
    "error": logging.ERROR,
}

_LEVEL_NAMES = {"DEBUG": "DEBUG", "INFO": "INFO", "WARNING": "WARN", "ERROR": "ERROR", "CRITICAL": "ERROR"}

#: Attributes every LogRecord carries; anything else on the record arrived via
#: ``extra=`` and belongs inside ``fields``.
_RECORD_ATTRS = frozenset(logging.LogRecord("", 0, "", 0, "", None, None).__dict__) | {"message", "asctime", "taskName"}


class _JsonFormatter(logging.Formatter):
    def format(self, record: logging.LogRecord) -> str:
        fields: dict[str, object] = {"message": record.getMessage()}
        for key, value in record.__dict__.items():
            if key not in _RECORD_ATTRS:
                fields[key] = value
        if record.exc_info:
            fields["exception"] = self.formatException(record.exc_info)

        timestamp = datetime.fromtimestamp(record.created, tz=UTC)
        line = {
            "timestamp": timestamp.strftime("%Y-%m-%dT%H:%M:%S.%fZ"),
            "level": _LEVEL_NAMES.get(record.levelname, record.levelname),
            "fields": fields,
            "target": record.name,
        }
        return json.dumps(line, default=str)


def level_from_env() -> int:
    """The install-wide level; an absent or unrecognised value means info."""
    return _LEVELS.get(os.environ.get("INSIGHT_LOG_LEVEL", "").strip().lower(), logging.INFO)


class _BoundFields(logging.Filter):
    """Stamps job-wide fields (a correlation id, ...) on every record."""

    def __init__(self, fields: dict[str, object]) -> None:
        super().__init__()
        self._fields = fields

    def filter(self, record: logging.LogRecord) -> bool:
        for key, value in self._fields.items():
            if key not in record.__dict__:
                record.__dict__[key] = value
        return True


def configure(target: str, **bound_fields: object) -> logging.Logger:
    """Install the shared JSON handler on the root logger and return `target`'s logger.

    Idempotent: reconfiguring replaces the handler rather than stacking a
    second one, so a line is never emitted twice. `bound_fields` land inside
    "fields" on every line; a per-call `extra` value wins over a bound one.
    """
    handler = logging.StreamHandler(sys.stderr)
    handler.setFormatter(_JsonFormatter())
    if bound_fields:
        handler.addFilter(_BoundFields(bound_fields))

    root = logging.getLogger()
    root.handlers = [handler]
    root.setLevel(level_from_env())
    return logging.getLogger(target)

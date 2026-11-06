from __future__ import annotations

import re
import subprocess
from collections.abc import Iterator
from contextlib import contextmanager

SENSITIVE = re.compile(
    r"\S*(?:password|passwd|token|credential|authorization|api[-_ ]?key|client[-_ ]?secret)\S*"
    r"|\b(?:pass|pwd|secret)\s*[:=]"
    r"|\b(?:bearer|basic)\b|\b(?:stringdata|data)\s*[:=]"
    r"|\w+://|[\"'{]|[\w+/=-]{20,}"
    r"|\b(?=[\w+/=-]{8,})(?=[\w+/=-]*[0-9_+/=-])[\w+/=-]+",
    re.IGNORECASE,
)


def safe_stderr(stderr: str | bytes | None) -> str:
    if isinstance(stderr, bytes):
        stderr = stderr.decode(errors="replace")
    lines = (stderr or "").splitlines()
    if not lines or not lines[0].strip():
        return "<empty stderr>"

    line = "".join(char for char in lines[0] if char.isprintable())
    sensitive = SENSITIVE.search(line)
    if sensitive:
        line = line[: sensitive.start()] + "[redacted]"
    return line[:300]


class ScheduleFailure(Exception):
    def __init__(self, stage: str, cause: Exception) -> None:
        self.cause = cause
        detail = type(cause).__name__
        if isinstance(cause, subprocess.CalledProcessError):
            detail += f"; kubectl exit code {cause.returncode}; {safe_stderr(cause.stderr)}"
        super().__init__(f"schedule {stage} failed: {detail}")


@contextmanager
def failure_stage(stage: str) -> Iterator[None]:
    try:
        yield
    except ScheduleFailure:
        raise
    except Exception as exc:
        raise ScheduleFailure(stage, exc) from None

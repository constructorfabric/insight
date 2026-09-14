"""What a failed subprocess is allowed to tell a test."""

from __future__ import annotations

TAIL_CHARS = 1500


def tail(stream: str | bytes | None, limit: int = TAIL_CHARS) -> str:
    """A stream's last characters. `TimeoutExpired` carries bytes where `run` gave str."""
    if stream is None:
        return ""
    text = stream.decode(errors="replace") if isinstance(stream, bytes) else stream
    return text[-limit:]

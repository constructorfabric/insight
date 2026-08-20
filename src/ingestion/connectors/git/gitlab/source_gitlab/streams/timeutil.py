from __future__ import annotations

from datetime import UTC, datetime, timedelta


def parse_iso(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=UTC)
    return parsed


def to_utc_z(moment: datetime) -> str:
    return moment.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


def subtract_minutes(iso_timestamp: str, minutes: int) -> str:
    return to_utc_z(parse_iso(iso_timestamp) - timedelta(minutes=minutes))

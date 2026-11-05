from __future__ import annotations


def render_schedule_fields(schedule: str, origin: str) -> dict:
    crons = [line.strip() for line in schedule.splitlines() if line.strip()]
    if not crons:
        raise ValueError("schedule carries no cron expression")
    if origin not in {"api", "annotation", "descriptor", "default"}:
        raise ValueError("invalid schedule origin")
    return {"schedules": crons, "timezone": "UTC", "origin": origin}

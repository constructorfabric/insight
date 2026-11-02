from collections.abc import Mapping
from typing import NamedTuple


class AppliedSchedule(NamedTuple):
    schedules: tuple[str, ...] = ()
    timezone: str = ""
    origin: str = ""

    def row_fields(self) -> dict[str, list[str] | str]:
        return {"schedules": list(self.schedules), "schedule_timezone": self.timezone, "schedule_origin": self.origin}


def read_schedule(work: Mapping[str, object]) -> AppliedSchedule:
    schedules = work.get("schedules", [])
    timezone = work.get("schedule_timezone", "")
    origin = work.get("schedule_origin", "")
    if not isinstance(schedules, list) or not all(isinstance(cron, str) for cron in schedules):
        return AppliedSchedule()
    if not isinstance(timezone, str) or not isinstance(origin, str):
        return AppliedSchedule()
    return AppliedSchedule(tuple(schedules), timezone, origin)

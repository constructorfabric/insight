from collections.abc import Mapping
from typing import NamedTuple


class AppliedSchedule(NamedTuple):
    schedules: tuple[str, ...] = ()
    timezone: str = ""
    origin: str = ""
    suspended: bool = False

    def row_fields(self) -> dict[str, list[str] | str | bool]:
        return {
            "schedules": list(self.schedules),
            "schedule_timezone": self.timezone,
            "schedule_origin": self.origin,
            "schedule_suspended": self.suspended,
        }


def read_schedule(work: Mapping[str, object]) -> AppliedSchedule:
    schedules = work.get("schedules", [])
    timezone = work.get("schedule_timezone", "")
    origin = work.get("schedule_origin", "")
    suspended = work.get("schedule_suspended", False)
    if not isinstance(schedules, list) or not all(isinstance(cron, str) for cron in schedules):
        return AppliedSchedule()
    if not isinstance(timezone, str) or not isinstance(origin, str) or not isinstance(suspended, bool):
        return AppliedSchedule()
    return AppliedSchedule(tuple(schedules), timezone, origin, suspended)

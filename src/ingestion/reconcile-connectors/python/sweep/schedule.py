from collections.abc import Mapping
from typing import NamedTuple


class AppliedSchedule(NamedTuple):
    schedules: tuple[str, ...] = ()
    timezone: str = ""
    origin: str = ""
    suspended: bool = False
    inherited_schedules: tuple[str, ...] = ()
    inherited_origin: str = ""
    requested: bool = False

    def row_fields(self) -> dict[str, list[str] | str | bool]:
        return {
            "schedules": list(self.schedules),
            "schedule_timezone": self.timezone,
            "schedule_origin": self.origin,
            "schedule_suspended": self.suspended,
            "inherited_schedules": list(self.inherited_schedules),
            "inherited_schedule_origin": self.inherited_origin,
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
    inherited = work.get("inherited_schedules", [])
    inherited_origin = work.get("inherited_schedule_origin", "")
    requested = work.get("schedule_requested", False)
    if not isinstance(inherited, list) or not all(isinstance(cron, str) for cron in inherited):
        inherited = []
    if not isinstance(inherited_origin, str) or not isinstance(requested, bool):
        return AppliedSchedule()
    return AppliedSchedule(tuple(schedules), timezone, origin, suspended, tuple(inherited), inherited_origin, requested)

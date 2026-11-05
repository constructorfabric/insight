import logging
from collections.abc import Mapping

from .plan import Instance
from .schedule import AppliedSchedule

_LOG = logging.getLogger("sweep")


def warn_shadowed_changes(
    schedules: Mapping[Instance, AppliedSchedule], previous: Mapping[Instance, tuple[tuple[str, ...], str]]
) -> None:
    for instance, schedule in schedules.items():
        before = previous.get(instance)
        if not schedule.requested or not before or not before[0] or not schedule.inherited_schedules:
            continue
        current = (schedule.inherited_schedules, schedule.inherited_origin)
        if before[0] == current[0]:
            continue
        _LOG.warning(
            "deployment schedule changed for %s/%s/%s: %s (%s) -> %s (%s); overridden by API request",
            instance.connector,
            instance.tenant_id,
            instance.source_id,
            "; ".join(before[0]),
            before[1],
            "; ".join(current[0]),
            current[1],
        )

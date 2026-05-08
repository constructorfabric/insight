#!/usr/bin/env python3
# ---------------------------------------------------------------------------
# build_schedule_json.py <cron_expression>
#
# Emit the Airbyte ConnectionCreate schedule fields (1.7+ shape) for the
# given cron string. The output is a flat JSON object with `scheduleType`
# and (for cron) `scheduleData`; the caller splices it directly into the
# ConnectionCreate body via Python `dict.update`.
#
# Outputs:
#   - empty / "manual" -> {"scheduleType":"manual"}
#   - cron expression  -> {"scheduleType":"cron",
#                          "scheduleData":{"cron":{"cronExpression":"<cron>",
#                                                  "cronTimeZone":"UTC"}}}
#
# We deliberately avoid the legacy `schedule` field (basic schedule with
# `timeUnit` + `units`). Airbyte 1.7+ requires `timeUnit` to be non-null
# inside `scheduleData.basicSchedule`; sending an empty basic schedule
# (which is what mis-shaped legacy bodies stored) poisons the
# connection_manager Temporal workflow — the worker raises
# `MissingKotlinParameterException: timeUnit` on every signal and no
# Sync job is ever created. See AIRBYTE-DEPLOY-NOTES.md.
# ---------------------------------------------------------------------------

import json
import sys


def _to_quartz(cron: str) -> str:
    """Convert standard 5-field cron to Quartz 6-field accepted by Airbyte.

    Standard cron:  min hour dom month dow
    Quartz cron:    sec min hour dom month dow [year]

    Airbyte (1.7+) validates incoming cron via Quartz and rejects 5-field
    standard cron with `error:cron-validation/invalid-expression`. We
    prepend `0` for seconds and replace `*` in the day-of-week field with
    `?` if the day-of-month is also `*` (Quartz forbids both `*` because
    they conflict).

    Already-Quartz expressions (>=6 fields) pass through unchanged.
    """
    fields = cron.split()
    if len(fields) >= 6:
        return cron
    if len(fields) != 5:
        # Hand back as-is; Airbyte will reject with a meaningful error.
        return cron
    minute, hour, dom, month, dow = fields
    if dom == "*" and dow == "*":
        dow = "?"
    return " ".join(["0", minute, hour, dom, month, dow])


def main() -> int:
    cron = (sys.argv[1] if len(sys.argv) > 1 else "").strip()
    if not cron or cron.lower() == "manual":
        out = {"scheduleType": "manual"}
    else:
        out = {
            "scheduleType": "cron",
            "scheduleData": {
                "cron": {
                    "cronExpression": _to_quartz(cron),
                    "cronTimeZone":   "UTC",
                },
            },
        }
    json.dump(out, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())

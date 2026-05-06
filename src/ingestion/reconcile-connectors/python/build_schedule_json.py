#!/usr/bin/env python3
# ---------------------------------------------------------------------------
# build_schedule_json.py <cron_expression>
#
# Emit an Airbyte connection schedule JSON for a given cron string:
#   {"scheduleType":"cron","cronExpression":"<cron>"}
# Empty / "manual" cron yields {"scheduleType":"manual"}.
#
# Used by reconcile_connections to seed a fresh connection's schedule from
# the cron resolved by reconcile_compute_schedule (Secret annotation /
# descriptor.yaml.schedule / default).
# ---------------------------------------------------------------------------

import json
import sys


def main() -> int:
    cron = (sys.argv[1] if len(sys.argv) > 1 else "").strip()
    if not cron or cron.lower() == "manual":
        out = {"scheduleType": "manual"}
    else:
        out = {"scheduleType": "cron", "cronExpression": cron}
    json.dump(out, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())

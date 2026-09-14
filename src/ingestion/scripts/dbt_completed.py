"""Emit the dbt.completed lifecycle line for a dbt workflow step.

One structured line per run for the high-level "dbt finished after N minutes,
success/failed" view; inputs ride the environment so whatever characters land
in DBT_SELECT never touch JSON or shell text.
"""

from __future__ import annotations

import os
import sys

import insight_logging


def main() -> int:
    log = insight_logging.configure("dbt")

    rc = int(os.environ["DBT_RC"])
    level = log.info if rc == 0 else log.error
    level(
        "dbt run finished",
        extra={
            "event": "dbt.completed",
            "status": "success" if rc == 0 else "failed",
            "exit_code": rc,
            "duration_ms": int(os.environ["DBT_DURATION_MS"]),
            "select": os.environ.get("DBT_SELECT", ""),
            "exclude": os.environ.get("DBT_EXCLUDE", ""),
            "full_refresh": os.environ.get("DBT_FULL_REFRESH", "false") == "true",
        },
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())

from __future__ import annotations

import json
import subprocess
import sys

from schedule_failure import ScheduleFailure, failure_stage


def main() -> int:
    try:
        listing = sys.argv[2] == "--list"
        with failure_stage("cronworkflow list" if listing else "cronworkflow readback"):
            target = (
                ["cronworkflows.argoproj.io", "-l", "app.kubernetes.io/component=connector-sync"]
                if listing
                else [f"cronworkflow.argoproj.io/{sys.argv[2]}"]
            )
            result = subprocess.run(
                ["kubectl", "-n", sys.argv[1], "get", *target, "-o", "json"],
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            result.check_returncode()
            observed = json.loads(result.stdout)
            if not isinstance(observed, dict):
                raise ValueError("invalid CronWorkflow readback")
            if listing and not isinstance(observed.get("items"), list):
                raise ValueError("invalid CronWorkflow listing")
            print(json.dumps(observed))
        return 0
    except ScheduleFailure as exc:
        print(str(exc), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

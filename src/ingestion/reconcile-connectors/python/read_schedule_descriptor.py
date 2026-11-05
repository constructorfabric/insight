from __future__ import annotations

import sys
from pathlib import Path

import yaml


def read_schedule(path: Path) -> tuple[str, ...]:
    document = yaml.safe_load(path.read_text())
    if not isinstance(document, dict):
        raise ValueError("invalid descriptor")
    schedule = document.get("schedule")
    if schedule is None:
        return ()
    crons = [schedule] if isinstance(schedule, str) else schedule
    if not isinstance(crons, list) or any(not isinstance(cron, str) for cron in crons):
        raise ValueError("invalid descriptor schedule")
    return tuple(crons)


if __name__ == "__main__":
    print("\n".join(read_schedule(Path(sys.argv[1]))), end="")

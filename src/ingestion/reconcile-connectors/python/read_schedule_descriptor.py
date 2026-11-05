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


def validate_directory(root: Path) -> None:
    if not root.is_dir():
        raise ValueError("missing descriptor directory")
    for path in root.rglob("descriptor.yaml"):
        read_schedule(path)
        document = yaml.safe_load(path.read_text())
        if not document.get("name") or document.get("version") is None:
            raise ValueError("descriptor requires name and version")


if __name__ == "__main__":
    if sys.argv[1] == "--directory":
        validate_directory(Path(sys.argv[2]))
    else:
        print("\n".join(read_schedule(Path(sys.argv[1]))), end="")

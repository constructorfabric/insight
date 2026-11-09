#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
import subprocess
from pathlib import Path

CHART = re.compile(r"src/[\w./-]+/helm/Chart\.yaml")
APP_VERSION = re.compile(r'^appVersion:\s*"?[^"\s]*-([0-9a-f]{7,40})"?\s*$', re.MULTILINE)


def pinned_charts(root: Path) -> list[Path]:
    script = (root / "dev-compose.sh").read_text()

    return [root / chart for chart in dict.fromkeys(CHART.findall(script))]


def build_commit(chart: Path) -> str | None:
    match = APP_VERSION.search(chart.read_text()) if chart.is_file() else None

    return match.group(1) if match else None


def comparison_point(root: Path) -> str | None:
    commits = [build_commit(chart) for chart in pinned_charts(root)]
    if not commits or None in commits:
        return None

    result = subprocess.run(
        ["git", "-C", str(root), "merge-base", "--octopus", *commits],
        capture_output=True, text=True,
    )

    return result.stdout.strip() or None if result.returncode == 0 else None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()

    print(comparison_point(args.root) or "")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

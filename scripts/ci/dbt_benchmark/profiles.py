from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import yaml
from fixtures import query_settings


def profile(source: Path, profiles: Path) -> dict[str, str | int]:
    subprocess.run(
        [
            sys.executable,
            str(source / "src/ingestion/scripts/dbt_profiles.py"),
            "--target",
            "benchmark",
            "--profiles-dir",
            str(profiles),
        ],
        check=True,
    )
    output = yaml.safe_load((profiles / "profiles.yml").read_text())["ingestion"]["outputs"]["benchmark"]
    return query_settings(output["custom_settings"]) if "custom_settings" in output else {}

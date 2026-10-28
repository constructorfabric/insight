"""The dbt project's manifest, parsed without a warehouse.

Parsing never connects: the profile is a placeholder written by the deploy's own
profile writer, so a body dbt would reject fails here rather than on the install
that first runs it.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

import yaml

# dbt cannot share an environment with mypy — see the `datapath` dependency group.
from dbt.cli.main import dbtRunner  # type: ignore[import-not-found]

REPO_ROOT = Path(__file__).resolve().parents[3]
DBT_PROJECT = REPO_ROOT / "src/ingestion/dbt"

# The deploy's own profile writer — a top-level module in the toolbox image.
sys.path.insert(0, str(REPO_ROOT / "src/ingestion/scripts"))

from dbt_profiles import Connection, build_profile  # type: ignore[import-not-found] # noqa: E402

Manifest = dict[str, Any]


def _write_profile(directory: Path) -> Path:
    directory.mkdir(parents=True)
    profile = build_profile(
        "parse",
        Connection(host="parse.invalid", port=8123, user="parse", password="parse", schema="parse"),
        correlated_subqueries=False,
    )
    (directory / "profiles.yml").write_text(yaml.safe_dump(profile), encoding="utf-8")
    return directory


def parse_project(workspace: Path) -> Manifest:
    """Parse into `workspace`, ignoring any cache: the environment may change what parses."""
    profiles_dir = _write_profile(workspace / "profiles")
    target_dir = workspace / "target"
    result = dbtRunner().invoke(
        [
            "parse",
            "--no-partial-parse",
            "--project-dir",
            str(DBT_PROJECT),
            "--profiles-dir",
            str(profiles_dir),
            "--target-path",
            str(target_dir),
        ]
    )
    if not result.success:
        raise AssertionError(f"dbt parse failed: {result.exception}")

    manifest: Manifest = json.loads((target_dir / "manifest.json").read_text(encoding="utf-8"))
    return manifest

#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.request
from collections.abc import Callable
from urllib.parse import quote

Fetch = Callable[[str], dict]


def superseding_run(current: dict, runs: list[dict]) -> dict | None:
    return next(
        (
            run
            for run in runs
            if run["id"] != current["id"]
            and run["status"] != "completed"
            and run["created_at"] > current["created_at"]
        ),
        None,
    )


def find(fetch: Fetch, repo: str, run_id: int) -> dict | None:
    current = fetch(f"/repos/{repo}/actions/runs/{run_id}")

    branch = quote(current["head_branch"], safe="")
    listing = fetch(f"/repos/{repo}/actions/workflows/{current['workflow_id']}/runs?branch={branch}&per_page=50")

    return superseding_run(current, listing["workflow_runs"])


def github(path: str) -> dict:
    request = urllib.request.Request(
        os.environ.get("GITHUB_API_URL", "https://api.github.com") + path,
        headers={
            "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
            "Accept": "application/vnd.github+json",
        },
    )

    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def main(argv: list[str] | None = None, fetch: Fetch = github) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY"))
    parser.add_argument("--run-id", type=int, default=os.environ.get("GITHUB_RUN_ID"))
    args = parser.parse_args(argv)

    try:
        run = find(fetch, args.repo, args.run_id)
    except (OSError, KeyError, ValueError) as err:
        print(f"cannot list this workflow's runs: {err}", file=sys.stderr)
        return 1

    if run is None:
        return 1

    print(run["html_url"])

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

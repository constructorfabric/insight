#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

Command = Callable[[list[str]], str]


@dataclass(frozen=True)
class ArtifactRun:
    run_id: int
    conclusion: str
    url: str


@dataclass(frozen=True)
class ImageRequest:
    name: str
    required: bool


def run_command(command: list[str]) -> str:
    return subprocess.run(command, check=True, capture_output=True, text=True).stdout


def find_run(head_sha: str, command: Command) -> ArtifactRun | None:
    output = command(
        [
            "gh",
            "api",
            f"repos/{os.environ['GITHUB_REPOSITORY']}/actions/runs?head_sha={head_sha}&per_page=100",
            "--jq",
            '[.workflow_runs[] | select(.name == "Build & Push Container Images")] | sort_by(.run_started_at) | last // empty',
        ]
    ).strip()
    if not output:
        return None

    payload = json.loads(output)
    return ArtifactRun(int(payload["id"]), str(payload.get("conclusion") or ""), str(payload["html_url"]))


def wait_for_run(
    head_sha: str,
    require_run: bool,
    attempts: int,
    interval_seconds: int,
    command: Command,
) -> ArtifactRun | None:
    for attempt in range(1, attempts + 1):
        run = find_run(head_sha, command)
        if run is None:
            if not require_run:
                return None
        elif run.conclusion == "success":
            return run
        elif run.conclusion:
            raise RuntimeError(f"image-build run concluded {run.conclusion!r}: {run.url}")

        if attempt < attempts:
            time.sleep(interval_seconds)

    raise RuntimeError("image-build run did not complete within the polling window")


def artifact_names(run: ArtifactRun, command: Command) -> set[str]:
    output = command(
        [
            "gh",
            "api",
            f"repos/{os.environ['GITHUB_REPOSITORY']}/actions/runs/{run.run_id}/artifacts?per_page=100",
        ]
    )
    payload = json.loads(output)
    return {str(artifact["name"]) for artifact in payload["artifacts"]}


def materialize_images(
    run: ArtifactRun | None,
    images: list[ImageRequest],
    registry: str,
    destination: Path,
    command: Command = run_command,
) -> None:
    available = artifact_names(run, command) if run else set()
    destination.mkdir(parents=True, exist_ok=True)

    for image in images:
        artifact = f"e2e-prebuilt-{image.name}"
        local_tag = f"{image.name}:e2e-prebuilt"
        if artifact in available:
            artifact_dir = destination / artifact
            command(
                [
                    "gh",
                    "run",
                    "download",
                    str(run.run_id),
                    "-R",
                    os.environ["GITHUB_REPOSITORY"],
                    "-n",
                    artifact,
                    "-D",
                    str(artifact_dir),
                ]
            )
            tarball = artifact_dir / "e2e-prebuilt.tar"
            if not tarball.is_file():
                raise RuntimeError(f"artifact {artifact!r} did not contain e2e-prebuilt.tar")
            command(["docker", "load", "--input", str(tarball)])
            continue

        if image.required:
            raise RuntimeError(f"changed image {image.name!r} has no artifact in the successful build run")

        published = f"{registry}/{image.name}:latest"
        command(["docker", "pull", published])
        command(["docker", "tag", published, local_tag])


def parse_image(value: str) -> ImageRequest:
    name, separator, policy = value.partition(":")
    if not name or separator != ":" or policy not in {"required", "unchanged"}:
        raise argparse.ArgumentTypeError("image must be NAME:required or NAME:unchanged")
    return ImageRequest(name, required=policy == "required")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--head-sha", required=True)
    parser.add_argument("--registry", required=True)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--attempts", required=True, type=int)
    parser.add_argument("--interval-seconds", required=True, type=int)
    parser.add_argument("--image", required=True, action="append", type=parse_image)
    args = parser.parse_args()

    try:
        run = wait_for_run(
            args.head_sha,
            any(image.required for image in args.image),
            args.attempts,
            args.interval_seconds,
            run_command,
        )
        materialize_images(run, args.image, args.registry, args.destination)
    except (json.JSONDecodeError, KeyError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

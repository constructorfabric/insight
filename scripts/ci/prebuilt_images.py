#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

Command = Callable[[list[str]], str]


@dataclass(frozen=True)
class ImageRequest:
    name: str
    required: bool


def run_command(command: list[str]) -> str:
    return subprocess.run(command, check=True, capture_output=True, text=True).stdout


def artifact_names(run_id: int, command: Command) -> set[str]:
    output = command(
        [
            "gh",
            "api",
            f"repos/{os.environ['GITHUB_REPOSITORY']}/actions/runs/{run_id}/artifacts?per_page=100",
        ]
    )
    payload = json.loads(output)
    return {str(artifact["name"]) for artifact in payload["artifacts"]}


def materialize_images(
    run_id: int,
    images: list[ImageRequest],
    registry: str,
    destination: Path,
    command: Command = run_command,
) -> None:
    available = artifact_names(run_id, command)
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
                    str(run_id),
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
    parser.add_argument("--run-id", required=True, type=int)
    parser.add_argument("--registry", required=True)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--image", required=True, action="append", type=parse_image)
    args = parser.parse_args()

    try:
        materialize_images(args.run_id, args.image, args.registry, args.destination)
    except (json.JSONDecodeError, KeyError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

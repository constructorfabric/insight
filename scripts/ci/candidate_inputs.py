from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

FILES = {"source-sha", "version", "release.patch", "values.json"}
COMPOSE_IMAGES = {
    "analytics": "ANALYTICS_IMAGE",
    "authenticator": "AUTHENTICATOR_IMAGE",
    "identityResolution": "IDENTITY_RESOLUTION_IMAGE",
    "gateway": "GATEWAY_IMAGE",
    "insightV3Core": "INSIGHT_V3_CORE_IMAGE",
    "frontend": "FRONTEND_IMAGE",
}
IMAGE = re.compile(r"[a-z0-9][a-z0-9./:_-]*:[A-Za-z0-9._-]+@sha256:[0-9a-f]{64}")


def verify(directory: Path, source_sha: str) -> dict[str, dict[str, dict[str, str]]]:
    if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
        raise ValueError("Invalid source SHA")
    seen: set[str] = set()
    for line in (directory / "SHA256SUMS").read_text().splitlines():
        checksum, name = line.split("  ", 1)
        if name not in FILES or name in seen:
            raise ValueError("Unexpected or repeated candidate file")
        seen.add(name)
        if hashlib.sha256((directory / name).read_bytes()).hexdigest() != checksum:
            raise ValueError(f"Candidate checksum mismatch: {name}")
    if seen != FILES:
        raise ValueError("Incomplete candidate checksums")
    if (directory / "source-sha").read_text().strip() != source_sha:
        raise ValueError("Candidate belongs to another source revision")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", (directory / "version").read_text().strip()):
        raise ValueError("Invalid candidate version")
    values = json.loads((directory / "values.json").read_text())
    compose_images(values)
    return values


def compose_images(values: dict[str, dict[str, dict[str, str]]]) -> dict[str, str]:
    images: dict[str, str] = {}
    for alias, variable in COMPOSE_IMAGES.items():
        definition = values[alias]["image"]
        image = f"{definition['repository']}:{definition['tag']}"
        if not IMAGE.fullmatch(image):
            raise ValueError(f"Candidate image must have a digest: {alias}")
        images[variable] = image
    return images


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("source_sha")
    parser.add_argument("--compose-env", type=Path)
    args = parser.parse_args()
    values = verify(args.directory, args.source_sha)
    if args.compose_env is not None:
        args.compose_env.write_text("".join(f"{key}={value}\n" for key, value in compose_images(values).items()))


if __name__ == "__main__":
    main()

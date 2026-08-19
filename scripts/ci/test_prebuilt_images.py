from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

from prebuilt_images import ImageRequest, materialize_images


class FakeCommands:
    def __init__(self, artifacts: list[str]) -> None:
        self.artifacts = artifacts
        self.calls: list[list[str]] = []

    def __call__(self, command: list[str]) -> str:
        self.calls.append(command)
        if command[:2] == ["gh", "api"]:
            return json.dumps({"artifacts": [{"name": name} for name in self.artifacts]})
        if command[:3] == ["gh", "run", "download"]:
            destination = Path(command[command.index("-D") + 1])
            destination.mkdir(parents=True)
            (destination / "e2e-prebuilt.tar").touch()
        return ""


class MaterializeImagesTests(unittest.TestCase):
    def setUp(self) -> None:
        os.environ["GITHUB_REPOSITORY"] = "example/repository"

    def test_loads_available_artifact_and_pulls_unchanged_image(self) -> None:
        commands = FakeCommands(["e2e-prebuilt-insight-gateway"])
        images = [
            ImageRequest("insight-gateway", required=True),
            ImageRequest("insight-authenticator", required=False),
        ]

        with tempfile.TemporaryDirectory() as directory:
            materialize_images(42, images, "ghcr.io/example", Path(directory), commands)

        self.assertIn(
            ["docker", "load", "--input", str(Path(directory) / "e2e-prebuilt-insight-gateway/e2e-prebuilt.tar")],
            commands.calls,
        )
        self.assertIn(["docker", "pull", "ghcr.io/example/insight-authenticator:latest"], commands.calls)
        self.assertIn(
            [
                "docker",
                "tag",
                "ghcr.io/example/insight-authenticator:latest",
                "insight-authenticator:e2e-prebuilt",
            ],
            commands.calls,
        )

    def test_rejects_missing_artifact_for_changed_image(self) -> None:
        commands = FakeCommands([])

        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "insight-gateway"):
                materialize_images(
                    42,
                    [ImageRequest("insight-gateway", required=True)],
                    "ghcr.io/example",
                    Path(directory),
                    commands,
                )

    def test_rejects_download_without_tarball(self) -> None:
        commands = FakeCommands(["e2e-prebuilt-insight-gateway"])
        commands.artifacts = ["e2e-prebuilt-insight-gateway"]

        def omit_tarball(command: list[str]) -> str:
            commands.calls.append(command)
            if command[:2] == ["gh", "api"]:
                return json.dumps({"artifacts": [{"name": commands.artifacts[0]}]})
            return ""

        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "did not contain"):
                materialize_images(
                    42,
                    [ImageRequest("insight-gateway", required=True)],
                    "ghcr.io/example",
                    Path(directory),
                    omit_tarball,
                )

    def test_propagates_artifact_download_failure(self) -> None:
        def fail_download(command: list[str]) -> str:
            if command[:2] == ["gh", "api"]:
                return json.dumps({"artifacts": [{"name": "e2e-prebuilt-insight-gateway"}]})
            raise subprocess.CalledProcessError(1, command)

        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(subprocess.CalledProcessError):
                materialize_images(
                    42,
                    [ImageRequest("insight-gateway", required=True)],
                    "ghcr.io/example",
                    Path(directory),
                    fail_download,
                )

    def test_missing_artifact_uses_published_image_when_component_is_unchanged(self) -> None:
        commands = FakeCommands([])
        with tempfile.TemporaryDirectory() as directory:
            materialize_images(
                42,
                [ImageRequest("insight-gateway", required=False)],
                "ghcr.io/example",
                Path(directory),
                commands,
            )

        self.assertIn(["docker", "pull", "ghcr.io/example/insight-gateway:latest"], commands.calls)


if __name__ == "__main__":
    unittest.main()

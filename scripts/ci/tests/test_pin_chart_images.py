from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SCRIPT = ROOT / ".github/workflows/scripts/pin-chart-images.sh"
DIGEST = "sha256:" + "a" * 64


@unittest.skipUnless(shutil.which("yq"), "requires mikefarah yq")
class PinChartImagesTests(unittest.TestCase):
    def run_pinner(self, root: Path, digest: str) -> subprocess.CompletedProcess[str]:
        chart = root / "charts/insight"
        dependency = root / "charts/service"
        chart.mkdir(parents=True)
        dependency.mkdir()
        (chart / "Chart.yaml").write_text("dependencies:\n  - alias: service\n    repository: file://../service\n")
        (chart / "values.yaml").write_text(
            'service:\n  image:\n    repository: example.invalid/service\n    tag: ""\n'
            "ingestion:\n  toolboxImage: example.invalid/toolbox:v1\n"
            "  seedImage: example.invalid/seed:v1\n"
        )
        (dependency / "Chart.yaml").write_text("appVersion: v1\n")
        (dependency / "values.yaml").write_text('image:\n  tag: ""\n')
        docker = root / "docker"
        docker.write_text('#!/bin/sh\nprintf \'{"digest":"%s"}\\n\' "$TEST_DIGEST"\n')
        docker.chmod(0o755)
        env = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}", TEST_DIGEST=digest)
        return subprocess.run(
            ["bash", str(SCRIPT)], check=False, cwd=root, env=env, capture_output=True, text=True, timeout=10
        )

    def test_service_and_ingestion_images_are_pinned_to_digests(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = self.run_pinner(root, DIGEST)
            self.assertEqual(result.returncode, 0, result.stderr)
            values = (root / "charts/insight/values.yaml").read_text()
            self.assertIn(f"v1@{DIGEST}", values)
            self.assertIn(f"example.invalid/toolbox:v1@{DIGEST}", values)
            self.assertIn(f"example.invalid/seed:v1@{DIGEST}", values)

    def test_invalid_registry_digest_stops_packaging(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_pinner(Path(directory), "null")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Invalid image digest", result.stderr)


if __name__ == "__main__":
    unittest.main()

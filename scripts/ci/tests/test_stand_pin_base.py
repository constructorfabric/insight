from __future__ import annotations

import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2].parent
SCRIPT = ROOT / "scripts" / "ci" / "stand_pin_base.py"
sys.path.insert(0, str(ROOT / "scripts" / "ci"))

import stand_pin_base  # noqa: E402

DEV_COMPOSE = """
test_stand_frontend_image() {
  test_stand_pinned_image src/frontend/helm/Chart.yaml frontend
}
TEST_STAND_PINNED_BACKENDS=(
  "ANALYTICS_IMAGE|src/backend/services/analytics/helm/Chart.yaml|analytics"
)
"""


def git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args], check=True, capture_output=True, text=True
    ).stdout.strip()


def commit(repo: Path, message: str) -> str:
    git(repo, "commit", "--allow-empty", "-q", "-m", message)

    return git(repo, "rev-parse", "HEAD")


def pin(repo: Path, chart: str, app_version: str) -> None:
    path = repo / chart
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f'apiVersion: v2\nname: x\nversion: 0.1.0\nappVersion: "{app_version}"\n')


class StandRepository(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = Path(self.tmp.name)

        git(self.repo, "init", "-q")
        git(self.repo, "config", "user.email", "ci@example.invalid")
        git(self.repo, "config", "user.name", "ci")
        (self.repo / "dev-compose.sh").write_text(DEV_COMPOSE)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def test_the_base_is_the_oldest_commit_a_pinned_image_was_built_from(self) -> None:
        oldest = commit(self.repo, "analytics built here")
        newer = commit(self.repo, "frontend built here")
        commit(self.repo, "analytics changed after its image")

        pin(self.repo, "src/backend/services/analytics/helm/Chart.yaml", f"2026.11.06.14.31-{oldest[:7]}")
        pin(self.repo, "src/frontend/helm/Chart.yaml", f"2026.11.09.08.15-{newer[:7]}")

        self.assertEqual(stand_pin_base.comparison_point(self.repo), oldest)

    def test_a_pin_naming_a_commit_this_history_lacks_has_no_base(self) -> None:
        built = commit(self.repo, "frontend built here")

        pin(self.repo, "src/backend/services/analytics/helm/Chart.yaml", "2026.11.06.14.31-abcdef0")
        pin(self.repo, "src/frontend/helm/Chart.yaml", f"2026.11.09.08.15-{built[:7]}")

        self.assertIsNone(stand_pin_base.comparison_point(self.repo))

    def test_a_chart_without_a_build_commit_has_no_base(self) -> None:
        built = commit(self.repo, "frontend built here")

        pin(self.repo, "src/backend/services/analytics/helm/Chart.yaml", "latest")
        pin(self.repo, "src/frontend/helm/Chart.yaml", f"2026.11.09.08.15-{built[:7]}")

        self.assertIsNone(stand_pin_base.comparison_point(self.repo))

    def test_the_script_prints_the_base_and_nothing_when_there_is_none(self) -> None:
        built = commit(self.repo, "everything built here")

        pin(self.repo, "src/backend/services/analytics/helm/Chart.yaml", f"2026.11.06.14.31-{built[:7]}")
        pin(self.repo, "src/frontend/helm/Chart.yaml", f"2026.11.06.14.31-{built[:7]}")
        found = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.repo)],
            check=True, capture_output=True, text=True,
        )

        pin(self.repo, "src/frontend/helm/Chart.yaml", "latest")
        missing = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.repo)],
            check=True, capture_output=True, text=True,
        )

        self.assertEqual(found.stdout.strip(), built)
        self.assertEqual(missing.stdout.strip(), "")


class Repository(unittest.TestCase):
    def test_the_stand_pins_the_frontend_and_every_backend(self) -> None:
        charts = {str(path.relative_to(ROOT)) for path in stand_pin_base.pinned_charts(ROOT)}

        self.assertIn("src/frontend/helm/Chart.yaml", charts)
        self.assertIn("src/backend/services/analytics/helm/Chart.yaml", charts)
        self.assertGreaterEqual(len(charts), 5)

    def test_every_pinned_chart_names_the_commit_its_image_was_built_from(self) -> None:
        for chart in stand_pin_base.pinned_charts(ROOT):
            with self.subTest(chart=str(chart.relative_to(ROOT))):
                self.assertRegex(stand_pin_base.build_commit(chart) or "", re.compile(r"^[0-9a-f]{7,40}$"))


if __name__ == "__main__":
    unittest.main()

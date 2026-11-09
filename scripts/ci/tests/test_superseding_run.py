from __future__ import annotations

import io
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2].parent
sys.path.insert(0, str(ROOT / "scripts" / "ci"))

import superseding_run  # noqa: E402

CURRENT = {"id": 7, "status": "in_progress", "created_at": "2026-11-09T12:03:25Z", "html_url": "https://x/7"}


def run(run_id: int, status: str, created_at: str) -> dict:
    return {"id": run_id, "status": status, "created_at": created_at, "html_url": f"https://x/{run_id}"}


class Selection(unittest.TestCase):
    def test_a_newer_run_waiting_on_the_branch_supersedes(self) -> None:
        newer = run(8, "pending", "2026-11-09T12:40:55Z")

        self.assertEqual(superseding_run.superseding_run(CURRENT, [newer, CURRENT]), newer)

    def test_a_finished_newer_run_does_not(self) -> None:
        newer = run(8, "completed", "2026-11-09T12:40:55Z")

        self.assertIsNone(superseding_run.superseding_run(CURRENT, [newer, CURRENT]))

    def test_an_older_unfinished_run_does_not(self) -> None:
        older = run(6, "queued", "2026-11-09T11:00:00Z")

        self.assertIsNone(superseding_run.superseding_run(CURRENT, [CURRENT, older]))


class Lookup(unittest.TestCase):
    def fetch(self, path: str) -> dict:
        self.requested.append(path)
        if path == "/repos/o/r/actions/runs/7":
            return {**CURRENT, "workflow_id": 42, "head_branch": "main"}

        return {"workflow_runs": self.runs}

    def setUp(self) -> None:
        self.requested: list[str] = []
        self.runs = [run(8, "pending", "2026-11-09T12:40:55Z"), CURRENT]

    def test_the_runs_compared_are_the_same_workflow_on_the_same_branch(self) -> None:
        found = superseding_run.find(self.fetch, "o/r", 7)

        self.assertEqual(found["id"], 8)
        self.assertEqual(self.requested[1].split("?")[0], "/repos/o/r/actions/workflows/42/runs")
        self.assertIn("branch=main", self.requested[1])

    def test_the_cli_names_the_superseding_run(self) -> None:
        out = io.StringIO()
        with redirect_stdout(out):
            code = superseding_run.main(["--repo", "o/r", "--run-id", "7"], fetch=self.fetch)

        self.assertEqual(code, 0)
        self.assertEqual(out.getvalue().strip(), "https://x/8")

    def test_the_cli_fails_when_nothing_supersedes(self) -> None:
        self.runs = [CURRENT]

        with redirect_stdout(io.StringIO()):
            code = superseding_run.main(["--repo", "o/r", "--run-id", "7"], fetch=self.fetch)

        self.assertEqual(code, 1)

    def test_the_cli_fails_when_github_cannot_answer(self) -> None:
        def broken(path: str) -> dict:
            raise OSError("HTTP 502")

        with redirect_stdout(io.StringIO()):
            code = superseding_run.main(["--repo", "o/r", "--run-id", "7"], fetch=broken)

        self.assertEqual(code, 1)


if __name__ == "__main__":
    unittest.main()

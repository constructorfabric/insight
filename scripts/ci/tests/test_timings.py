from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from timings import duration, render, summarize


class TimingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.run = {
            "id": 7,
            "run_attempt": 2,
            "name": "Example CI",
            "event": "pull_request",
            "head_sha": "a" * 40,
            "status": "completed",
            "conclusion": "failure",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:04:00Z",
        }
        self.job = {
            "id": 8,
            "name": "Example test",
            "conclusion": "failure",
            "created_at": "2026-01-01T00:01:00Z",
            "started_at": "2026-01-01T00:02:00Z",
            "completed_at": "2026-01-01T00:04:00Z",
            "steps": [
                {
                    "name": "Tests",
                    "conclusion": "failure",
                    "started_at": "2026-01-01T00:02:30Z",
                    "completed_at": "2026-01-01T00:04:00Z",
                }
            ],
        }

    def test_pending_execution_and_step_time_are_separate(self) -> None:
        result = summarize(self.run, [self.job])
        self.assertEqual(result["attempt"], 2)
        self.assertEqual(result["conclusion"], "failure")
        self.assertEqual(result["elapsed_seconds"], 240)
        job = result["jobs"][0]
        self.assertEqual(job["pending_seconds"], 60)
        self.assertEqual(job["execution_seconds"], 120)
        self.assertEqual(job["steps"][0]["execution_seconds"], 90)

    def test_missing_pending_timestamp_remains_unknown(self) -> None:
        del self.job["created_at"]
        result = summarize(self.run, [self.job])
        self.assertIsNone(result["jobs"][0]["pending_seconds"])
        self.assertIn("unknown", render(result))

    def test_skipped_work_is_not_reported_as_zero_cost(self) -> None:
        self.job.update(
            conclusion="skipped", started_at="2026-01-01T00:02:00Z", completed_at="2026-01-01T00:01:59Z", steps=[]
        )
        result = summarize(self.run, [self.job])
        job = result["jobs"][0]
        self.assertEqual(job["conclusion"], "skipped")
        self.assertIsNone(job["pending_seconds"])
        self.assertIsNone(job["execution_seconds"])

    def test_incomplete_runs_are_rejected(self) -> None:
        self.run["status"] = "in_progress"
        with self.assertRaisesRegex(ValueError, "completed workflow"):
            summarize(self.run, [self.job])

    def test_invalid_timestamp_pairs_are_rejected(self) -> None:
        pairs = [("2026-01-01T00:02:00Z", "2026-01-01T00:01:00Z"), ("2026-01-01T00:00:00", "2026-01-01T00:01:00")]
        for start, end in pairs:
            with self.subTest(start=start, end=end), self.assertRaises(ValueError):
                duration(start, end)

    def test_parallel_jobs_are_not_added_to_elapsed_time(self) -> None:
        result = summarize(self.run, [self.job, self.job, self.job])
        self.assertEqual(result["elapsed_seconds"], 240)

    def test_job_names_cannot_inject_table_rows_or_html(self) -> None:
        self.job["name"] = "test|extra\n<script>[link](example)"
        report = render(summarize(self.run, [self.job]))
        self.assertNotIn("<script>", report)
        self.assertNotIn("\n<script>", report)
        self.assertIn("test\\|extra &lt;script&gt;\\[link\\]", report)

    def test_cli_writes_json_and_preserves_existing_summary(self) -> None:
        script = Path(__file__).resolve().parents[1] / "timings.py"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            event, jobs = root / "event.json", root / "jobs.jsonl"
            output, summary = root / "timings.json", root / "summary.md"
            event.write_text(json.dumps({"workflow_run": self.run}))
            jobs.write_text(json.dumps(self.job) + "\n")
            summary.write_text("Existing report\n")
            result = subprocess.run(
                [
                    sys.executable,
                    str(script),
                    "--event",
                    str(event),
                    "--jobs",
                    str(jobs),
                    "--output",
                    str(output),
                    "--summary",
                    str(summary),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(output.read_text())["run_id"], 7)
            self.assertTrue(summary.read_text().startswith("Existing report\n"))


if __name__ == "__main__":
    unittest.main()

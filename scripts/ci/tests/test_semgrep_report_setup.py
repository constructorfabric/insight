"""The report job installs uv when an earlier step has already failed."""

from __future__ import annotations

import unittest
from pathlib import Path

SECURITY = Path(__file__).resolve().parents[3] / ".github" / "workflows" / "security.yml"


def _between(text: str, start: str, end: str) -> str:
    try:
        return text.split(start, 1)[1].split(end, 1)[0]
    except IndexError as exc:
        raise AssertionError(f"missing section {start!r} .. {end!r}") from exc


class SemgrepReportSetupTests(unittest.TestCase):
    def test_report_installs_uv_after_an_earlier_step_fails(self) -> None:
        workflow = SECURITY.read_text(encoding="utf-8")
        report = _between(workflow, "\n  report:\n", "\n  secrets-history:\n")
        uv_step = _between(report, "\n      - name: Install uv\n", "\n      - name: Install Helm\n")
        semgrep_step = _between(
            report,
            "\n      - name: Semgrep scan (full tree",
            "\n      - name: Summarize findings in the job summary\n",
        )

        self.assertIn("uses: astral-sh/setup-uv@", uv_step)
        self.assertIn("if: ${{ !cancelled() }}", uv_step)
        self.assertNotIn("continue-on-error", uv_step)
        self.assertLess(report.index("\n      - name: Install uv\n"), report.index("\n      - name: Semgrep scan (full tree"))
        self.assertIn("if: ${{ !cancelled() }}", semgrep_step)
        self.assertIn("set -euo pipefail", semgrep_step)
        self.assertIn('uv venv --python 3.12 "$RUNNER_TEMP/semgrep"', semgrep_step)
        self.assertNotIn("continue-on-error", semgrep_step)
        self.assertNotIn("|| true", semgrep_step)


if __name__ == "__main__":
    unittest.main()

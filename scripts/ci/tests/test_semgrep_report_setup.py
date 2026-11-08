"""The report job's Semgrep step runs after earlier steps fail. Its uv install must too."""

from __future__ import annotations

import re
import unittest
from pathlib import Path

SECURITY = Path(__file__).resolve().parents[3] / ".github" / "workflows" / "security.yml"

REPORT_SEMGREP = "Semgrep scan (full tree — owns the SARIF upload, never blocks)"
BLOCKING_SEMGREP = "Semgrep scan — findings added by this pull request (BLOCKING)"


def runs_after_an_earlier_failure(condition: str, event_name: str) -> bool:
    """Whether a step still runs when a previous step failed and the job was not cancelled.

    An empty condition is GitHub's default `success()`, which is false after a failure.
    """
    expr = condition.strip()
    if expr.startswith("${{") and expr.endswith("}}"):
        expr = expr[3:-2].strip()
    if expr == "!cancelled()":
        return True
    event = re.fullmatch(r"!cancelled\(\) && github\.event_name == '([^']+)'", expr)
    if event:
        return event.group(1) == event_name
    return False


def _job_body(workflow: str, job_id: str) -> str:
    lines = workflow.splitlines()
    try:
        jobs_at = lines.index("jobs:")
    except ValueError as exc:
        raise AssertionError("security.yml has no jobs block") from exc
    start = None
    for index, line in enumerate(lines):
        if index > jobs_at and line == f"  {job_id}:":
            start = index
            break
    if start is None:
        raise AssertionError(f"job {job_id!r} is missing")
    end = len(lines)
    for index in range(start + 1, len(lines)):
        if re.match(r"^  [A-Za-z0-9_-]+:\s*$", lines[index]):
            end = index
            break
    return "\n".join(lines[start + 1 : end])


def _steps(workflow: str, job_id: str) -> list[dict[str, str]]:
    body = _job_body(workflow, job_id).splitlines()
    try:
        steps_at = body.index("    steps:")
    except ValueError as exc:
        raise AssertionError(f"job {job_id!r} has no steps") from exc
    steps: list[list[str]] = []
    for line in body[steps_at + 1 :]:
        if line.startswith("      - "):
            steps.append([line])
        elif steps:
            steps[-1].append(line)
    parsed: list[dict[str, str]] = []
    for chunk in steps:
        text = "\n".join(chunk)
        parsed.append(
            {
                "name": _field(text, "name"),
                "uses": _field(text, "uses").split()[0] if _field(text, "uses") else "",
                "if": _field(text, "if"),
                "continue_on_error": _field(text, "continue-on-error"),
                "run": _run(chunk),
                "text": text,
            }
        )
    return parsed


def _field(text: str, key: str) -> str:
    match = re.search(rf"(?m)^        {re.escape(key)}: (.+)$", text)
    if match is None:
        match = re.search(rf"(?m)^      - {re.escape(key)}: (.+)$", text)
    return match.group(1).strip() if match else ""


def _run(chunk: list[str]) -> str:
    for index, line in enumerate(chunk):
        folded = re.match(r"^        run: \|(.*)$", line)
        if folded:
            script: list[str] = []
            for follow in chunk[index + 1 :]:
                if follow.startswith("          "):
                    script.append(follow[10:])
                elif follow.strip() == "":
                    script.append("")
                else:
                    break
            return "\n".join(script).strip("\n")
        inline = re.match(r"^        run: (.*)$", line)
        if inline:
            return inline.group(1)
    return ""


def _named(steps: list[dict[str, str]], name: str) -> dict[str, str]:
    matches = [step for step in steps if step["name"] == name]
    if len(matches) != 1:
        found = [step["name"] for step in steps if step["name"]]
        raise AssertionError(f"expected one step {name!r}, found {matches!r} among {found}")
    return matches[0]


class SemgrepReportSetupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = SECURITY.read_text(encoding="utf-8")

    def test_a_missing_condition_skips_the_step_after_an_earlier_failure(self) -> None:
        self.assertFalse(runs_after_an_earlier_failure("", "schedule"))
        self.assertFalse(runs_after_an_earlier_failure("${{ success() }}", "schedule"))

    def test_report_installs_uv_when_an_earlier_step_failed(self) -> None:
        steps = _steps(self.workflow, "report")
        uv = _named(steps, "Install uv")
        semgrep = _named(steps, REPORT_SEMGREP)

        self.assertIn("astral-sh/setup-uv@", uv["uses"])
        self.assertLess(steps.index(uv), steps.index(semgrep))
        self.assertIn('version: "0.12.17"', uv["text"])
        self.assertEqual(uv["if"], "${{ !cancelled() }}")
        for event in ("push", "schedule", "workflow_dispatch"):
            self.assertTrue(
                runs_after_an_earlier_failure(uv["if"], event),
                f"report uv install must still run on {event} after an earlier step fails",
            )
        self.assertEqual(uv["continue_on_error"], "")

    def test_report_semgrep_fails_the_job_when_the_command_fails(self) -> None:
        semgrep = _named(_steps(self.workflow, "report"), REPORT_SEMGREP)

        self.assertEqual(semgrep["if"], "${{ !cancelled() }}")
        self.assertEqual(semgrep["continue_on_error"], "")
        self.assertNotIn("|| true", semgrep["run"])
        self.assertIn("set -euo pipefail", semgrep["run"])
        self.assertIn('uv venv --python 3.12 "$RUNNER_TEMP/semgrep"', semgrep["run"])
        self.assertIn("--require-hashes -r .github/semgrep-requirements.txt", semgrep["run"])
        self.assertNotIn("--error", semgrep["run"])
        self.assertIn("--exclude-rule generic.secrets.security.detected-generic-secret", semgrep["run"])

    def test_blocking_scan_installs_uv_when_an_earlier_step_failed(self) -> None:
        steps = _steps(self.workflow, "scan")
        uv = _named(steps, "Install uv")
        semgrep = _named(steps, BLOCKING_SEMGREP)

        self.assertIn("astral-sh/setup-uv@", uv["uses"])
        self.assertLess(steps.index(uv), steps.index(semgrep))
        self.assertEqual(uv["if"], "${{ !cancelled() && github.event_name == 'pull_request' }}")
        self.assertTrue(runs_after_an_earlier_failure(uv["if"], "pull_request"))
        self.assertFalse(runs_after_an_earlier_failure(uv["if"], "schedule"))
        self.assertEqual(semgrep["if"], "${{ !cancelled() && github.event_name == 'pull_request' }}")
        self.assertEqual(semgrep["continue_on_error"], "")
        self.assertIn("set -euo pipefail", semgrep["run"])
        self.assertIn("--error", semgrep["run"])
        self.assertIn("--require-hashes -r .github/semgrep-requirements.txt", semgrep["run"])


if __name__ == "__main__":
    unittest.main()

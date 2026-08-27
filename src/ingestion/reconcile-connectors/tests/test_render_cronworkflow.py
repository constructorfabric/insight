"""Contract of the rendered CronWorkflow's `schedules:` array.

`descriptor.schedule` may be one cron or a list of them. `parse_descriptor.py`
prints a list newline-separated and `render_cronworkflow.py` turns every line
into one entry of the Argo >=3.5 `schedules:` array.

Run: python3 -m unittest discover -s src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

RECONCILE_DIR = Path(__file__).resolve().parents[1]
RENDERER = RECONCILE_DIR / "python" / "render_cronworkflow.py"
PARSER = RECONCILE_DIR / "python" / "parse_descriptor.py"
TEMPLATE = RECONCILE_DIR / "templates" / "cron-workflow.yaml.tpl"
CLAUDE_TEAM_DESCRIPTOR = RECONCILE_DIR.parent / "connectors" / "ai" / "claude-team" / "descriptor.yaml"

# parse_descriptor.py's PyYAML-free fallback reader parses scalars only, so the
# list leg of these cases needs the real parser.
HAS_PYYAML = importlib.util.find_spec("yaml") is not None

# WORKAROUND: PYTHONIOENCODING pins the child's stdout codec — the template
# carries non-ASCII comments that a non-UTF-8 console default cannot encode.
CHILD_ENV = {
    **os.environ,
    "PYTHONIOENCODING": "utf-8",
    "INSIGHT_NAMESPACE": "insight",
    "ARGO_SERVICE_ACCOUNT": "insight-reconcile",
    "ARGO_INSTANCE_ID": "",
}

DESCRIPTOR_CASES = (
    ("a scalar schedule", "schedule: '0 4 * * *'\n", ["0 4 * * *"]),
    (
        "a list schedule",
        "schedule:\n  - '50 23 * * *'\n  - '0 9,15,21 28-31 * *'\n",
        ["50 23 * * *", "0 9,15,21 28-31 * *"],
    ),
)


def _run(argv: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(argv, env=CHILD_ENV, capture_output=True, text=True, encoding="utf-8", check=False)


def _render(schedule: str) -> subprocess.CompletedProcess[str]:
    return _run(
        [
            sys.executable,
            str(RENDERER),
            "--connector",
            "example-connector",
            "--connection-name",
            "example-connector-main-example-conn",
            "--schedule",
            schedule,
            "--tenant",
            "example",
            "--insight-source-id",
            "main",
            "--tpl",
            str(TEMPLATE),
        ]
    )


def _rendered_schedules(document: str) -> list[str]:
    """The cron expressions of the rendered `schedules:` array, in order."""
    prefix = "  schedules: "
    for line in document.splitlines():
        if line.startswith(prefix):
            return json.loads(line[len(prefix) :])
    raise AssertionError("the rendered document carries no `schedules:` key")


class RendererTests(unittest.TestCase):
    def test_a_single_cron_renders_exactly_one_entry(self) -> None:
        result = _render("0 4 * * *")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(_rendered_schedules(result.stdout), ["0 4 * * *"])

    def test_every_cron_line_renders_its_own_entry_in_order(self) -> None:
        result = _render("50 23 * * *\n0 9,15,21 28-31 * *")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(_rendered_schedules(result.stdout), ["50 23 * * *", "0 9,15,21 28-31 * *"])

    def test_a_schedule_carrying_no_cron_expression_is_rejected(self) -> None:
        result = _render("\n   \n")

        self.assertEqual(result.returncode, 2, result.stdout)


@unittest.skipUnless(HAS_PYYAML, "PyYAML unavailable")
class DescriptorScheduleTests(unittest.TestCase):
    """The descriptor -> parse_descriptor -> renderer path reconcile takes."""

    def test_the_whole_rendered_document_stays_valid_yaml(self) -> None:
        import yaml

        rendered = _render("50 23 * * *\n0 9,15,21 28-31 * *")

        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        document = yaml.safe_load(rendered.stdout)
        self.assertEqual(document["kind"], "CronWorkflow")
        self.assertEqual(document["spec"]["schedules"], ["50 23 * * *", "0 9,15,21 28-31 * *"])

    def _schedule_of(self, descriptor: Path) -> str:
        result = _run([sys.executable, str(PARSER), "--descriptor", str(descriptor), "--field", "schedule"])

        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def test_a_descriptor_schedule_reaches_the_rendered_array(self) -> None:
        for label, body, expected in DESCRIPTOR_CASES:
            with self.subTest(descriptor=label), tempfile.TemporaryDirectory() as tmp:
                descriptor = Path(tmp) / "descriptor.yaml"
                descriptor.write_text(f'name: example-connector\nversion: "1.0.0"\n{body}', encoding="utf-8")

                rendered = _render(self._schedule_of(descriptor))

                self.assertEqual(rendered.returncode, 0, rendered.stderr)
                self.assertEqual(_rendered_schedules(rendered.stdout), expected, f"should render: {label}")

    def test_claude_team_reads_more_than_once_a_day_at_month_end(self) -> None:
        rendered = _render(self._schedule_of(CLAUDE_TEAM_DESCRIPTOR))

        self.assertEqual(rendered.returncode, 0, rendered.stderr)
        self.assertEqual(
            _rendered_schedules(rendered.stdout),
            ["50 23 * * *", "0 9,15,21 28-31 * *"],
            "claude-team must keep the extra month-end readings its billing month depends on",
        )


if __name__ == "__main__":
    unittest.main()

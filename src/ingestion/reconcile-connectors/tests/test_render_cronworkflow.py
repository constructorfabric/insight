"""Contract of the rendered CronWorkflow's `schedules:` array.

`descriptor.schedule` may be one cron or a list of them. `parse_descriptor.py`
prints a list newline-separated and `render_cronworkflow.py` turns every line
into one entry of the Argo >=3.5 `schedules:` array.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

RECONCILE_DIR = Path(__file__).resolve().parents[1]
RENDERER = RECONCILE_DIR / "python" / "render_cronworkflow.py"
PARSER = RECONCILE_DIR / "python" / "parse_descriptor.py"
TEMPLATE = RECONCILE_DIR / "templates" / "cron-workflow.yaml.tpl"
ARGO = RECONCILE_DIR / "lib" / "argo.sh"
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
            "--cron-name",
            "example-connector-example-sync",
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


def _rendered_name(document: str) -> str:
    prefix = "  name: "
    for line in document.splitlines():
        if line.startswith(prefix):
            return line[len(prefix) :]
    raise AssertionError("the rendered document carries no `metadata.name`")


class TestRenderer:
    def test_a_single_cron_renders_exactly_one_entry(self) -> None:
        result = _render("0 4 * * *")

        assert result.returncode == 0, result.stderr
        assert _rendered_schedules(result.stdout) == ["0 4 * * *"]
        assert _rendered_name(result.stdout) == "example-connector-example-sync"

    def test_every_cron_line_renders_its_own_entry_in_order(self) -> None:
        result = _render("50 23 * * *\n0 9,15,21 28-31 * *")

        assert result.returncode == 0, result.stderr
        assert _rendered_schedules(result.stdout) == [
            "50 23 * * *",
            "0 9,15,21 28-31 * *",
        ]

    def test_a_schedule_carrying_no_cron_expression_is_rejected(self) -> None:
        result = _render("\n   \n")

        assert result.returncode == 2, result.stderr
        expected = "render_cronworkflow: --schedule carries no cron expression"
        assert expected in result.stderr


class TestArgoCronWorkflow:
    @pytest.fixture(autouse=True)
    def _kubectl_on_path(self, tmp_path: Path) -> None:
        """A fake `kubectl` earlier on PATH than the real one, and a log of what
        the script asked it to do."""
        self.tmp_path = tmp_path
        self.kubectl_log = tmp_path / "kubectl.log"
        kubectl = tmp_path / "kubectl"
        kubectl.write_text(
            """#!/usr/bin/env bash
printf '%s\\n' \"$*\" >> \"${FAKE_KUBECTL_LOG}\"
if [[ -n \"${FAKE_KUBECTL_FAIL_DELETE_NAME:-}\" \
  && \"$*\" == *\"delete cronworkflow.argoproj.io/${FAKE_KUBECTL_FAIL_DELETE_NAME}\"* ]]; then
  printf 'delete rejected\\n' >&2
  exit 1
fi
printf 'ok\\n'
""",
            encoding="utf-8",
        )
        kubectl.chmod(0o755)
        self.env = {
            **CHILD_ENV,
            "ARGO_SCRIPT": str(ARGO),
            "FAKE_KUBECTL_LOG": str(self.kubectl_log),
            "PATH": f"{self.tmp_path}{os.pathsep}{os.environ['PATH']}",
        }

    def _run_argo(self, script: str, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", "-c", script, "argo-test", *args],
            env={**self.env, **env},
            capture_output=True,
            text=True,
            encoding="utf-8",
            check=False,
        )

    def _apply(self, connector: str, tenant: str, **env: str) -> subprocess.CompletedProcess[str]:
        return self._run_argo(
            """source \"$ARGO_SCRIPT\"
argo_render_cronworkflow() {
  printf 'apiVersion: v1\\n'
}
argo_apply_cronworkflow \"$@\"
""",
            connector,
            "example-connection",
            "0 4 * * *",
            tenant,
            "example-source",
            "",
            "",
            **env,
        )

    def _delete(self, connector: str, tenant: str) -> subprocess.CompletedProcess[str]:
        return self._run_argo('source "$ARGO_SCRIPT"\nargo_delete_cronworkflow "$@"', connector, tenant)

    def _kubectl_calls(self) -> list[str]:
        if not self.kubectl_log.exists():
            return []
        return self.kubectl_log.read_text(encoding="utf-8").splitlines()

    def test_a_52_character_name_is_accepted(self) -> None:
        connector = "c" * 38
        result = self._run_argo('source "$ARGO_SCRIPT"\nargo_cron_workflow_name "$@"', connector, "tenantid")

        assert result.returncode == 0, result.stderr
        assert result.stdout.strip() == f"{connector}-tenantid-sync"
        assert len(result.stdout.strip()) == 52

    def test_a_53_character_name_is_rejected_before_kubectl(self) -> None:
        result = self._run_argo(
            'source "$ARGO_SCRIPT"\nargo_apply_cronworkflow "$@"',
            "c" * 39,
            "example-connection",
            "0 4 * * *",
            "tenantid",
            "example-source",
            "",
            "",
        )

        assert result.returncode == 1, result.stderr
        assert "Argo accepts at most 52" in result.stderr
        assert self._kubectl_calls() == []

    def test_apply_removes_the_legacy_full_tenant_name(self) -> None:
        tenant = "tenant-identifier"
        result = self._apply("example-connector", tenant)

        assert result.returncode == 0, result.stderr
        assert self._kubectl_calls() == [
                "apply -f -",
                f"-n insight delete cronworkflow.argoproj.io/example-connector-{tenant}-sync --ignore-not-found",
            ]

    def test_apply_returns_2_when_legacy_cleanup_fails(self) -> None:
        tenant = "tenant-identifier"
        legacy_name = f"example-connector-{tenant}-sync"
        result = self._apply("example-connector", tenant, FAKE_KUBECTL_FAIL_DELETE_NAME=legacy_name)

        assert result.returncode == 2, result.stderr
        assert f"{legacy_name}: kubectl delete failed: delete rejected" in result.stderr

    def test_apply_for_a_short_tenant_does_not_delete_the_same_name_twice(self) -> None:
        result = self._apply("example-connector", "short")

        assert result.returncode == 0, result.stderr
        assert self._kubectl_calls() == ["apply -f -"]

    def test_delete_removes_full_tenant_and_bounded_names(self) -> None:
        tenant = "tenant-identifier"
        result = self._delete("example-connector", tenant)

        assert result.returncode == 0, result.stderr
        assert self._kubectl_calls() == [
                f"-n insight delete cronworkflow.argoproj.io/example-connector-{tenant}-sync --ignore-not-found",
                "-n insight delete cronworkflow.argoproj.io/example-connector-tenant-i-sync --ignore-not-found",
            ]


@pytest.mark.skipif(not HAS_PYYAML, reason="PyYAML unavailable")
class TestDescriptorSchedule:
    """The descriptor -> parse_descriptor -> renderer path reconcile takes."""

    def test_the_whole_rendered_document_stays_valid_yaml(self) -> None:
        import yaml

        rendered = _render("50 23 * * *\n0 9,15,21 28-31 * *")

        assert rendered.returncode == 0, rendered.stderr
        document = yaml.safe_load(rendered.stdout)
        assert document["kind"] == "CronWorkflow"
        assert document["spec"]["schedules"] == ["50 23 * * *", "0 9,15,21 28-31 * *"]

    def _schedule_of(self, descriptor: Path) -> str:
        result = _run([sys.executable, str(PARSER), "--descriptor", str(descriptor), "--field", "schedule"])

        assert result.returncode == 0, result.stderr
        return result.stdout

    @pytest.mark.parametrize(("label", "body", "expected"), DESCRIPTOR_CASES)
    def test_a_descriptor_schedule_reaches_the_rendered_array(
        self, tmp_path: Path, label: str, body: str, expected: list[str]
    ) -> None:
        descriptor = tmp_path / "descriptor.yaml"
        descriptor.write_text(
            f'name: example-connector\nversion: "1.0.0"\n{body}', encoding="utf-8"
        )

        rendered = _render(self._schedule_of(descriptor))

        assert rendered.returncode == 0, rendered.stderr
        assert _rendered_schedules(rendered.stdout) == expected, (
            f"should render: {label}"
        )

    def test_claude_team_reads_more_than_once_a_day_at_month_end(self) -> None:
        rendered = _render(self._schedule_of(CLAUDE_TEAM_DESCRIPTOR))

        assert rendered.returncode == 0, rendered.stderr
        assert _rendered_schedules(rendered.stdout) == [
            "50 23 * * *",
            "0 9,15,21 28-31 * *",
        ], (
            "claude-team must keep the extra month-end readings its billing month depends on"
        )

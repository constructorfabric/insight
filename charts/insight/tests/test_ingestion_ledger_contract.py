"""Helm render-contract for the ingestion run-ledger writers.

These templates record what an ingestion run did, and their correctness rests on
two facts a diff does not show, both measured against a live Argo 3.6.10 before
this file existed:

* A DAG's phase comes from its TARGETS, and a target is any task nothing depends
  on. A recorder that runs after the work it records is therefore the only
  target, and a recorder that succeeds after a failure **erases that failure** —
  a failed sync reported a green run, and dbt went on to rebuild from stale
  bronze. The fix is `dag.target` naming the real work alongside the recorder,
  and that is what these tests pin.

* The inverse must hold too: a recorder that cannot run — an image it cannot
  pull, an eviction, its deadline — must not fail the run it only observes. A
  `depends` DAG rejects `continueOn`, so every write goes through a `steps`
  wrapper that carries it.

Pure `helm template` plus assertions — no cluster, no images.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]
RELEASE = "insight"

#: The smallest value set that renders the chart. Nothing here is about the
#: ledger; these are the fields the umbrella refuses to render without.
REQUIRED_VALUES = {
    "clickhouse.host": "ch.example",
    "clickhouse.database": "insight",
    "clickhouse.username": "insight",
    "redis.host": "redis.example",
    "mariadb.host": "maria.example",
    "mariadb.database": "insight",
    "mariadb.username": "insight",
    "authenticator.oidc.issuerUrl": "https://idp.example/realms/x",
    "authenticator.oidc.redirectUri": "https://app.example/callback",
    "authenticator.oidc.sourceType": "keycloak",
    "ingestion.toolboxImage": "toolbox:test",
    "ingestion.reconcile.tenantId": "default",
    "global.tenantDefaultId": "00000000-0000-0000-0000-000000000001",
    "redpanda.brokers": "redpanda.example:9092",
}

LEDGER_TEMPLATE = "ledger-write"
TOLERATED_ENTRY = "write-tolerated"


def render(template: str) -> dict:
    settings = [f"--set={key}={value}" for key, value in REQUIRED_VALUES.items()]
    result = subprocess.run(
        [
            "helm",
            "template",
            RELEASE,
            str(CHART),
            *settings,
            f"--show-only=templates/ingestion/{template}.yaml",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    # `check=True` would raise with the whole argv and none of the reason —
    # a missing value or an unvendored subchart reads as an opaque exit 1.
    if result.returncode != 0:
        raise AssertionError(f"helm template {template} failed: {result.stderr.strip()}")

    return yaml.safe_load(result.stdout)


def templates_by_name(document: dict) -> dict[str, dict]:
    return {template["name"]: template for template in document["spec"]["templates"]}


def dag_targets(dag: dict) -> set[str]:
    """What the phase is assessed over: the declared targets, or every leaf."""
    declared = dag.get("target")
    if declared:
        return set(declared.split())
    depended_on = {
        dependency
        for task in dag["tasks"]
        for dependency in task.get("depends", "").replace("||", " ").replace("&&", " ").split()
        if "." not in dependency or dependency.split(".")[0]
    }
    named = {part.split(".")[0] for part in depended_on}
    return {task["name"] for task in dag["tasks"]} - named


def ledger_task_names(dag: dict) -> set[str]:
    return {
        task["name"]
        for task in dag["tasks"]
        if task.get("templateRef", {}).get("name") == LEDGER_TEMPLATE
    }


@pytest.fixture(scope="module")
def sync_dag() -> dict:
    return templates_by_name(render("airbyte-sync"))["sync"]["dag"]


@pytest.fixture(scope="module")
def pipeline() -> dict:
    return render("ingestion-pipeline")


class TestRecordingCannotEraseAFailure:
    """A recorder must never be the only thing a DAG's phase is read from."""

    def test_the_sync_dag_is_assessed_over_the_poll_and_not_only_the_recorder(
        self, sync_dag: dict
    ) -> None:
        targets = dag_targets(sync_dag)

        assert "poll" in targets, (
            "the poll's outcome must reach the phase assessment; with only the "
            f"recorder as a target a failed sync reports a green run. targets={targets}"
        )

    def test_the_pipeline_dag_is_assessed_over_the_real_work(self, pipeline: dict) -> None:
        targets = dag_targets(templates_by_name(pipeline)["pipeline"]["dag"])

        assert {"sync", "transform"} <= targets, (
            f"a failed sync or transform must reach the phase assessment. targets={targets}"
        )

    @pytest.mark.parametrize("template_name", ["airbyte-sync", "ingestion-pipeline"])
    def test_no_dag_is_assessed_over_recorders_alone(self, template_name: str) -> None:
        for name, template in templates_by_name(render(template_name)).items():
            dag = template.get("dag")
            if not dag:
                continue
            targets = dag_targets(dag)
            recorders = ledger_task_names(dag)
            if not recorders:
                continue
            assert targets - recorders, (
                f"{template_name}/{name}: every target is a ledger write, so any "
                f"failure below them is erased. targets={targets}"
            )


class TestRecordingCannotFailTheRun:
    """The inverse: a recorder that cannot run must not redden a green run."""

    def test_the_ledger_entry_point_tolerates_its_own_failure(self) -> None:
        wrapper = templates_by_name(render(LEDGER_TEMPLATE))[TOLERATED_ENTRY]

        step = wrapper["steps"][0][0]
        assert step.get("continueOn") == {"failed": True, "error": True}, (
            "a write that cannot run must not reach the run it records; "
            "`continueOn` is legal here and rejected inside a `depends` DAG"
        )

    def test_the_ledger_template_offers_the_tolerant_entry_point_by_default(self) -> None:
        document = render(LEDGER_TEMPLATE)

        assert document["spec"]["entrypoint"] == TOLERATED_ENTRY

    @pytest.mark.parametrize("template_name", ["airbyte-sync", "ingestion-pipeline"])
    def test_every_caller_writes_through_the_tolerant_entry_point(
        self, template_name: str
    ) -> None:
        document = render(template_name)

        for name, template in templates_by_name(document).items():
            entries = [*template.get("dag", {}).get("tasks", [])]
            for group in template.get("steps", []):
                entries.extend(group)
            for entry in entries:
                ref = entry.get("templateRef", {})
                if ref.get("name") != LEDGER_TEMPLATE:
                    continue
                assert ref.get("template") == TOLERATED_ENTRY, (
                    f"{template_name}/{name}/{entry['name']} writes through "
                    f"{ref.get('template')!r}, which does not tolerate its own failure"
                )


class TestRecordingIsOptional:
    """A submitter that predates the ledger must run, and record nothing."""

    def test_the_identity_is_a_template_input_not_a_workflow_reference(
        self, pipeline: dict
    ) -> None:
        # The manual submitter reaches this template by `templateRef`, which
        # merges no `spec.arguments` — a workflow-scope reference is
        # unresolvable there and the run errors before it starts.
        dag = templates_by_name(pipeline)["pipeline"]
        rendered = yaml.safe_dump(dag)

        assert "workflow.parameters.connector" not in rendered, (
            "the pipeline DAG must read its identity from inputs, so the "
            "templateRef submitter can resolve it"
        )
        declared = {parameter["name"] for parameter in dag["inputs"]["parameters"]}
        assert {"connector", "tenant_id"} <= declared

    def test_every_recorder_is_guarded_on_a_connector_being_named(
        self, pipeline: dict
    ) -> None:
        for name, template in templates_by_name(pipeline).items():
            dag = template.get("dag")
            if not dag:
                continue
            for task in dag["tasks"]:
                if task["name"] not in ledger_task_names(dag):
                    continue
                assert "!= ''" in task.get("when", ""), (
                    f"{name}/{task['name']} records unattributable rows for a "
                    "caller that names no connector"
                )

    def test_the_exit_handler_is_guarded_too(self, pipeline: dict) -> None:
        handler = templates_by_name(pipeline)[pipeline["spec"]["onExit"]]

        step = handler["steps"][0][0]
        assert "!= ''" in step.get("when", "")

"""Helm render-contract for the ingestion run-ledger writers.

These templates record what an ingestion run did, and their correctness rests on
two facts a diff does not show, both following from documented Argo semantics:

* A DAG's phase comes from its TARGETS, and a target is any task nothing depends
  on. A recorder that runs after the work it records is therefore the only
  target, so a recorder succeeding after a failure would **erase that failure**:
  the DAG would report Succeeded, a failed sync would read as a green run, and
  dbt would rebuild from stale bronze. `dag.target` names the real work
  alongside the recorder, and that is what these tests pin.

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


def render(template: str, extra: dict[str, str] | None = None) -> dict:
    settings = [f"--set={key}={value}" for key, value in {**REQUIRED_VALUES, **(extra or {})}.items()]
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


#: Every template that submits the pipeline. A submitter reaching it by a
#: `templateRef` step pulls in that one template and leaves `spec.onExit`
#: behind, so its run records no outcome — see TestEverySubmitterCarriesTheHandler.
SUBMITTERS = [
    "src/ingestion/workflows/onetime/sync.yaml.tpl",
    "src/ingestion/reconcile-connectors/templates/sync-trigger.yaml.tpl",
    # The scheduled path — the one that produces almost every run. A
    # CronWorkflow nests its spec, which is why it was invisible to assertions
    # that read `spec` directly.
    "src/ingestion/reconcile-connectors/templates/cron-workflow.yaml.tpl",
]

REPO = CHART.parents[1]


class TestTheSweepIsToldItsPolicy:
    """A value the chart declares must reach the loop, not a default beside it."""

    HORIZON = "ingestion.reconcile.claimHorizonSeconds"

    def test_the_configured_horizon_reaches_the_container(self) -> None:
        # A `default` in the template made a misplaced value render as the
        # default, so the render "passed" while the setting was never read.
        # Only a non-default value proves the wiring.
        document = render("reconcile-cron", {self.HORIZON: "4242"})
        env = self._sweep_env(document)

        assert env == "4242", (
            "the configured horizon must reach the sweep; a default here hides "
            f"a value declared in the wrong place. got: {env!r}"
        )

    def test_the_chart_declares_a_horizon_for_a_default_install(self) -> None:
        document = render("reconcile-cron")

        assert self._sweep_env(document), "values.yaml must carry it, or every install fails to render"

    @staticmethod
    def _sweep_env(document: dict) -> str | None:
        spec = document["spec"]["workflowSpec"]["templates"][0]["container"]
        for entry in spec.get("env", []):
            if entry["name"] == "SWEEP_CLAIM_HORIZON_SECONDS":
                return entry.get("value")
        return None


class TestEverySubmitterCarriesTheHandler:
    """The terminal row is written by `spec.onExit`, which one reference drops."""

    @staticmethod
    def submitted_spec(submitter: str) -> dict:
        """The workflow spec, wherever this kind of resource keeps it."""
        # envsubst placeholders are plain scalars, so these parse as YAML as-is.
        document = yaml.safe_load((REPO / submitter).read_text())
        spec = document["spec"]
        return spec.get("workflowSpec", spec)

    @pytest.mark.parametrize("submitter", SUBMITTERS)
    def test_the_submitter_references_the_pipeline_at_spec_level(self, submitter: str) -> None:
        spec = self.submitted_spec(submitter)

        assert spec.get("workflowTemplateRef", {}).get("name") == "ingestion-pipeline", (
            f"{submitter} must reach the pipeline by workflowTemplateRef; a "
            "templateRef step carries no spec.onExit, so the run records no outcome"
        )
        assert "templates" not in spec, (
            f"{submitter} wraps the pipeline in a local template, which reinstates "
            "the spec that has no exit handler"
        )

    @pytest.mark.parametrize("submitter", SUBMITTERS)
    def test_the_submitter_supplies_every_input_without_a_default(
        self, submitter: str, pipeline: dict
    ) -> None:
        # An input with no default and no argument is unresolvable, and the run
        # dies before its first step — so it records nothing at all, which is
        # the failure this whole file exists to prevent.
        template = templates_by_name(pipeline)["pipeline"]
        required = {
            parameter["name"]
            for parameter in template["inputs"]["parameters"]
            if "default" not in parameter
        }
        supplied = {
            parameter["name"]
            for parameter in self.submitted_spec(submitter).get("arguments", {}).get("parameters", [])
        }

        assert required <= supplied, (
            f"{submitter} leaves {sorted(required - supplied)} unresolved; the "
            "workflow fails input resolution before it starts"
        )

    @pytest.mark.parametrize("submitter", SUBMITTERS)
    def test_the_submitter_names_the_ledger_identity(self, submitter: str) -> None:
        supplied = {
            parameter["name"]
            for parameter in self.submitted_spec(submitter).get("arguments", {}).get("parameters", [])
        }

        assert {"connector", "tenant_id"} <= supplied, (
            f"{submitter} submits a run the ledger cannot attribute"
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

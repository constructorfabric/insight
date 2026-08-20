"""Helm render-contract for the ingestion pipeline's identity-seed step.

Two production incidents motivate every assertion here, both invisible to a
`helm lint` and to any test that does not compare the umbrella's two version
sources against each other:

  * the step's image tag came from the UMBRELLA's `.Chart.AppVersion`, while
    the identity Deployment resolves the identity SUBCHART's — the release
    pipeline stamps those independently, so the pipeline pulled a tag that was
    never built (`ImagePullBackOff`, every connector's gold build blocked);
  * the gold dbt step excludes what the staging step just built, which empties
    the selection entirely for a roster-only connector (its only models ARE
    the ones excluded), and an empty selection is what the #2362 guard exists
    to reject — so a pipeline that had nothing left to do failed instead.

Both are contract, not plumbing: nothing else in CI renders the umbrella and
compares these values, and the functional lane does not run a connector
pipeline end to end.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

HERE = Path(__file__).resolve()
REPO_ROOT = HERE.parents[6]
UMBRELLA = REPO_ROOT / "charts" / "insight"
VALUES = REPO_ROOT / "deploy" / "gitops" / "environments" / "test-stand" / "values.yaml"


def _render(*extra: str) -> list[dict]:
    proc = subprocess.run(  # noqa: S603 — test-controlled argv
        [  # noqa: S607
            "helm",
            "template",
            "contract-test",
            str(UMBRELLA),
            "--values",
            str(VALUES),
            *extra,
        ],
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    return [d for d in yaml.safe_load_all(proc.stdout) if isinstance(d, dict)]


@pytest.fixture(scope="module")
def docs() -> list[dict]:
    subprocess.run(  # noqa: S603, S607 — refresh the vendored subcharts
        ["helm", "dependency", "update", str(UMBRELLA)],
        capture_output=True,
        text=True,
        timeout=300,
        check=True,
    )
    return _render()


def _named(docs: list[dict], kind: str, suffix: str) -> dict:
    matches = [
        d for d in docs if d.get("kind") == kind and d["metadata"]["name"].endswith(suffix)
    ]
    assert len(matches) == 1, f"expected one {kind} ending {suffix}, got {len(matches)}"
    return matches[0]


def _seed_step_image(docs: list[dict]) -> str:
    template = _named(docs, "WorkflowTemplate", "identity-seed-run")["spec"]["templates"][0]
    (param,) = [p for p in template["inputs"]["parameters"] if p["name"] == "identity_image"]
    return param["default"]


def _dbt_run_script(docs: list[dict]) -> str:
    template = _named(docs, "WorkflowTemplate", "dbt-run")["spec"]["templates"][0]
    return template["script"]["source"]


def test_the_seed_step_runs_the_same_image_as_the_identity_deployment(docs) -> None:
    """The one assertion the ImagePullBackOff needed.

    The step must not derive a tag of its own: the umbrella and the identity
    subchart carry independently-stamped appVersions, so any expression that
    reaches for the wrong one produces a tag no build ever pushed.
    """
    deployment = _named(docs, "Deployment", "-identity-resolution")
    (container,) = deployment["spec"]["template"]["spec"]["containers"]
    assert _seed_step_image(docs) == container["image"]


def test_an_explicit_image_tag_still_wins(docs) -> None:
    """An operator pinning `identityResolution.image.tag` pins BOTH."""
    pinned = _render("--set", "identityResolution.image.tag=0.0.0-pinned")
    deployment = _named(pinned, "Deployment", "-identity-resolution")
    (container,) = deployment["spec"]["template"]["spec"]["containers"]
    assert _seed_step_image(pinned) == container["image"]
    assert container["image"].endswith(":0.0.0-pinned")


def test_the_gold_step_excludes_what_the_staging_step_built(docs) -> None:
    """The dedup this contract keeps: a `tag:X+` graph walk would otherwise
    rebuild the staging models the pipeline just built."""
    template = _named(docs, "WorkflowTemplate", "ingestion-pipeline")["spec"]["templates"]
    (transform,) = [t for t in template if t["name"] == "transform"]
    (gold,) = [t for t in transform["dag"]["tasks"] if t["name"] == "transform-legacy"]
    excludes = [p["value"] for p in gold["arguments"]["parameters"] if p["name"] == "dbt_exclude"]
    assert excludes == ["tag:{{inputs.parameters.data_source}} identity_inputs"]


def test_the_selector_guard_resolves_the_selection_without_the_exclusion(docs) -> None:
    """#2362's guard must judge the SELECTOR, not what survives the exclusion.

    Resolving `--select` together with `--exclude` conflates two different
    failures: a misspelled selector (nothing to run, and nobody meant that)
    and a selection fully covered by an earlier step (nothing to run, and that
    is correct — a roster-only connector's models are exactly the excluded
    ones).
    """
    script = _dbt_run_script(docs)
    assert 'ls --profiles-dir . --resource-type model --select "$DBT_SELECT")' in script, (
        "the typo guard must resolve --select on its own"
    )


def test_a_selection_fully_covered_by_an_earlier_step_succeeds(docs) -> None:
    """...and it must skip the run rather than invoke dbt with nothing to do."""
    script = _dbt_run_script(docs)
    assert "SKIP_RUN=1" in script, "an empty post-exclusion selection must skip the run"
    assert 'if [[ "$RC" -eq 0 && "${SKIP_RUN:-0}" -eq 0 ]]; then' in script, (
        "the skip must bypass `dbt run`, not merely log"
    )

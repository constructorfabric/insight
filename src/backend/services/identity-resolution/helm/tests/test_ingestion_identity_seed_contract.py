"""Helm render-contract for the ingestion pipeline's identity-seed step.

Two production defects motivate this file, neither visible to `helm lint`:

  * the step's image tag came from the UMBRELLA's `.Chart.AppVersion`, while
    the identity Deployment resolves the identity SUBCHART's. The release
    pipeline sets the umbrella's to whichever service built last, so the
    pipeline pulled a tag that was never pushed (`ImagePullBackOff`, and every
    connector's gold build blocked behind it);
  * the gold step's `--exclude` was folded into the selector guard, so a
    connector whose only models ARE the excluded ones (a roster-only source)
    resolved to nothing and tripped the #2362 typo guard — a pipeline with
    nothing left to do failed instead.

The dbt-run assertions are behavioural: the rendered script is executed
against a stub `dbt`, so a rewrite that preserves the text but breaks the
wiring fails here.
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest
import yaml
from conftest import TENANT, UMBRELLA, UMBRELLA_BASE, render

INGESTION = [
    "--set",
    "ingestion.templates.enabled=true",
    "--set",
    "ingestion.toolboxImage=ghcr.io/example/insight-toolbox:0.0.0-test",
    # The umbrella refuses to render an enabled seed CronJob without a tenant.
    "--set",
    f"global.tenantDefaultId={TENANT}",
]


def _docs(*extra: str) -> list[dict]:
    rc, out, err = render(UMBRELLA, *UMBRELLA_BASE, *INGESTION, *extra)
    assert rc == 0, err
    return [d for d in yaml.safe_load_all(out) if isinstance(d, dict)]


@pytest.fixture(scope="module")
def docs(umbrella_deps) -> list[dict]:
    return _docs()


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


def _identity_deployment_image(docs: list[dict]) -> str:
    deployment = _named(docs, "Deployment", "-identity-resolution")
    # `migrate` runs as a hook Job, so the app container is the only one here.
    (container,) = deployment["spec"]["template"]["spec"]["containers"]
    return container["image"]


def test_the_seed_step_runs_the_same_image_as_the_identity_deployment(docs) -> None:
    """The one assertion the ImagePullBackOff needed: the step must not derive
    a tag of its own, because the umbrella's appVersion names a different
    service's build."""
    assert _seed_step_image(docs) == _identity_deployment_image(docs)


def test_an_explicit_tag_pins_both_sides(umbrella_deps) -> None:
    """An operator pinning `identityResolution.image.tag` must move the step
    and the Deployment together, not one of them."""
    docs = _docs("--set", "identityResolution.image.tag=0.0.0-pinned")
    assert _seed_step_image(docs) == _identity_deployment_image(docs)
    assert _identity_deployment_image(docs).endswith(":0.0.0-pinned")


def test_the_gold_step_excludes_what_the_staging_step_built(docs) -> None:
    """The dedup this contract keeps: a `tag:X+` graph walk would otherwise
    rebuild the staging models the pipeline just built."""
    templates = _named(docs, "WorkflowTemplate", "ingestion-pipeline")["spec"]["templates"]
    (transform,) = [t for t in templates if t["name"] == "transform"]
    (gold,) = [t for t in transform["dag"]["tasks"] if t["name"] == "transform-legacy"]
    excludes = [p["value"] for p in gold["arguments"]["parameters"] if p["name"] == "dbt_exclude"]
    assert excludes == ["tag:{{inputs.parameters.data_source}} identity_inputs"]


# ── dbt-run: the selector guard, executed ────────────────────────────────────


def _dbt_run_script(docs: list[dict]) -> str:
    template = _named(docs, "WorkflowTemplate", "dbt-run")["spec"]["templates"][0]
    return template["script"]["source"]


def _run_script(
    tmp_path: Path,
    script: str,
    *,
    selected: str,
    remaining: str,
    select: str,
    exclude: str,
) -> tuple[int, str]:
    """Execute the rendered script with a stub `dbt` on PATH.

    Two lines cannot run outside the toolbox image and are replaced: the `cd`
    into the image's dbt project, and the profiles.yml heredoc (it needs a
    real adapter). Everything the guard does is executed verbatim.
    """
    stub = tmp_path / "bin"
    stub.mkdir()
    # The stub distinguishes the two resolutions, which is the whole point:
    # `ls --select X` answers with the selection, `ls --select X --exclude Y`
    # with what survives it. A stub blind to --exclude cannot reproduce the
    # roster-only failure at all.
    (stub / "dbt").write_text(
        "#!/usr/bin/env bash\n"
        'case "$*" in\n'
        '  *" ls "*--exclude*) printf "%s" "$STUB_LS_REMAINING" ;;\n'
        '  *" ls "*) printf "%s" "$STUB_LS_SELECTED" ;;\n'
        '  *) echo "DBT_RUN_INVOKED $*" ;;\n'
        "esac\n"
        "exit 0\n"
    )
    (stub / "dbt").chmod(0o755)

    # Strip ONLY the first heredoc — the profiles.yml writer, which needs a
    # real adapter. The trailing one emits the lifecycle event and must run.
    keep, skipping, stripped = [], False, False
    for line in script.replace("cd /ingestion/dbt", f"cd {tmp_path}").splitlines():
        if not stripped and line.strip() == "python3 - <<'PY'":
            skipping, stripped = True, True
            continue
        if skipping:
            if line.strip() == "PY":
                skipping = False
            continue
        keep.append(line)
    script_file = tmp_path / "script.sh"
    script_file.write_text("\n".join(keep))

    env = {
        **os.environ,
        "PATH": f"{stub}:{os.environ['PATH']}",
        "STUB_LS_SELECTED": selected,
        "STUB_LS_REMAINING": remaining,
        "DBT_SELECT": select,
        "DBT_EXCLUDE": exclude,
        "DBT_FULL_REFRESH": "false",
        "CLICKHOUSE_HOST": "h",
        "CLICKHOUSE_PORT": "8123",
        "CLICKHOUSE_USER": "u",
        "CLICKHOUSE_PASSWORD": "p",
    }
    proc = subprocess.run(
        ["bash", str(script_file)],
        capture_output=True,
        text=True,
        timeout=120,
        env=env,
        check=False,
    )
    return proc.returncode, proc.stdout + proc.stderr


def test_a_selection_an_earlier_step_covered_is_not_an_error(docs, tmp_path) -> None:
    """The roster-only case that failed in production: the selector resolves,
    the exclusion covers all of it, and the step must still succeed."""
    rc, output = _run_script(
        tmp_path,
        _dbt_run_script(docs),
        selected="model.ingestion.github_directory__org_members",
        remaining="",  # the exclusion covers every model the selector matched
        select="tag:github-directory+",
        exclude="tag:github-directory identity_inputs",
    )
    assert rc == 0, output
    assert "DBT_RUN_INVOKED" in output, "the run must still be attempted"
    assert "--exclude" in output, "the exclusion must reach dbt run"


def test_a_selector_matching_nothing_still_fails(docs, tmp_path) -> None:
    """#2362's guard, intact: a typo'd selector must not report success."""
    rc, output = _run_script(
        tmp_path,
        _dbt_run_script(docs),
        selected="",
        remaining="",
        select="tag:typo+",
        exclude="",
    )
    assert rc == 1, output
    assert "matches no models" in output
    assert "DBT_RUN_INVOKED" not in output, "nothing may run behind a failed guard"


def test_a_normal_connector_runs_with_both_flags(docs, tmp_path) -> None:
    rc, output = _run_script(
        tmp_path,
        _dbt_run_script(docs),
        selected="model.a\nmodel.b",
        remaining="model.b",
        select="tag:github+",
        exclude="tag:github identity_inputs",
    )
    assert rc == 0, output
    assert "--select tag:github+" in output
    assert "--exclude tag:github identity_inputs" in output


def test_an_empty_selector_still_runs_everything(docs, tmp_path) -> None:
    """Manual triggers submit no selector; the guard must stay out of the way."""
    rc, output = _run_script(
        tmp_path,
        _dbt_run_script(docs),
        selected="",
        remaining="",
        select="",
        exclude="",
    )
    assert rc == 0, output
    assert "DBT_RUN_INVOKED" in output
    assert "--select" not in output

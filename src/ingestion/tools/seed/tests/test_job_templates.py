"""Every variable a Job template carries is one `seed-stand.sh` substitutes.

The templates are applied to a cluster with `envsubst <allowlist>`, which
leaves anything outside the list verbatim: a placeholder the render function
forgets reaches the apiserver as the literal `${NAME}`. Kubernetes accepts most
of them — a `${SEED_PULL_SECRETS}` that never expands is a valid-looking field
whose first symptom is ImagePullBackOff on a stand that pulls privately — so
nothing downstream catches it.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

SEED_DIR = Path(__file__).resolve().parents[1]
SEED_STAND = SEED_DIR / "seed-stand.sh"

#: template file -> the render function in seed-stand.sh that substitutes it.
TEMPLATES = {
    "seed-job.yaml.tpl": "render_seed_manifest",
    "bronze-job.yaml.tpl": "render_bronze_manifest",
}

PLACEHOLDER = re.compile(r"\$\{([A-Z_][A-Z0-9_]*)\}")


def _allowlist(function: str) -> set[str]:
    """The names `function`'s envsubst call will expand."""
    body = re.search(
        rf"^{function}\(\) \{{\n(.*?)^\}}",
        SEED_STAND.read_text(encoding="utf-8"),
        re.DOTALL | re.MULTILINE,
    )
    assert body is not None, f"{function} is not defined in {SEED_STAND.name}"

    return set(PLACEHOLDER.findall(body.group(1)))


def _placeholders(template: str) -> set[str]:
    return set(PLACEHOLDER.findall((SEED_DIR / template).read_text(encoding="utf-8")))


@pytest.mark.parametrize(("template", "function"), TEMPLATES.items())
def test_every_placeholder_in_the_template_is_substituted(template: str, function: str) -> None:
    missing = sorted(_placeholders(template) - _allowlist(function))

    assert not missing, (
        f"{template} carries {missing}, which {function} does not substitute — "
        f"they would reach the cluster as literals"
    )


@pytest.mark.parametrize(("template", "function"), TEMPLATES.items())
def test_the_substitution_list_names_nothing_the_template_dropped(
    template: str, function: str
) -> None:
    """A stale name is a variable the script still resolves for nobody."""
    stale = sorted(_allowlist(function) - _placeholders(template))

    assert not stale, f"{function} substitutes {stale}, which {template} no longer carries"


@pytest.mark.parametrize("template", TEMPLATES)
def test_a_failure_is_read_rather_than_retried(template: str) -> None:
    """Every failure these Jobs produce is deterministic — an unreachable
    ClickHouse, a rejected catalogue, a refused preflight. A retry reproduces
    it and leaves the reader asking which attempt to open."""
    body = (SEED_DIR / template).read_text(encoding="utf-8")

    assert "kind: Job" in body, f"{template} is not a Job manifest"
    assert "backoffLimit: 0" in body, f"{template} retries a deterministic failure"


def test_the_destination_reads_its_records_from_the_traces_file() -> None:
    """The redirect IS the mechanism: a Pod cannot otherwise give it stdin.

    Without it the destination waits on an empty stdin, exits having been told
    about no stream, and the Job goes green with no table created.
    """
    body = (SEED_DIR / "bronze-job.yaml.tpl").read_text(encoding="utf-8")

    assert "< /catalog/traces.jsonl" in body, (
        "the destination is not fed the stream statuses that make it create tables"
    )
    assert "exec /airbyte/base.sh write" in body, (
        "the image's entrypoint must replace the shell, or the deadline reaches sh and not it"
    )

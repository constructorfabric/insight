"""Helm render-contract for the `insight-preview` per-experiment bundle (#1971).

A preview experiment's contract is that it serves ONE FE build under
`/exp/<name>` on a shared host, added and removed as one route object without
touching central config. These render facts encode that: `helm template` +
assertions, no cluster. Runs anywhere helm + PyYAML exist
(CI: .github/workflows/preview-helm.yml).

Covered:
  * three resources named `preview-<experiment>` so experiments coexist and
    `helm uninstall preview-<experiment>` removes exactly one;
  * the Ingress prefix-strips `/exp/<name>` (`rewrite-target: /$2`,
    `path: /exp/<name>(/|$)(.*)`) so one image serves under any prefix;
  * an invalid experiment slug and a missing host FAIL the render (they would
    otherwise produce a broken URL segment or an unroutable object);
  * the FE carries no auth env (login is the gateway+authenticator's job).
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

HERE = Path(__file__).resolve()
CHART = HERE.parents[1]  # deploy/preview

BASE = ["--set", "experiment=widget-alpha", "--set", "image.tag=abc123", "--set", "ingress.host=preview.example.com"]


def _render(*extra: str) -> tuple[int, str, str]:
    proc = subprocess.run(  # noqa: S603 — test-controlled argv
        ["helm", "template", "pv", str(CHART), *extra],  # noqa: S607
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    return proc.returncode, proc.stdout, proc.stderr


def _docs(manifests: str) -> list[dict]:
    return [d for d in yaml.safe_load_all(manifests) if isinstance(d, dict)]


def _the(docs: list[dict], kind: str) -> dict:
    matches = [d for d in docs if d.get("kind") == kind]
    assert len(matches) == 1, f"expected exactly one {kind}, got {len(matches)}"
    return matches[0]


def _render_ok(*extra: str) -> list[dict]:
    code, out, err = _render(*BASE, *extra)
    assert code == 0, f"render failed: {err}"
    return _docs(out)


def test_resources_are_named_per_experiment():
    docs = _render_ok()
    for kind in ("Deployment", "Service", "Ingress"):
        assert _the(docs, kind)["metadata"]["name"] == "preview-widget-alpha"


def test_ingress_prefix_strips_the_exp_path():
    ingress = _the(_render_ok(), "Ingress")
    anns = ingress["metadata"]["annotations"]
    assert anns["nginx.ingress.kubernetes.io/rewrite-target"] == "/$2"
    assert anns["nginx.ingress.kubernetes.io/use-regex"] == "true"

    rule = ingress["spec"]["rules"][0]
    assert rule["host"] == "preview.example.com"
    path = rule["http"]["paths"][0]
    assert path["path"] == "/exp/widget-alpha(/|$)(.*)"
    assert path["pathType"] == "ImplementationSpecific"
    assert path["backend"]["service"]["name"] == "preview-widget-alpha"


def test_custom_base_path_is_honored():
    ingress = _the(_render_ok("--set", "ingress.basePath=/preview"), "Ingress")
    path = ingress["spec"]["rules"][0]["http"]["paths"][0]["path"]
    assert path == "/preview/widget-alpha(/|$)(.*)"


def test_service_selects_the_deployment_pods():
    docs = _render_ok()
    selector = _the(docs, "Service")["spec"]["selector"]
    pod_labels = _the(docs, "Deployment")["spec"]["template"]["metadata"]["labels"]
    assert selector.items() <= pod_labels.items()


def test_image_is_the_pinned_tag():
    container = _the(_render_ok(), "Deployment")["spec"]["template"]["spec"]["containers"][0]
    assert container["image"].endswith(":abc123")


@pytest.mark.parametrize("bad", ["Widget_Bad", "UPPER", "-lead", "trail-", "a/b"])
def test_invalid_experiment_slug_fails(bad):
    code, _out, err = _render(
        "--set", f"experiment={bad}", "--set", "image.tag=t", "--set", "ingress.host=h.example.com"
    )
    assert code != 0, f"slug {bad!r} should be rejected"
    assert "DNS-1123 label" in err


def test_overlong_experiment_slug_fails():
    code, _out, err = _render(
        "--set", f"experiment={'a' * 56}", "--set", "image.tag=t", "--set", "ingress.host=h.example.com"
    )
    assert code != 0
    assert "too long" in err


def test_missing_host_fails():
    code, _out, err = _render("--set", "experiment=ok", "--set", "image.tag=t")
    assert code != 0
    assert "ingress.host is required" in err


def test_missing_image_tag_fails():
    code, _out, err = _render("--set", "experiment=ok", "--set", "ingress.host=h.example.com")
    assert code != 0
    assert "image.tag is required" in err


def test_frontend_carries_no_auth_env():
    container = _the(_render_ok(), "Deployment")["spec"]["template"]["spec"]["containers"][0]
    assert "env" not in container

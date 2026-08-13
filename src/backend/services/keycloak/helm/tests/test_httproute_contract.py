"""Helm render-contract for the Keycloak HTTPRoute realm allowlist.

The route publishes /kc on the shared Gateway, and the allowlist is the
security boundary: a bare /kc/realms PathPrefix would also publish the master
realm, whose admin-cli password-grant token endpoint is a brute-force target.
The exact set of published path values is therefore contract, not plumbing:
these tests render the chart(s) with `helm template` and assert the manifests
the cluster would actually get. No cluster involved; runs anywhere helm +
PyYAML exist (CI: .github/workflows/keycloak-helm.yml).

Covered:
  * one realm publishes exactly its own /kc/realms/<realm> prefix plus
    /kc/resources — nothing else;
  * several realms publish one exact prefix each;
  * an empty list, an empty-string entry, a null entry, and `master` each
    FAIL the render (fail-closed — never degrade to the bare prefix);
  * no successful render contains /kc/realms or /kc/realms/ as a complete
    path value;
  * the committed local gitops overlay renders the umbrella successfully
    (it enables the route, so it would fail atomically if the allowlist
    requirement and the overlay ever drift apart).
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest
import yaml

HERE = Path(__file__).resolve()
SUBCHART = HERE.parents[1]  # .../keycloak/helm
REPO_ROOT = HERE.parents[6]
UMBRELLA = REPO_ROOT / "charts" / "insight"
LOCAL_OVERLAY = REPO_ROOT / "deploy" / "gitops" / "environments" / "local" / "values.yaml.template"

# Minimum viable subchart install: every `required` value outside the route.
SUBCHART_BASE = [
    "--set",
    "hostname=https://host.test/kc",
    "--set",
    "admin.existingSecret=kc-admin",
    "--set",
    "database.host=db",
    "--set",
    "database.username=kc",
    "--set",
    "database.passwordSecret.name=db-creds",
    "--set",
    "database.passwordSecret.key=pw",
    "--set",
    "route.enabled=true",
    "--set",
    "route.host=host.test",
]

BARE_PREFIXES = {"/kc/realms", "/kc/realms/"}


def _render(chart: Path, *extra: str) -> tuple[int, str, str]:
    proc = subprocess.run(  # noqa: S603 — test-controlled argv
        ["helm", "template", "contract-test", str(chart), *extra],  # noqa: S607
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    return proc.returncode, proc.stdout, proc.stderr


def _docs(manifests: str) -> list[dict]:
    return [d for d in yaml.safe_load_all(manifests) if isinstance(d, dict)]


def _realms_flag(realms: list) -> list[str]:
    # --set-json survives entries --set cannot express: "", null.
    return ["--set-json", f"route.realms={json.dumps(realms)}"]


def _route_paths(docs: list[dict]) -> list[str]:
    routes = [d for d in docs if d.get("kind") == "HTTPRoute" and "keycloak" in d["metadata"]["name"]]
    assert len(routes) == 1, f"expected exactly one Keycloak HTTPRoute, got {len(routes)}"

    (rule,) = routes[0]["spec"]["rules"]
    paths = [m["path"] for m in rule["matches"]]
    assert all(p["type"] == "PathPrefix" for p in paths)
    return [p["value"] for p in paths]


def _published(realms: list) -> list[str]:
    rc, out, err = _render(SUBCHART, *SUBCHART_BASE, *_realms_flag(realms))
    assert rc == 0, err

    paths = _route_paths(_docs(out))
    assert not BARE_PREFIXES & set(paths), f"bare realm prefix published: {paths}"
    return paths


def test_one_realm_publishes_exactly_that_realm() -> None:
    assert _published(["insight"]) == ["/kc/realms/insight", "/kc/resources"]


def test_several_realms_publish_one_exact_prefix_each() -> None:
    assert _published(["insight", "partner"]) == ["/kc/realms/insight", "/kc/realms/partner", "/kc/resources"]


@pytest.mark.parametrize(
    ("realms", "expected_error"),
    [
        ([], "must list the realm"),
        ([""], "must be non-empty"),
        ([None], "must be non-empty"),
        (["master"], "must not include master"),
        (["insight", ""], "must be non-empty"),
    ],
    ids=["empty-list", "empty-string", "null", "master", "trailing-empty"],
)
def test_bad_allowlists_fail_the_render(realms: list, expected_error: str) -> None:
    rc, _, err = _render(SUBCHART, *SUBCHART_BASE, *_realms_flag(realms))
    assert rc != 0, f"render must fail for realms={realms!r}"
    assert expected_error in err, err


def test_route_disabled_renders_no_httproute() -> None:
    rc, out, err = _render(SUBCHART, *SUBCHART_BASE, "--set", "route.enabled=false")
    assert rc == 0, err
    assert not [d for d in _docs(out) if d.get("kind") == "HTTPRoute"]


def test_local_gitops_overlay_renders() -> None:
    """The committed local overlay enables the route, so it must carry a
    valid allowlist — this fails atomically if the two ever drift apart.

    Requires `helm dependency update charts/insight` (the CI workflow runs
    it; locally, run it once).
    """
    if not (UMBRELLA / "charts").is_dir():
        pytest.skip("umbrella dependencies not built — run `helm dependency update charts/insight`")

    rc, out, err = _render(UMBRELLA, "-f", str(LOCAL_OVERLAY))
    assert rc == 0, err

    paths = _route_paths(_docs(out))
    assert "/kc/realms/insight" in paths
    assert not BARE_PREFIXES & set(paths), f"bare realm prefix published: {paths}"

"""A connector image is registered under the install's registry mirror.

A cluster that may only pull from its own registry rejects a connector pod
whose image names another one, and Airbyte's JOB_KUBE_CONNECTOR_IMAGE_REGISTRY
only prefixes references that name no registry. So `disc_load_descriptors`
rewrites the descriptor's `images.cdk.image` through RECONCILE_IMAGE_MIRRORS
before anything downstream reads it.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]

IMAGE = "ghcr.io/example-org/source-alpha:1.2.3"


def cdk_image_column(tmp_path: Path, image: str, mirrors: dict[str, str] | None) -> str:
    """The cdk_image column `disc_load_descriptors` emits for one descriptor."""
    connector = tmp_path / "connectors" / "hr" / "alpha"
    connector.mkdir(parents=True)
    (connector / "descriptor.yaml").write_text(
        f"name: alpha\nversion: '1.0.0'\ntype: cdk\nimages:\n  cdk:\n    image: \"{image}\"\n", encoding="utf-8"
    )
    env = f"export RECONCILE_IMAGE_MIRRORS={json.dumps(json.dumps(mirrors))}" if mirrors is not None else ""
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight
    export CONNECTORS_DIR="{tmp_path / "connectors"}"
    {env}
    source "{ROOT}/lib/discover.sh"
    disc_load_descriptors
    """
    result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, check=False)
    assert result.returncode == 0, result.stderr
    return result.stdout.rstrip("\n").split("\t")[4]


class TestConnectorImageMirrors:
    @pytest.mark.parametrize("mirror", ["harbor.example.com/proxy-ghcr.io", "harbor.example.com/proxy-ghcr.io/"])
    def test_a_mapped_registry_is_replaced_by_its_mirror(self, tmp_path: Path, mirror: str) -> None:
        image = cdk_image_column(tmp_path, IMAGE, {"ghcr.io": mirror})

        assert image == "harbor.example.com/proxy-ghcr.io/example-org/source-alpha:1.2.3"

    def test_an_unmapped_registry_is_left_alone(self, tmp_path: Path) -> None:
        image = cdk_image_column(tmp_path, IMAGE, {"quay.io": "harbor.example.com/proxy-quay.io"})

        assert image == IMAGE

    def test_without_mirrors_the_descriptor_reference_is_emitted_as_is(self, tmp_path: Path) -> None:
        assert cdk_image_column(tmp_path, IMAGE, None) == IMAGE

    def test_a_registry_name_inside_the_path_is_not_rewritten(self, tmp_path: Path) -> None:
        """Only the leading registry component is a registry."""
        image = cdk_image_column(tmp_path, "harbor.example.com/proxy-ghcr.io/x:1", {"ghcr.io": "mirror.example.com"})

        assert image == "harbor.example.com/proxy-ghcr.io/x:1"


#: The Airbyte calls `reconcile_definitions` makes for a CDK connector whose
#: definition already exists, answered from env; the migration is replaced by a
#: record of its arguments, which is the decision under test.
DEFINITION_STUBS = """
log_line()  { :; }
log_event() { :; }
ab_workspace_id() { printf 'workspace-1'; }
reconcile_find_custom_definition_id() {
  if [[ -f "$MIGRATED" ]]; then printf 'def-2'; else printf 'def-1'; fi
}
ab_get_definition() {
  printf '{"dockerRepository": "%s", "dockerImageTag": "1.2.3"}' "$CURRENT_REPO"
}
ab_set_definition_image_tag() { printf 'TAG %s\\n' "$*" >> "$MIGRATED.calls"; }
reconcile_migrate_definition_kind() { printf 'MIGRATE %s\\n' "$*" > "$MIGRATED"; }
"""


def definitions_decision(tmp_path: Path, current_repo: str, desired: str, mirrors: dict[str, str]) -> tuple[str, str]:
    """`reconcile_definitions` output and the migration it asked for, if any."""
    migrated = tmp_path / "migrated"
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight
    export CONNECTORS_DIR="{ROOT}/../connectors"
    export AIRBYTE_URL=http://127.0.0.1:1
    export INSIGHT_TENANT_ID=example-tenant
    export RECONCILE_IMAGE_MIRRORS={json.dumps(json.dumps(mirrors))}
    export CURRENT_REPO="{current_repo}" MIGRATED="{migrated}"
    source "{ROOT}/lib/reconcile.sh"
    {DEFINITION_STUBS}
    reconcile_definitions alpha 1.0.0 cdk "{tmp_path}" "{desired}"
    """
    result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, check=False)
    assert result.returncode == 0, result.stderr
    return result.stdout.strip(), migrated.read_text(encoding="utf-8").strip() if migrated.exists() else ""


class TestAnExistingDefinitionFollowsTheMirror:
    MIRRORS = {"ghcr.io": "harbor.example.com/proxy-ghcr.io"}
    MIRRORED = "harbor.example.com/proxy-ghcr.io/example-org/source-alpha:1.2.3"

    def test_a_definition_on_the_mirrored_registry_is_migrated_to_the_mirror(self, tmp_path: Path) -> None:
        """Turning a mirror on after the first reconcile: the definition still
        names a registry the cluster may not pull from, and only its tag can be
        updated, so it is rebuilt under the mirror with its state kept."""
        out, migration = definitions_decision(tmp_path, "ghcr.io/example-org/source-alpha", self.MIRRORED, self.MIRRORS)

        assert migration == f"MIGRATE alpha def-1 {self.MIRRORED}"
        assert out == "republish\tpatch\tdef-2"

    def test_any_other_repository_change_is_left_to_an_operator(self, tmp_path: Path) -> None:
        out, migration = definitions_decision(tmp_path, "ghcr.io/other-org/source-alpha", self.MIRRORED, self.MIRRORS)

        assert migration == ""
        assert out == "noop\tnone\tdef-1"

    def test_a_definition_already_on_the_mirror_is_left_alone(self, tmp_path: Path) -> None:
        out, migration = definitions_decision(
            tmp_path, "harbor.example.com/proxy-ghcr.io/example-org/source-alpha", self.MIRRORED, self.MIRRORS
        )

        assert migration == ""
        assert out == "noop\tnone\tdef-1"

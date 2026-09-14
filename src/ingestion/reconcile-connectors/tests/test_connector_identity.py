"""Where a connector instance's identity actually comes from.

Everything downstream treats what this resolves as fact: the Airbyte source and
connection are named from it, the ledger records rows under it, and the backfill
hands a whole history to it. A value invented here would be written into the
record as something nobody configured.

So the rule under test is narrow — the Secret's own `source-id` annotation is
the identity, and `main` is not a stand-in for one that could not be read. It is
the id an unannotated instance is *already* named by everywhere else, which is
what keeps the row and the connection agreeing about which instance they are.

Drives the real bash with only its two collaborators replaced. A rewrite in
Python would test the rewrite.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

#: `disc_match_descriptor_to_secret` bodies — a Secret, or none at all.
A_SECRET = "printf 'insight-claude-team-second'"
NO_SECRET = "return 1"

#: `kubectl` bodies. On this path it is only ever asked for one annotation, so
#: the stub answers with the value rather than parsing the jsonpath.
ANNOTATED = "printf 'claude-team-second'"
UNANNOTATED = "printf ''"


def resolve(call: str, *, secret: str, annotation: str) -> str:
    """Run one naming helper with the cluster replaced by two stubs."""
    script = f"""
    set -uo pipefail
    export INSIGHT_NAMESPACE=insight
    export CONNECTORS_DIR="{ROOT}/../connectors"
    export INSIGHT_TENANT_ID=example-tenant
    source "{ROOT}/lib/connector-naming.sh"
    disc_match_descriptor_to_secret() {{ {secret}; }}
    kubectl() {{ {annotation}; }}
    {call}
    """
    result = subprocess.run(
        ["bash", "-c", script], capture_output=True, text=True, check=False
    )
    assert result.returncode == 0, result.stderr
    return result.stdout


class TestTheIdentityIsReadNotInvented:
    def test_the_secrets_own_source_id_is_what_is_used(self) -> None:
        """An instance annotated with an id of its own must not be recorded
        under a conventional one: the backfill hands it every row it ever
        wrote, and a wrong id there is indistinguishable from a right one."""
        assert (
            resolve(
                'reconcile_compute_source_id "claude-team"',
                secret=A_SECRET,
                annotation=ANNOTATED,
            )
            == "claude-team-second"
        )

    def test_an_unannotated_secret_resolves_the_id_it_is_already_named_by(self) -> None:
        """`main` is not a guess at an unknown value.

        It is the id this instance's Airbyte source and connection already
        carry, so recording it keeps the ledger saying what the rest of the
        install says about the same thing.
        """
        assert (
            resolve(
                'reconcile_compute_source_id "claude-team"',
                secret=A_SECRET,
                annotation=UNANNOTATED,
            )
            == "main"
        )

    def test_no_secret_at_all_resolves_the_same_way(self) -> None:
        assert (
            resolve(
                'reconcile_compute_source_id "claude-team"',
                secret=NO_SECRET,
                annotation=UNANNOTATED,
            )
            == "main"
        )

    def test_the_tenant_is_the_installs_own_not_the_secrets(self) -> None:
        """One reconcile run drives one tenant — the chart requires it and the
        loop exports it — so the tenant a row is recorded under is the
        install's, whatever any Secret says."""
        assert (
            resolve(
                'reconcile_compute_tenant "claude-team"',
                secret=A_SECRET,
                annotation=ANNOTATED,
            )
            == "example-tenant"
        )


class TestTheNameAndTheRecordCannotDisagree:
    def test_the_connection_name_is_built_from_the_same_resolution(self) -> None:
        """The sweep finds an instance's jobs by this name and records them
        under that identity. Resolved by two different rules they could differ,
        and the sweep would then file one instance's syncs under another's id.
        """
        assert (
            resolve(
                'reconcile_compute_connection_name "claude-team"',
                secret=A_SECRET,
                annotation=ANNOTATED,
            )
            == "claude-team-claude-team-second-example-tenant-conn"
        )

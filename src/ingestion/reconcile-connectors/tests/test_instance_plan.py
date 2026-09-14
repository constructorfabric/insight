"""The desired state: every descriptor, joined with every instance of it.

This is the relation the whole tick is driven from, including its destructive
half — "not in the plan" is what removes a source. So the cases here are mostly
about what the plan must NOT say: not short because an annotation was missing,
not arbitrary because two Secrets claimed one instance, not silent about a
connector this build does not ship.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXTRACT = ROOT / "python" / "extract_secret_loop.py"
PLAN = ROOT / "python" / "plan_instances.py"

DESCRIPTOR_COLUMNS = ("dir", "1", "nocode", "", "", "")


def descriptor(name: str) -> str:
    """The eight columns `disc_load_descriptors` emits, namespace last."""
    namespace = "bronze_" + name.replace("-", "_")
    return "\t".join([name, *DESCRIPTOR_COLUMNS, namespace])


def secret_row(connector: str, source_id: str, name: str, cfg_hash: str = "hash") -> str:
    return "\t".join([connector, source_id, name, cfg_hash])


def run_plan(
    tmp_path: Path, descriptors: list[str], secrets: list[str]
) -> subprocess.CompletedProcess[str]:
    descriptors_file = tmp_path / "descriptors.tsv"
    secrets_file = tmp_path / "secrets.tsv"
    descriptors_file.write_text("\n".join(descriptors) + "\n", encoding="utf-8")
    secrets_file.write_text("\n".join(secrets) + "\n" if secrets else "", encoding="utf-8")
    return subprocess.run(
        [sys.executable, str(PLAN), str(descriptors_file), str(secrets_file)],
        capture_output=True,
        text=True,
        check=False,
    )


def instances(result: subprocess.CompletedProcess[str]) -> list[tuple[str, str, str]]:
    """`(connector, source_id, secret_name)` per emitted row."""
    rows = []
    for line in result.stdout.splitlines():
        fields = line.split("\t")
        rows.append((fields[0], fields[8], fields[9]))
    return rows


def run_extract(secrets: list[dict]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(EXTRACT)],
        input=json.dumps({"items": secrets}),
        capture_output=True,
        text=True,
        check=False,
    )


def a_secret(name: str, annotations: dict[str, str]) -> dict:
    return {"metadata": {"name": name, "annotations": annotations}, "data": {"k": "dg=="}}


class TestASecretIsReadIntoAnInstance:
    def test_the_annotated_source_id_names_the_instance(self) -> None:
        result = run_extract(
            [
                a_secret(
                    "insight-claude-team-second",
                    {
                        "insight.cyberfabric.com/connector": "claude-team",
                        "insight.cyberfabric.com/source-id": "claude-team-second",
                    },
                )
            ]
        )

        assert result.returncode == 0, result.stderr
        assert result.stdout.split("\t")[:3] == [
            "claude-team",
            "claude-team-second",
            "insight-claude-team-second",
        ]

    def test_a_secret_naming_no_source_id_is_the_main_instance(self) -> None:
        """Not skipped. A skipped Secret takes its connector out of the desired
        state, and a connector absent from the desired state is one the loop
        deletes the sources of."""
        result = run_extract(
            [
                a_secret(
                    "insight-claude-team",
                    {"insight.cyberfabric.com/connector": "claude-team"},
                )
            ]
        )

        assert result.returncode == 0, result.stderr
        assert result.stdout.split("\t")[1] == "main"

    def test_a_secret_naming_no_connector_is_skipped_with_a_warning(self) -> None:
        """It names nothing this loop manages, so there is nothing to place it
        against — and no connector loses anything by its absence."""
        result = run_extract([a_secret("someone-elses", {"other/annotation": "x"})])

        assert result.returncode == 0, result.stderr
        assert result.stdout.strip() == ""
        assert "missing connector annotation" in result.stderr


class TestTheJoin:
    def test_a_connector_installed_twice_is_two_rows(self, tmp_path: Path) -> None:
        result = run_plan(
            tmp_path,
            [descriptor("claude-team")],
            [
                secret_row("claude-team", "claude-team-main", "secret-main"),
                secret_row("claude-team", "claude-team-second", "secret-second"),
            ],
        )

        assert result.returncode == 0, result.stderr
        assert instances(result) == [
            ("claude-team", "claude-team-main", "secret-main"),
            ("claude-team", "claude-team-second", "secret-second"),
        ]

    def test_a_descriptor_no_secret_names_is_still_a_row(self, tmp_path: Path) -> None:
        """It is the row the loop reads as "not installed here", and the cascade
        that removes such a connector's leftovers walks the plan — dropped, its
        sources and schedules would stay behind for ever."""
        result = run_plan(tmp_path, [descriptor("jira")], [])

        assert result.returncode == 0, result.stderr
        assert instances(result) == [("jira", "", "")]

    def test_a_secret_for_a_connector_this_build_lacks_is_reported(self, tmp_path: Path) -> None:
        result = run_plan(
            tmp_path,
            [descriptor("jira")],
            [secret_row("from-the-future", "main", "secret-future")],
        )

        assert result.returncode == 0, result.stderr
        assert instances(result) == [("jira", "", "")]
        assert "this build does not ship" in result.stderr

    def test_two_secrets_claiming_one_instance_are_refused(self, tmp_path: Path) -> None:
        """Which one won would depend on the order the API listed them, so one
        tick would reconcile the instance towards one Secret's credentials and
        the next towards the other's — with nothing wrong-looking in either."""
        result = run_plan(
            tmp_path,
            [descriptor("claude-team")],
            [
                secret_row("claude-team", "main", "secret-a"),
                secret_row("claude-team", "main", "secret-b"),
            ],
        )

        assert result.returncode == 3
        assert "both claim" in result.stderr

    def test_the_order_is_the_same_from_one_tick_to_the_next(self, tmp_path: Path) -> None:
        """The log is read by comparing ticks, which an order that moves makes
        impossible."""
        result = run_plan(
            tmp_path,
            [descriptor("zulip"), descriptor("alpha")],
            [
                secret_row("zulip", "zulip-main", "s1"),
                secret_row("alpha", "b-second", "s2"),
                secret_row("alpha", "a-first", "s3"),
            ],
        )

        assert result.returncode == 0, result.stderr
        assert [(row[0], row[1]) for row in instances(result)] == [
            ("alpha", "a-first"),
            ("alpha", "b-second"),
            ("zulip", "zulip-main"),
        ]

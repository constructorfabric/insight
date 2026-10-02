"""The two contracts the connector-seeding pair owes its caller.

`create-connector-tables.sh` hands paths to a Docker bind mount, which the
daemon resolves — not the client. `seed-connectors.sh` attempts every connector
before reporting, so its exit code is the only thing that tells bootstrap-db.sh
whether the bronze layer it is about to run dbt over is complete.

Neither test needs Docker or ClickHouse: the first stubs the Docker CLI and
reads back the arguments it was handed, the second drives the real loop over
connectors that cannot resolve.

Run: pytest src/ingestion/scripts/tests
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest

BOOTSTRAP_DIR = Path(__file__).resolve().parents[1] / "bootstrap-db"
CREATE_CONNECTOR_TABLES = BOOTSTRAP_DIR / "create-connector-tables.sh"
SEED_CONNECTORS = BOOTSTRAP_DIR / "seed-connectors.sh"

#: Both scripts parse YAML with mikefarah yq v4 and JSON with jq.
requires_shell_tooling = pytest.mark.skipif(
    not all(shutil.which(tool) for tool in ("bash", "yq", "jq")),
    reason="the bootstrap scripts need bash, mikefarah yq v4 and jq",
)

CLICKHOUSE_ENV = {
    "CLICKHOUSE_HOST": "clickhouse.example.internal",
    "CLICKHOUSE_PORT": "8123",
    "CLICKHOUSE_PROTOCOL": "http",
    "CLICKHOUSE_USER": "user-under-test",
    "CLICKHOUSE_PASSWORD": "password-under-test",
    "CLICKHOUSE_DATABASE": "insight",
    "DESTINATION_CLICKHOUSE_IMAGE": "destination-clickhouse:under-test",
    "SOURCE_DECLARATIVE_MANIFEST_IMAGE": "source-declarative-manifest:under-test",
}


def _connector_dir(root: Path) -> Path:
    """A connector the script can read: it wants a name, a type and a namespace."""
    connector = root / "connector"
    connector.mkdir()
    (connector / "descriptor.yaml").write_text("name: probe\nconnection:\n  namespace: bronze_probe\n")
    (connector / "connector.yaml").write_text("version: 0.1.0\n")

    return connector


#: One stream, keyed the way every bronze table is, so the script's own
#: unique_key gate lets a stubbed discover through to the destination.
STUB_CATALOG = (
    '{"type":"CATALOG","catalog":{"streams":[{"name":"probe",'
    '"json_schema":{"properties":{"unique_key":{"type":"string"}}}}]}}'
)


def _docker_stub(root: Path, *, answer_discover: bool = False) -> tuple[Path, Path]:
    """A `docker` that records its arguments instead of running anything.

    Exits non-zero so the script stops at the first `docker run`, which is the
    only invocation most of these tests care about. `answer_discover` hands back a
    catalog instead, for the two that need the destination invocation to happen.
    """
    stub_dir = root / "stub-bin"
    stub_dir.mkdir()
    argv_log = root / "docker-argv.log"
    discover = f"if [[ \"$*\" == *' discover '* ]]; then\n  echo '{STUB_CATALOG}'\n  exit 0\nfi\n"
    stub = stub_dir / "docker"
    stub.write_text(
        f'#!/usr/bin/env bash\nprintf "%s\\n" "$*" >> "{argv_log}"\n'
        f'[[ "$1" == "build" ]] && exit 0\n'
        f"{discover if answer_discover else ''}"
        f"exit 1\n"
    )
    stub.chmod(0o755)

    return stub_dir, argv_log


def _bind_mount_source(argv_log: Path, target: str) -> str:
    """The host side of the `-v <source>:<target>[:<option>]` handed to Docker."""
    for invocation in argv_log.read_text().splitlines():
        arguments = invocation.split()
        for flag, value in zip(arguments, arguments[1:]):
            if flag == "-v" and value.split(":")[1:2] == [target]:
                return value.split(":")[0]

    raise AssertionError(f"no bind mount onto {target} in:\n{argv_log.read_text()}")


def _destination_invocation(argv_log: Path) -> str:
    """The recorded `docker run` that hands the catalogue to destination-clickhouse."""
    for invocation in argv_log.read_text().splitlines():
        if " write " in invocation:
            return invocation

    raise AssertionError(f"the destination was never run:\n{argv_log.read_text()}")


def _run_create_connector_tables(
    root: Path, runner_temp: str | None, *, overrides: dict[str, str] | None = None, answer_discover: bool = False
) -> Path:
    """Drive the script over its `docker run`s; return the argv log."""
    root.mkdir(parents=True, exist_ok=True)
    connector = _connector_dir(root)
    config = root / "config.json"
    config.write_text('{"token": "fake"}\n')
    stub_dir, argv_log = _docker_stub(root, answer_discover=answer_discover)

    env = {**os.environ, **CLICKHOUSE_ENV, "PATH": f"{stub_dir}{os.pathsep}{os.environ['PATH']}"}
    env.pop("RUNNER_TEMP", None)
    if runner_temp is not None:
        env["RUNNER_TEMP"] = runner_temp
    env.update(overrides or {})

    subprocess.run(
        ["bash", str(CREATE_CONNECTOR_TABLES), str(connector), str(config)],
        env=env,
        capture_output=True,
        text=True,
        timeout=60,
    )

    return argv_log


@requires_shell_tooling
def test_the_bind_mounted_workdir_lands_under_runner_temp(tmp_path: Path) -> None:
    """A private /tmp is invisible to a separate Docker daemon; RUNNER_TEMP is not.

    Under docker-in-docker the client and the daemon are different containers,
    and a missing bind source is silently created empty rather than refused — so
    getting this wrong costs the connector its config.json, not an error.
    """
    runner_temp = tmp_path / "runner-temp"
    runner_temp.mkdir()

    source = _bind_mount_source(_run_create_connector_tables(tmp_path / "case", str(runner_temp)), "/work")

    assert Path(source).parent == runner_temp, (
        f"the /work bind mount came from {source}, outside RUNNER_TEMP "
        f"({runner_temp}) — a separate Docker daemon cannot see it"
    )


def test_the_workdir_is_allocated_from_a_template_not_the_p_flag() -> None:
    """`mktemp -p` is a GNU extension, and the lane is not the only caller.

    Nothing about the resulting path betrays which spelling produced it, so the
    behavioural tests above cannot catch a return to the non-portable one.
    """
    allocation = next(line for line in CREATE_CONNECTOR_TABLES.read_text().splitlines() if line.startswith("WORKDIR="))

    assert "mktemp -d " in allocation and " -p " not in allocation, (
        f"the workdir must be allocated from an explicit template: {allocation}"
    )


@requires_shell_tooling
def test_the_workdir_falls_back_to_tmp_without_runner_temp(tmp_path: Path) -> None:
    source = _bind_mount_source(_run_create_connector_tables(tmp_path, None), "/work")

    assert Path(source).parent == Path("/tmp"), f"outside Actions the workdir must stay in /tmp, got {source}"


@requires_shell_tooling
def test_a_failing_connector_fails_the_seed_run_but_only_after_all_are_tried(tmp_path: Path) -> None:
    """Partial bootstraps are reported in full and still count as failures.

    Returning zero here would let bootstrap-db.sh run dbt over a bronze layer
    that is missing databases, burying the one real error under an
    UNKNOWN_DATABASE per downstream model.
    """
    names = ["alpha", "beta", "gamma"]
    config = tmp_path / "connectors-config.yaml"
    config.write_text(
        "connectors:\n"
        + "".join(
            f'  {name}:\n    path: no-such/{name}\n    config:\n      token:\n        value: "fake"\n' for name in names
        )
    )

    result = subprocess.run(
        ["bash", str(SEED_CONNECTORS), str(config)],
        env={**os.environ, **CLICKHOUSE_ENV},
        capture_output=True,
        text=True,
        timeout=120,
    )

    assert result.returncode != 0, (
        f"every connector failed and the script still reported success:\n{result.stdout}\n{result.stderr}"
    )
    for name in names:
        assert f"[{name}] FAILED" in result.stderr, (
            f"{name} was never attempted — the loop stopped early:\n{result.stderr}"
        )
    assert f"failed connectors: {' '.join(names)}" in result.stderr, result.stderr


@requires_shell_tooling
def test_the_destination_joins_the_network_it_is_told_to(tmp_path: Path) -> None:
    """A ClickHouse addressable only inside a docker network needs the container on it.

    The destination is the one container that talks to ClickHouse, and the rig that
    needs this (a compose stand) publishes a port no container of ours shares.
    """
    argv_log = _run_create_connector_tables(
        tmp_path, None, overrides={"DOCKER_NETWORK": "network-under-test"}, answer_discover=True
    )

    invocation = _destination_invocation(argv_log)

    assert "--network network-under-test" in invocation, (
        f"the destination did not join the network it was given: {invocation}"
    )


@requires_shell_tooling
def test_the_destination_joins_no_network_unless_told_to(tmp_path: Path) -> None:
    """Every other caller reaches ClickHouse on the host, where an empty --network
    would be an argument Docker rejects rather than a default."""
    argv_log = _run_create_connector_tables(tmp_path, None, answer_discover=True)

    invocation = _destination_invocation(argv_log)

    assert "--network" not in invocation, f"an unasked-for network reached Docker: {invocation}"

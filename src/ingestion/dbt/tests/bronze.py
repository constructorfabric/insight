"""Create one connector's bronze tables the way its first sync does.

A transform suite reads bronze and has to put tables there first. It used to apply
the committed `connectors-ddl/<connector>.sql` snapshot, which made the suite a second
creator of a layer that has exactly one: `destination-clickhouse`, handed a catalogue
built from the connector's own `discover`. The snapshot is gone, and with it the
question of whether the shape a test ran against was the shape a deployment gets.

So the suite runs that creator, through the same `seed-connectors.sh` /
`create-connector-tables.sh` pair `bootstrap-db.sh` uses. Docker is therefore a
dependency of these suites; both lanes already run ClickHouse in a container.

A declarative connector costs two image pulls and two container runs; a CDK one is
built from the working tree first.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

INGESTION = Path(__file__).resolve().parents[2]
BOOTSTRAP = INGESTION / "scripts/bootstrap-db"
CONNECTORS_CONFIG = BOOTSTRAP / "connectors-config.yaml"

#: One `discover` and one `write`, after up to two image pulls.
TIMEOUT_S = 900.0

_TOOLS = ("docker", "yq", "jq")


class BronzeCreationError(RuntimeError):
    """The destination did not create the bronze layer this suite reads."""


def container_host() -> str:
    """ClickHouse's address as a CONTAINER sees it.

    The destination runs in one, so the loopback a host-side client connects on does
    not reach the server. `CLICKHOUSE_CONTAINER_HOST` says what does; without it
    `CLICKHOUSE_HOST` is taken to be an address both sides share — on Linux the docker
    bridge gateway, which is what the CI lanes set.
    """
    return os.environ.get("CLICKHOUSE_CONTAINER_HOST") or os.environ["CLICKHOUSE_HOST"]


def create_bronze(
    connector: str,
    *,
    port: int | str,
    user: str,
    password: str,
    database: str,
    host: str | None = None,
    protocol: str = "http",
    docker_network: str | None = None,
) -> None:
    """Create `connector`'s bronze database and tables in the warehouse named here."""
    missing = [tool for tool in _TOOLS if shutil.which(tool) is None]
    if missing:
        raise BronzeCreationError(
            f"bronze is created by the real connector, which needs {', '.join(missing)} "
            f"on PATH (docker, mikefarah yq v4 and jq)"
        )

    env = {
        **os.environ,
        **_pins(),
        "CLICKHOUSE_HOST": host or container_host(),
        "CLICKHOUSE_PORT": str(port),
        "CLICKHOUSE_PROTOCOL": protocol,
        "CLICKHOUSE_USER": user,
        "CLICKHOUSE_PASSWORD": password,
        "CLICKHOUSE_DATABASE": database,
    }
    if docker_network:
        env["DOCKER_NETWORK"] = docker_network

    try:
        result = subprocess.run(
            [str(BOOTSTRAP / "seed-connectors.sh"), str(CONNECTORS_CONFIG), connector],
            env=env,
            capture_output=True,
            text=True,
            check=False,
            timeout=TIMEOUT_S,
        )
    except subprocess.TimeoutExpired as timeout:
        raise BronzeCreationError(f"{connector} did not create bronze within {TIMEOUT_S:.0f}s") from timeout
    if result.returncode != 0:
        raise BronzeCreationError(
            f"destination-clickhouse did not create {connector}'s bronze "
            f"(exit {result.returncode}):\n{result.stdout[-4000:]}\n{result.stderr[-4000:]}"
        )


def _pins() -> dict[str, str]:
    """The image pins, from the one committed place that holds them."""
    pins: dict[str, str] = {}
    for line in (BOOTSTRAP / "pins.env").read_text(encoding="utf-8").splitlines():
        if line.strip() and not line.startswith("#"):
            key, _, value = line.partition("=")
            pins[key.strip()] = value.strip()
    return pins

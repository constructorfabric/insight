"""Create an instance's bronze tables the way a connector's first sync does.

The harness used to apply the committed `connectors-ddl` snapshot, which made it a
second creator of bronze: the tables a spec seeded were whatever shape the snapshot
carried when it was last dumped, and a table the snapshot lacked was invented. A
deployment has exactly one creator — `destination-clickhouse`, handed a catalogue
built from the connector's own `discover` — so this runs that creator, through the
same `seed-connectors.sh` / `create-connector-tables.sh` pair `bootstrap-db.sh` uses.

Docker is a dependency the harness already has (it restarts services and runs the
seeder through compose), so the scripts are reused rather than reimplemented.

The destination runs in a container, where the instance's published ClickHouse port
is a host address it does not share. It joins the instance's own compose network and
is handed ClickHouse's address on that network.
"""

from __future__ import annotations

import logging
import os
import shutil
import subprocess
from pathlib import Path

from insight_stand.stand import parse_env_file

from insight_datapath.instance import InstanceConfig
from insight_datapath.process import tail

LOG = logging.getLogger("datapath.bronze")

BOOTSTRAP_DIR = Path("src/ingestion/scripts/bootstrap-db")

#: ClickHouse's port inside its own container, not the instance's published one.
SERVICE_PORT = 8123

#: Every connector is discovered and written in turn, and four of them build their
#: source image first.
DEFAULT_TIMEOUT_S = 3600.0

#: One local daemon query. A wedged daemon must fail the fixture, not hang it.
INSPECT_TIMEOUT_S = 60.0

_TOOLS = ("docker", "yq", "jq")


class BronzeCreationError(RuntimeError):
    """The destination did not create the bronze layer a spec seeds into."""


def create_bronze(
    cfg: InstanceConfig,
    *,
    repo_root: Path,
    project: str,
    timeout_s: float = DEFAULT_TIMEOUT_S,
) -> None:
    """Create every connector's bronze database and tables on `cfg`'s instance.

    `project` is the instance's compose project, which `docker-compose.yml` also
    names its network after.
    """
    bootstrap = repo_root / BOOTSTRAP_DIR
    _require_tooling()

    env = {
        **os.environ,
        **parse_env_file(bootstrap / "pins.env"),
        "CLICKHOUSE_HOST": _clickhouse_address(project),
        "CLICKHOUSE_PORT": str(SERVICE_PORT),
        "CLICKHOUSE_PROTOCOL": "http",
        "CLICKHOUSE_USER": cfg.ch_user,
        "CLICKHOUSE_PASSWORD": cfg.ch_password,
        "CLICKHOUSE_DATABASE": cfg.ch_database,
        "DOCKER_NETWORK": project,
    }

    LOG.info("creating bronze through destination-clickhouse on network %s", project)
    try:
        result = subprocess.run(
            [str(bootstrap / "seed-connectors.sh"), str(bootstrap / "connectors-config.yaml")],
            cwd=repo_root,
            env=env,
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout_s,
        )
    except subprocess.TimeoutExpired as timeout:
        raise BronzeCreationError(
            f"the connectors did not create bronze within {timeout_s:.0f}s:\n{tail(timeout.stderr)}"
        ) from timeout
    if result.returncode != 0:
        raise BronzeCreationError(
            f"destination-clickhouse did not create bronze (exit {result.returncode}):\n"
            f"{tail(result.stdout)}\n{tail(result.stderr)}"
        )
    LOG.info("bronze created from the connectors' own catalogues")


def _clickhouse_address(project: str) -> str:
    """ClickHouse's address on the instance's network, as an IP.

    Addressing it by its compose service name would put a container-side DNS lookup
    in front of every connector, and the embedded resolver fails often enough to cost
    a run its warehouse. The address is read once and the name never asked for.
    """
    container = f"{project}-clickhouse"
    template = f'{{{{ (index .NetworkSettings.Networks "{project}").IPAddress }}}}'
    try:
        result = subprocess.run(
            ["docker", "inspect", "--format", template, container],
            capture_output=True,
            text=True,
            check=False,
            timeout=INSPECT_TIMEOUT_S,
        )
    except subprocess.TimeoutExpired as timeout:
        raise BronzeCreationError(
            f"docker did not say where {container} is within {INSPECT_TIMEOUT_S:.0f}s"
        ) from timeout
    address = result.stdout.strip()
    if result.returncode != 0 or not address:
        raise BronzeCreationError(
            f"cannot find {container} on the {project} network, so the connectors have "
            f"no ClickHouse to create bronze in:\n{tail(result.stderr)}"
        )
    return address


def _require_tooling() -> None:
    """Fail before the first connector rather than inside the loop."""
    missing = [tool for tool in _TOOLS if shutil.which(tool) is None]
    if missing:
        raise BronzeCreationError(
            f"bronze is created by the real connectors, which need {', '.join(missing)} "
            f"on PATH (docker, mikefarah yq v4 and jq)"
        )

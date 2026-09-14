"""Resolving which instance's databases a data-path run writes to."""

from __future__ import annotations

from pathlib import Path

import pytest
from insight_datapath.instance import resolve_instance
from insight_stand.errors import StandConnectionError

ENV_FILE_VAR = "INSIGHT_STAND_ENV_FILE"

COMPLETE = """
GATEWAY_PORT=18080
MARIADB_PORT=13306
MARIADB_USER=insight
MARIADB_PASSWORD=insight-local
MARIADB_DATABASE=analytics
CLICKHOUSE_HTTP_PORT=18123
CLICKHOUSE_USER=insight
CLICKHOUSE_PASSWORD=insight-local
CLICKHOUSE_DATABASE=insight
"""


def _env_file(tmp_path: Path, body: str = COMPLETE) -> Path:
    path = tmp_path / ".env.compose.test-stand-example"
    path.write_text(body.strip() + "\n", encoding="utf-8")
    return path


def test_the_named_env_file_decides_which_instance_is_written_to(tmp_path: Path) -> None:
    cfg = resolve_instance(environ={ENV_FILE_VAR: str(_env_file(tmp_path))})
    assert (cfg.ch_http_port, cfg.mariadb_port) == (18123, 13306)
    assert (cfg.ch_user, cfg.ch_database) == ("insight", "insight")
    assert cfg.source.endswith(".env.compose.test-stand-example")


def test_the_source_file_is_named_when_a_port_is_missing(tmp_path: Path) -> None:
    """Every instance shares one user and database name, so the file is the only
    thing that distinguishes the warehouse a failing run reached."""
    body = COMPLETE.replace("CLICKHOUSE_HTTP_PORT=18123", "")
    with pytest.raises(StandConnectionError, match="CLICKHOUSE_HTTP_PORT is missing") as refusal:
        resolve_instance(environ={ENV_FILE_VAR: str(_env_file(tmp_path, body))})
    assert ".env.compose.test-stand-example" in str(refusal.value)


@pytest.mark.parametrize("key", ["CLICKHOUSE_EXTERNAL", "MARIADB_EXTERNAL"])
def test_an_instance_whose_database_is_not_its_own_is_refused(tmp_path: Path, key: str) -> None:
    """This suite seeds and clears the warehouse; an external one belongs to someone else."""
    with pytest.raises(StandConnectionError, match=f"{key}=true"):
        resolve_instance(
            environ={ENV_FILE_VAR: str(_env_file(tmp_path, f"{COMPLETE}\n{key}=true"))}
        )


def test_a_port_that_is_not_a_number_is_refused(tmp_path: Path) -> None:
    body = COMPLETE.replace("MARIADB_PORT=13306", "MARIADB_PORT=not-a-port")
    with pytest.raises(StandConnectionError, match="is not a port"):
        resolve_instance(environ={ENV_FILE_VAR: str(_env_file(tmp_path, body))})


def test_no_env_file_names_everything_it_looked_for(tmp_path: Path) -> None:
    with pytest.raises(StandConnectionError, match="tried") as refusal:
        resolve_instance(repo_root=tmp_path, environ={})
    assert ".env.compose" in str(refusal.value)

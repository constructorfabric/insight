"""The assistant's ClickHouse account, asserted from the provisioning script.

`insight_v3_reader` is what the custom surfaces query as, and its whole point is
that it cannot write: every privilege arrives through `insight_v3_ro`, whose
read-only contract `test_ledger_grants.py` pins from the role SQL.

A privilege granted straight to the user would sit outside that contract and
outside that test — the role would still read as read-only while the account
querying with it could write. So this reads the script's statements and refuses
any grant to the user that is not the role itself.

The role reads every table, so it is provisioned only where insight-v3-core is
deployed (#3558). That gate is a deploy-time fact no SQL file can state, so it
is asserted here, at both ends: the deploy must consult it, and the chart must
supply it.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
REPO = Path(__file__).resolve().parents[4]
PROVISION_SCRIPT = SCRIPTS / "bootstrap-db" / "provision-v3-access.sh"
REVOKE_SCRIPT = SCRIPTS / "bootstrap-db" / "revoke-v3-access.sh"
ROLE_SQL = SCRIPTS / "bootstrap-db" / "v3-role.sql"
CREATE_DATABASES = SCRIPTS / "create-databases.sh"
MIGRATIONS = SCRIPTS / "apply-ch-migrations.sh"
CHART_HELPERS = REPO / "charts" / "insight" / "templates" / "_helpers.tpl"

READER_ROLE = "insight_v3_ro"
DEPLOY_FLAG = "INSIGHT_V3_CORE_ENABLED"

#: Anything that could change data. `ALL` covers `GRANT ALL PRIVILEGES`.
WRITE_PRIVILEGES = ("INSERT", "ALTER", "DROP", "TRUNCATE", "DELETE", "UPDATE", "CREATE TABLE", "ALL")


def statements() -> list[str]:
    """The script's SQL, one statement per entry, upper-cased."""
    body = PROVISION_SCRIPT.read_text()
    heredoc = re.search(r"run_ch <<SQL\n(.*?)\nSQL\n", body, re.S)
    assert heredoc, "the script no longer provisions through a run_ch heredoc"

    return [" ".join(statement.split()).upper() for statement in heredoc.group(1).split(";") if statement.strip()]


def role_grants() -> list[str]:
    """The role file's grants, one statement per entry, upper-cased."""
    lines = [line for line in ROLE_SQL.read_text().splitlines() if not line.strip().startswith("--")]
    statements = [" ".join(part.split()).upper() for part in " ".join(lines).split(";") if part.strip()]

    return [statement for statement in statements if statement.startswith("GRANT ")]


def test_the_reader_is_granted_the_read_only_role_and_defaults_to_it() -> None:
    granted = [s for s in statements() if s.startswith("GRANT ")]

    assert granted, "the reader is granted nothing, so it can read nothing"
    assert all(s.startswith(f"GRANT {READER_ROLE.upper()} TO ") for s in granted), (
        f"every grant must be the {READER_ROLE} role itself: {granted}"
    )
    assert any(s.startswith("ALTER USER") and f"DEFAULT ROLE {READER_ROLE.upper()}" in s for s in statements()), (
        f"the reader must default to {READER_ROLE}, or it queries with no role at all"
    )


def test_no_privilege_reaches_the_reader_directly() -> None:
    for statement in statements():
        if not statement.startswith("GRANT "):
            continue
        for privilege in WRITE_PRIVILEGES:
            assert privilege not in statement, (
                f"{privilege} is granted to the account itself, outside the read-only role: {statement}"
            )


def test_the_role_itself_only_reads() -> None:
    """The role is what the wildcard hangs off, so it is the one that must read
    and nothing more."""
    granted = role_grants()

    assert granted, f"{ROLE_SQL.name} grants the role nothing, so the assistant can read nothing"
    for grant in granted:
        privileges = grant[len("GRANT ") :].split(" ON ")[0]
        for privilege in WRITE_PRIVILEGES:
            assert privilege not in privileges, f"the assistant's role must not be able to write: {grant}"


def test_the_deploy_provisions_the_reader_only_where_the_assistant_runs() -> None:
    """A role that reads every table follows the service that needs it.

    Gating it is the whole decision (#3558): the grant stays `*.*` because an
    assistant metric may name any warehouse table and a connector adds a bronze
    database between deploys, so what an installation gets instead is an off
    switch.
    """
    deploy = CREATE_DATABASES.read_text()
    call = deploy.index(PROVISION_SCRIPT.name)
    guard = deploy.rindex("if [[", 0, call)

    assert DEPLOY_FLAG in deploy[guard:call], (
        f"{PROVISION_SCRIPT.name} must run behind {DEPLOY_FLAG}, or an installation "
        f"without the assistant still carries a role that reads every table"
    )
    assert CREATE_DATABASES.name in MIGRATIONS.read_text(), (
        f"{MIGRATIONS.name} no longer runs {CREATE_DATABASES.name}, so a deploy provisions nothing"
    )


def _statements_of(script: Path) -> str:
    """A shell script's executable lines. Its header explains which SQL it
    avoids, in the same words, so comments would match everything."""
    return "\n".join(line for line in script.read_text().splitlines() if not line.strip().startswith("#"))


def test_turning_the_assistant_off_takes_the_read_with_it() -> None:
    """Gating creation leaves an install that once ran the assistant holding a
    credential that reads every table, which is the thing being gated."""
    deploy = CREATE_DATABASES.read_text()
    call = deploy.index(PROVISION_SCRIPT.name)
    disabled = deploy[call:]

    assert REVOKE_SCRIPT.name in disabled, (
        f"the disabled branch only skips {PROVISION_SCRIPT.name}, so a reader "
        f"provisioned before the flag was turned off keeps SELECT on every table"
    )

    revoke = _statements_of(REVOKE_SCRIPT)
    assert "REVOKE ALL ON *.*" in revoke, f"{REVOKE_SCRIPT.name} must strip the wildcard grant"
    assert "CREATE ROLE" not in revoke, (
        f"{REVOKE_SCRIPT.name} must not probe with CREATE ROLE — it would create the role it exists to empty"
    )
    assert "DROP USER" not in revoke and "DROP ROLE" not in revoke, (
        f"{REVOKE_SCRIPT.name} removes the grant, not the identity: the deploy "
        f"did not create the user and cannot put it back next run"
    )


def test_a_strip_that_could_not_run_stops_the_deploy() -> None:
    """The role is absent and the role is unreadable produce the same empty
    answer, and only one of them means there is nothing left to take away."""
    result = subprocess.run(
        ["bash", str(REVOKE_SCRIPT)],
        capture_output=True,
        text=True,
        env={
            "CLICKHOUSE_URL": "http://127.0.0.1:1",
            "CLICKHOUSE_USER": "u",
            "CLICKHOUSE_PASSWORD": "p",
            "PATH": "/usr/bin:/bin",
        },
        check=False,
    )

    assert result.returncode != 0, (
        "the deploy reported success without stripping the grant, so a disabled "
        f"assistant keeps SELECT on every table: {result.stdout!r}"
    )
    assert "system.roles" in result.stderr, result.stderr


def test_the_chart_tells_the_deploy_whether_the_assistant_runs() -> None:
    """Unset, the flag reads false and the role is never created — so a chart
    that stops sending it turns the assistant off without saying so."""
    helpers = CHART_HELPERS.read_text()

    declaration = f"name: {DEPLOY_FLAG}"
    assert declaration in helpers, (
        f"the DDL Jobs no longer carry {DEPLOY_FLAG}, so the deploy cannot provision the reader"
    )

    value = helpers[helpers.index(declaration) :].splitlines()[1]
    assert "insightV3Core" in value, f"{DEPLOY_FLAG} must follow global.insightV3Core.deploy, not {value.strip()!r}"

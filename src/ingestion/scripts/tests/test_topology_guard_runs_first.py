"""The warehouse is checked against the declared topology before it is touched.

Three things a clustered install needs are properties of the server, not of
this release: Keeper, a replicated access backend, and databases that already
carry the replicated engine. None can be fixed by the deploy, and each fails
quietly rather than loudly — a role lands on one replica, a table replays its
DDL everywhere while replicating no rows, and `IF NOT EXISTS` keeps the second
deploy silent about all of it.

So the check runs first, or it is worth nothing: once a database is created
with the wrong engine, the flag no longer converts it.
"""

from __future__ import annotations

from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
CREATE_DATABASES = SCRIPTS / "create-databases.sh"
TOPOLOGY_LIB = SCRIPTS / "lib" / "ch-topology.sh"

GUARD = "ch_assert_topology"


def deploy() -> str:
    """The script's executable lines. Its header describes what it creates, in
    the same words the statements use, so comments would match everything."""
    lines = [line for line in CREATE_DATABASES.read_text().splitlines() if not line.strip().startswith("#")]

    return "\n".join(lines)


def test_the_guard_runs_before_anything_is_created_or_granted() -> None:
    text = deploy()

    assert GUARD in text, f"{CREATE_DATABASES.name} no longer verifies the topology it is about to emit"
    assert text.index(GUARD) < text.index("CREATE DATABASE"), (
        "a database created with the wrong engine is not converted by a later check"
    )
    assert text.index(GUARD) < text.index("provision-"), (
        "roles created without a replicated access backend reach one replica, "
        "and the deploy must refuse before it creates them"
    )


def test_the_guard_is_called_plainly() -> None:
    """`f || exit 1` turns `set -e` off inside `f`, so a probe that fails
    halfway would carry on to the next one and report the wrong remedy."""
    called = [line.strip() for line in deploy().splitlines() if line.strip().startswith(GUARD)]

    assert called, f"{GUARD} is never called"
    for call in called:
        assert "||" not in call and "if " not in call, f"the guard must stop the deploy: {call}"


def test_a_standalone_install_is_checked_too() -> None:
    """The flag off against replicated databases is the direction that loses
    data silently: every creator emits standalone DDL into them."""
    assert "_ch_refuse_replicated_databases" in TOPOLOGY_LIB.read_text(), (
        "turning the flag off against a replicated warehouse must fail, not proceed"
    )

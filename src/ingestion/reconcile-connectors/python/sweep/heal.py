"""Give rows recorded before the ledger carried identity the instance they ran on.

Until the ledger learned that one connector can be installed more than once, a
connector's rows were keyed by its name alone. Those rows are not ambiguous in
themselves — an install holding one instance of a connector recorded that
instance's syncs and no one else's — but nothing in the row says which, and the
read surface groups by identity: left empty they would stand as an instance of
their own beside the identified ones, and every connector would show twice.

The identity is looked for in order of how directly it is known:

1. the instance the install configures now, from its Secret;
2. the connector's OWN recorded data, which carries the identity it ran under;
3. failing both, the naming this install demonstrably follows — and only for a
   connector that never recorded a sync.

Rung 3 is the one place a value is inferred rather than read, so it is fenced
twice. It is licensed only by unanimity: every instance on this install whose
identity can be read must be named `<connector>-main` under one tenant, and a
single exception turns the rung off for the whole install. And it never touches
a connector with recorded syncs — there the identity decides whose work those
syncs were, and a wrong answer is indistinguishable from a right one afterwards.

Pure functions over values — no I/O. The ledger reads what is unidentified and
writes what this decides.
"""

from __future__ import annotations

from collections.abc import Iterable
from typing import NamedTuple

from .plan import Instance

#: What every instance is called on an install that follows one naming rule.
CONVENTIONAL_SUFFIX = "main"

FROM_SECRET = "its Secret"
FROM_RECORDED_DATA = "the identity its own data was recorded under"
FROM_INSTALL_NAMING = "this install's naming, unanimous across every instance it can read"


class Unidentified(NamedTuple):
    """A connector holding rows with no identity."""

    connector: str
    #: True when at least one of those rows records a sync.
    has_syncs: bool


class Claim(NamedTuple):
    """An identity to write, and what licensed it."""

    instance: Instance
    basis: str


class Unresolved(NamedTuple):
    """A connector whose rows keep an empty identity, and why."""

    connector: str
    reason: str


class Adoption(NamedTuple):
    claimed: list[Claim]
    unresolved: list[Unresolved]


def install_naming_rule(facts: Iterable[Instance]) -> str | None:
    """The tenant this install names every instance under, or None.

    None whenever anything contradicts one rule: no instance to learn from, more
    than one tenant, or a single instance named something other than
    `<connector>-main`. Fails closed on purpose — the rung it licenses writes a
    value nobody recorded, and one counter-example is enough to show the install
    does not work the way the rung would assume.
    """
    known = list(facts)
    if not known:
        return None
    tenants = {instance.tenant_id for instance in known}
    if len(tenants) != 1:
        return None
    if any(
        instance.source_id != f"{instance.connector}-{CONVENTIONAL_SUFFIX}"
        for instance in known
    ):
        return None
    return known[0].tenant_id


def plan_adoption(
    unidentified: Iterable[Unidentified],
    managed: Iterable[Instance],
    recovered: Iterable[Instance] = (),
) -> Adoption:
    """Which unidentified connector names can be given an identity, and on what.

    `managed` is what the Secrets configure now; `recovered` is what connectors
    recorded about themselves. Both are facts and both license the naming rule;
    only the first two rungs claim a connector directly.
    """
    # Counted off the list, not off the map it collapses into: a connector
    # installed twice must be refused, and two entries share one key.
    managed_per_connector: dict[str, int] = {}
    from_secret: dict[str, Instance] = {}
    for instance in managed:
        managed_per_connector[instance.connector] = (
            managed_per_connector.get(instance.connector, 0) + 1
        )
        from_secret[instance.connector] = instance
    from_data = {instance.connector: instance for instance in recovered}

    facts = [*from_secret.values(), *from_data.values()]
    rule_tenant = install_naming_rule(facts)

    claimed: list[Claim] = []
    unresolved: list[Unresolved] = []
    for entry in sorted(set(unidentified)):
        connector = entry.connector
        installed = managed_per_connector.get(connector, 0)
        if installed > 1:
            unresolved.append(
                Unresolved(
                    connector,
                    f"it is installed {installed} times and the rows do not say which ran them",
                )
            )
            continue
        if connector in from_secret:
            claimed.append(Claim(from_secret[connector], FROM_SECRET))
            continue
        if connector in from_data:
            claimed.append(Claim(from_data[connector], FROM_RECORDED_DATA))
            continue
        if entry.has_syncs:
            unresolved.append(
                Unresolved(
                    connector,
                    "it has recorded syncs and nothing left says which instance ran them",
                )
            )
            continue
        if rule_tenant is None:
            unresolved.append(
                Unresolved(
                    connector,
                    "nothing of it survives and this install does not name every "
                    "instance the same way, so nothing licenses inferring one",
                )
            )
            continue
        claimed.append(
            Claim(
                Instance(connector, rule_tenant, f"{connector}-{CONVENTIONAL_SUFFIX}"),
                FROM_INSTALL_NAMING,
            )
        )
    return Adoption(claimed, unresolved)

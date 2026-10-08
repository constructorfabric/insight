"""Which data-path legs a change can reach.

A spec observes a model only when bronze it seeds flows through that model, so a
changed model reaches the suites whose seeded sources are its ancestors — an M365
staging model never reaches the git specs, which seed no M365 bronze. Build breaks
in a model no seeded data crosses still surface: every leg runs the full non-gold
closure, and the meta tree runs on every partial plan.

Anything no rule recognises runs everything.
"""

from __future__ import annotations

from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass

from insight_datapath.dbt_graph import DbtGraph, closure
from insight_datapath.suite_scan import (
    DATAPATH,
    IDENTITY,
    IDENTITY_TREE,
    METRICS,
    SHARED_METRICS_DIRS,
    Suites,
)

META_TREE = f"{DATAPATH}/meta"
CONNECTORS = "src/ingestion/connectors/"
DBT_TREES = ("src/ingestion/dbt/", "src/ingestion/silver/", "src/ingestion/gold/", CONNECTORS)

#: Classes slow enough to hold a leg of their own; every other class shares `rest`.
SOLO_CLASSES = ("ai", "git", "tasks")
#: Read by no leg's stand, only as text by the meta tree.
META_ONLY_PATHS = frozenset({"src/ingestion/scripts/apply-ch-migrations.sh"})
#: Runs in every stand, but no data-path test calls it or reads what it writes.
UNREAD_SERVICE = "src/backend/services/insight-v3-core/"
#: The stand's identity and realm seed: a break fails every leg's logins alike,
#: and the identity leg logs in as the seeded operator and lead.
STAND_SEEDER = "src/ingestion/tools/seed/"


@dataclass(frozen=True)
class Full:
    reason: str


@dataclass(frozen=True)
class Partial:
    suites: frozenset[str]


Verdict = Full | Partial


@dataclass(frozen=True)
class Leg:
    shard: str
    trees: tuple[str, ...]
    metric_classes: tuple[str, ...]


@dataclass(frozen=True)
class Reach:
    graph: DbtGraph
    suites: Suites
    #: Suite -> every node its seeded data flows through, plus everything that
    #: shapes a union it crosses.
    flows: Mapping[str, frozenset[str]]


def _flow(
    roots: Iterable[str], edges: Mapping[str, Iterable[str]], graph: DbtGraph
) -> frozenset[str]:
    crossed = closure(roots, edges)
    members = {member for union in crossed & graph.unions.keys() for member in graph.unions[union]}
    return crossed | closure(members, graph.parents)


def reach(graph: DbtGraph, suites: Suites) -> Reach:
    no_gold = {key: value - graph.gold for key, value in graph.children.items()}
    flows: dict[str, frozenset[str]] = {}
    for suite, tables in suites.seeds.items():
        seeded = {
            graph.sources_by_relation[table]
            for table in tables
            if table in graph.sources_by_relation
        }
        edges = no_gold if suite == IDENTITY else graph.children
        flows[suite] = _flow(seeded | graph.shared_roots, edges, graph)
    return Reach(graph=graph, suites=suites, flows=flows)


def _reached_by(ids: Iterable[str], world: Reach) -> frozenset[str]:
    wanted = set(ids)
    return frozenset(suite for suite, flow in world.flows.items() if flow & wanted)


def _seeding(tables: Iterable[tuple[str, str]], world: Reach) -> frozenset[str]:
    wanted = set(tables)
    return frozenset(suite for suite, seeded in world.suites.seeds.items() if seeded & wanted)


def _datapath_rule(path: str, world: Reach) -> Verdict | None:
    if not path.startswith(f"{DATAPATH}/"):
        return None
    parts = path.split("/")

    if path.startswith(f"{IDENTITY_TREE}/"):
        return Partial(frozenset({IDENTITY}))
    if path.startswith(f"{META_TREE}/"):
        return Partial(frozenset())
    if not path.startswith(f"{METRICS}/") or len(parts) < 5:
        return Full(f"{path} is shared by every data-path suite")

    directory = parts[3]
    if directory == "schemas" and len(parts) == 5 and path.endswith(".yaml"):
        schema, _, table = parts[4].removesuffix(".yaml").partition(".")
        return Partial(_seeding([(schema, table)], world))
    if directory == "templates":
        return Partial(world.suites.template_users.get(path, frozenset()))
    if directory in SHARED_METRICS_DIRS:
        return Full(f"{path} is shared by every metric class")
    if directory not in world.suites.metric_classes:
        return Full(f"{path} is under metrics/{directory}, which is no longer a metric class")
    return Partial(frozenset({directory}))


def _dbt_rule(path: str, world: Reach) -> Verdict | None:
    graph = world.graph
    ids = graph.by_path.get(path, frozenset())
    macros = graph.macros_by_path.get(path, frozenset())
    if not ids and not macros:
        return Partial(frozenset()) if path in graph.inert_paths else None

    callers = frozenset(caller for macro in macros for caller in graph.macro_callers.get(macro, ()))
    if macros and not callers:
        return Full(f"{path} defines a macro no node calls, so dbt may call it implicitly")
    if (ids | callers) & graph.hooks:
        return Full(f"{path} feeds a hook every dbt invocation runs")
    return Partial(_reached_by(ids | callers, world))


def _connector_rule(path: str, world: Reach) -> Verdict | None:
    if not path.startswith(CONNECTORS):
        return None
    parts = path.split("/")
    if len(parts) < 6:
        return None
    if parts[5] == "dbt":
        return Full(
            f"{path} is a dbt file the manifest does not hold: deleted, renamed or unparsed"
        )

    connector = "/".join(parts[:5]) + "/"
    sources = [
        source for source, yml in world.graph.source_paths.items() if yml.startswith(connector)
    ]
    return Partial(_reached_by(sources, world))


def _markdown_rule(path: str, world: Reach) -> Verdict | None:
    if path.endswith(".md") and path.startswith(DBT_TREES):
        return Partial(frozenset())
    return None


def _meta_only_rule(path: str, world: Reach) -> Verdict | None:
    if path in META_ONLY_PATHS or path.startswith(UNREAD_SERVICE):
        return Partial(frozenset())
    return None


def _stand_seeder_rule(path: str, world: Reach) -> Verdict | None:
    return Partial(frozenset({IDENTITY})) if path.startswith(STAND_SEEDER) else None


RULES: tuple[Callable[[str, Reach], Verdict | None], ...] = (
    _datapath_rule,
    _dbt_rule,
    _connector_rule,
    _markdown_rule,
    _meta_only_rule,
    _stand_seeder_rule,
)


def _defines(graph: DbtGraph, path: str) -> bool:
    return path in graph.by_path or path in graph.macros_by_path


def _head_verdict(path: str, world: Reach) -> Verdict:
    for rule in RULES:
        verdict = rule(path, world)
        if verdict is not None:
            return verdict
    return Full(f"{path} is not mapped to a leg")


def verdict_for(path: str, world: Reach, previous: Reach | None = None) -> Verdict:
    """`previous` is the base revision: a dbt file is placed by what it fed before the
    change as well as after, so cutting a model off from seeded data still runs the
    suites it fed, and a deleted file is placed by what it defined."""
    if previous is None or not _defines(previous.graph, path):
        return _head_verdict(path, world)
    before = _dbt_rule(path, previous) or Partial(frozenset())
    if not _defines(world.graph, path):
        return before
    return combine([_head_verdict(path, world), before])


def combine(verdicts: Iterable[Verdict]) -> Verdict:
    suites: set[str] = set()
    for verdict in verdicts:
        if isinstance(verdict, Full):
            return verdict
        suites |= verdict.suites
    return Partial(frozenset(suites))


def plan_legs(verdict: Verdict, metric_classes: Sequence[str]) -> list[Leg]:
    """The legs to run; a partial plan always keeps `rest`, which carries the meta tree."""
    selected = {*metric_classes, IDENTITY} if isinstance(verdict, Full) else set(verdict.suites)

    legs = [
        Leg(shard=name, trees=(f"{METRICS}/{name}",), metric_classes=(name,))
        for name in SOLO_CLASSES
        if name in selected and name in metric_classes
    ]

    shared = tuple(name for name in metric_classes if name not in SOLO_CLASSES and name in selected)
    legs.append(
        Leg(
            shard="rest",
            trees=(*(f"{METRICS}/{name}" for name in shared), META_TREE),
            metric_classes=shared,
        )
    )

    if IDENTITY in selected:
        legs.append(Leg(shard=IDENTITY, trees=(IDENTITY_TREE,), metric_classes=()))
    return legs

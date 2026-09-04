"""In-process dbt runner: one long-lived runner, parse warmed once, build per test.

Uses dbt's programmatic entrypoint (`dbtRunner`) rather than shelling out to the
`dbt` CLI per build. A single runner is reused across the session, so every build
skips the interpreter cold-start and dbt/adapter import — the dominant cost of a
subprocess-per-build. Each build still runs its own parse, but `setup()` warms
dbt's partial-parse cache so those re-parses are cheap and, critically, every
build compiles from a fresh manifest — ephemeral models inject their CTEs anew
each time (dbt's ephemeral-CTE injection is a one-shot, in-place mutation, so a
shared/copied manifest drops a reused ephemeral model's CTE from a later build).

The profile is generated per instance rather than editing the project's own
`src/ingestion/dbt/profiles.yml`: dbt reads it from `--profiles-dir`, and the
instance it points at is the one the rest of the run seeds and asserts against.
"""

from __future__ import annotations

import json
import logging
import shutil
from collections.abc import Sequence
from pathlib import Path

import yaml

# dbt cannot share an environment with mypy — see the `datapath` dependency group.
from dbt.cli.main import dbtRunner  # type: ignore[import-not-found]

from insight_datapath import clickhouse as ch
from insight_datapath.instance import InstanceConfig

LOG = logging.getLogger("datapath.dbt")

# Where a model that configures no `schema` materializes. It is deliberately not
# `staging` or `silver`: such a model would land among the relations a spec's reset
# clears and be emptied under it.
PROFILE_SCHEMA = "default"


class DbtError(RuntimeError):
    pass


class DbtRunner:
    """Session-scoped in-process dbt runner."""

    def __init__(self, cfg: InstanceConfig, *, project_dir: Path, target_dir: Path) -> None:
        self.cfg = cfg
        self.dbt_project_dir = project_dir
        self.target_dir = target_dir
        self.profiles_dir = target_dir / "profiles"
        # One runner reused for every invocation; created in setup().
        self._runner: dbtRunner | None = None

    def setup(self) -> None:
        """One-time per session: write test profiles.yml + warm the parse cache."""
        self._write_profiles()
        self._runner = dbtRunner()
        self._parse()

    def build_closure(self) -> None:
        """Materialize every non-gold model once, over empty bronze.

        Relation existence becomes a session constant, so `union_by_tag`'s
        compile-time `adapter.get_relation` probe answers the same whatever ran
        before it. Without this a spec builds only its own slice, and a data test
        pulled in by indirect selection can reference an ephemeral model reading
        staging relations no spec ever created — which is a hard error, not a
        skip.

        `run`, not `build`: over empty bronze a uniqueness or completeness test
        has nothing to assert and several fail outright. Per-spec builds still
        use `build` on their own selection, so no model loses test coverage; it
        simply is not tested twice, once meaninglessly.
        """
        if self._runner is None:
            raise DbtError("dbt_runner.setup() must be called before build_closure()")
        LOG.info("dbt run --exclude tag:gold (closure build)")
        res = self._runner.invoke(
            [
                "run",
                "--exclude",
                "tag:gold",
                *self._base_flags(),
                *self._warm_parse_flags(),
            ]
        )
        if not res.success:
            failed = self._extract_failed_model_summary()
            raise DbtError(
                f"closure build failed\nfailed models: {failed}\nexception: {res.exception!r}"
            )

    def build(self, selector: str) -> None:
        """Build the selected models via the in-process runner.

        Reuses the session runner (no interpreter cold-start), and dbt's
        partial-parse cache — warmed in `setup()` — keeps this build's parse
        cheap. Each build parses fresh so ephemeral models inject their CTEs
        correctly. Raises DbtError on failure, with the failing model + compiled
        SQL excerpt from `run_results.json` surfaced in the message.
        """
        if self._runner is None:
            raise DbtError("dbt_runner.setup() must be called before build()")
        LOG.info("dbt build --select %s", selector)
        res = self._runner.invoke(
            [
                "build",
                "--select",
                selector,
                *self._base_flags(),
                *self._warm_parse_flags(),
                "--defer",
                "--state",
                str(self.target_dir),
            ]
        )
        if not res.success:
            failed = self._extract_failed_model_summary()
            raise DbtError(
                f"dbt build failed for selector {selector!r}\nfailed models: {failed}\nexception: {res.exception!r}"
            )

    def run(self, selector: str, *, full_refresh: bool = False) -> None:
        if self._runner is None:
            raise DbtError("dbt_runner.setup() must be called before run()")
        LOG.info("dbt run --select %s%s", selector, " --full-refresh" if full_refresh else "")
        res = self._runner.invoke(
            [
                "run",
                "--select",
                selector,
                *(["--full-refresh"] if full_refresh else []),
                *self._base_flags(),
                *self._warm_parse_flags(),
                "--defer",
                "--state",
                str(self.target_dir),
            ]
        )
        if not res.success:
            failed = self._extract_failed_model_summary()
            raise DbtError(
                f"dbt run failed for selector {selector!r}\nfailed models: {failed}\nexception: {res.exception!r}"
            )

    def derive_selectors(self, tables: set[tuple[str, str]]) -> tuple[list[str], list[str]]:
        """From the seeded bronze tables, find the dbt models to build.

        Returns (staging_models, silver_class_models). A staging model is any model
        whose `source(...)` is one of the seeded bronze tables; the silver targets
        are read off each staging model's `silver:<class>` tag. The caller builds
        `+<staging>` first (pulls `<connector>__bronze_promoted`), then the silver
        class models.
        """
        manifest_path = self.target_dir / "manifest.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        wanted = {f".{schema}.{table}" for schema, table in tables}
        wanted_sources = {
            source_id
            for source_id, source in manifest.get("sources", {}).items()
            if (source.get("schema"), source.get("identifier") or source.get("name")) in tables
        }
        existing_tables = set(
            ch.query(
                self.cfg, "SELECT database, name FROM system.tables WHERE database LIKE 'bronze_%'"
            )
        )
        available_sources = {
            source_id
            for source_id, source in manifest.get("sources", {}).items()
            if (source.get("schema"), source.get("identifier") or source.get("name"))
            in existing_tables | tables
        }
        nodes = manifest.get("nodes", {})
        children: dict[str, set[str]] = {}
        for node_id, node in nodes.items():
            for dependency in node.get("depends_on", {}).get("nodes", []):
                children.setdefault(dependency, set()).add(node_id)

        direct: set[str] = set()
        for node_id, node in nodes.items():
            if node.get("resource_type") != "model":
                continue
            deps = node.get("depends_on", {}).get("nodes", [])
            source_deps = {dependency for dependency in deps if dependency.startswith("source.")}
            matched_sources = {
                dependency
                for dependency in source_deps
                if dependency in wanted_sources
                or any(dependency.endswith(suffix) for suffix in wanted)
            }
            if matched_sources and source_deps <= available_sources:
                direct.add(node_id)

        staging_ids = set(direct)
        pending = list(direct)
        while pending:
            parent = pending.pop()
            for child in children.get(parent, set()):
                node = nodes.get(child, {})
                if node.get("resource_type") != "model":
                    continue
                if node.get("config", {}).get("schema") != "staging":
                    continue
                if child not in staging_ids:
                    staging_ids.add(child)
                    pending.append(child)

        staging: list[str] = []
        silver: set[str] = set()
        for node_id in staging_ids:
            node = nodes[node_id]
            staging.append(node["name"])
            for tag in node.get("tags", []):
                if tag.startswith("silver:"):
                    silver.add(tag.split(":", 1)[1])
        return sorted(set(staging)), sorted(silver)

    def materialized_relations(self, models: Sequence[str]) -> list[tuple[str, str]]:
        """`(schema, identifier)` that each named model materializes into.

        Read off the manifest rather than assumed by the caller, so a model
        configuring its own schema (`silver`, `identity`) is reported under the
        schema dbt actually writes to. Views and ephemeral models resolve to
        nothing: they hold no rows of their own.

        Raises rather than returning a short list. A caller feeds this the models
        it is about to build so their relations can be truncated afterwards, and
        a name that quietly resolved to nothing would leave those rows in place
        for the next test to read — the failure this exists to prevent.
        """
        manifest = json.loads((self.target_dir / "manifest.json").read_text(encoding="utf-8"))
        wanted = set(models)
        by_name = {
            node["name"]: node
            for node in manifest.get("nodes", {}).values()
            if node.get("resource_type") == "model" and node.get("name") in wanted
        }
        unknown = sorted(wanted - set(by_name))
        if unknown:
            raise DbtError(
                f"no dbt model is named {unknown} — the manifest knows nothing to build or truncate"
            )

        out: set[tuple[str, str]] = set()
        for name, node in by_name.items():
            if node.get("config", {}).get("materialized") in ("ephemeral", "view"):
                continue
            schema = node.get("schema")
            identifier = node.get("alias") or name
            if not schema:
                raise DbtError(f"model {name} resolved no target schema in the manifest")
            out.add((schema, identifier))
        return sorted(out)

    def ephemeral_silver_targets(self, tag: str) -> list[str]:
        """Silver class identifiers produced from an EPHEMERAL staging model tagged `tag`.

        `derive_selectors` only follows non-ephemeral staging whose `source()` is a
        seeded bronze table, so it misses silver targets fed by an enrich step: the
        enrich binary writes a `staging.*` table that a THIN EPHEMERAL view
        (e.g. `jira__task_field_history`, tag `jira` + `silver:class_task_field_history`)
        exposes to `union_by_tag`. Returns the `<class>` part of each `silver:<class>`
        tag on those ephemeral, connector-tagged models — exactly the silver tables the
        enrich output feeds (e.g. `class_task_field_history`). Generic: no per-connector
        hardcoding.
        """
        manifest = json.loads((self.target_dir / "manifest.json").read_text(encoding="utf-8"))
        out: set[str] = set()
        for n in manifest.get("nodes", {}).values():
            if n.get("resource_type") != "model":
                continue
            if n.get("config", {}).get("materialized") != "ephemeral":
                continue
            tags = n.get("tags", [])
            if tag not in tags:
                continue
            for t in tags:
                if t.startswith("silver:"):
                    out.add(t.split(":", 1)[1])
        return sorted(out)

    def enrich_output_tables(self, tag: str) -> list[tuple[str, str]]:
        """`(schema, table)` of the staging tables an enrich step WRITES.

        The enrich binary writes `staging.*` tables that are declared as dbt
        SOURCES (not models) and exposed to silver via a thin EPHEMERAL view
        tagged `tag` + `silver:<class>` (e.g. `jira__task_field_history` reading
        `source('staging_jira', 'jira__task_field_history')`). dbt never rebuilds
        these (they are sources), and the binary INSERTs (appends), so without
        explicit truncation their rows accumulate across tests and inflate
        absolute-count metrics (e.g. tasks_completed counted 10 instead of 2 when
        three jira tests ran back-to-back). Resolve them from the ephemeral
        models' `source()` dependencies so the caller can truncate per test.
        Generic: no per-connector hardcoding.
        """
        manifest = json.loads((self.target_dir / "manifest.json").read_text(encoding="utf-8"))
        sources = manifest.get("sources", {})
        out: set[tuple[str, str]] = set()
        for n in manifest.get("nodes", {}).values():
            if n.get("resource_type") != "model":
                continue
            if n.get("config", {}).get("materialized") != "ephemeral":
                continue
            tags = n.get("tags", [])
            if tag not in tags or not any(t.startswith("silver:") for t in tags):
                continue
            for dep in n.get("depends_on", {}).get("nodes", []):
                if not dep.startswith("source."):
                    continue
                src = sources.get(dep)
                if not src:
                    continue
                schema = src.get("schema")
                table = src.get("identifier") or src.get("name")
                if schema and table:
                    out.add((schema, table))
        return sorted(out)

    # ----------------------------------------------------------------------
    # internals
    # ----------------------------------------------------------------------

    def _warm_parse_flags(self) -> list[str]:
        """Skip the project file walk when the parse cache can supply the file set.

        dbt walks every `model-paths` entry without pruning, and one of them holds a
        connector's virtualenv, so the walk costs more than the build it precedes.
        Without the cache the flag makes dbt find zero models and report success,
        so it is only ever passed alongside one.
        """
        cache = self.target_dir / "partial_parse.msgpack"
        return ["--no-partial-parse-file-diff"] if cache.exists() else []

    def _base_flags(self) -> list[str]:
        """Flags shared by every invocation: which project, profile, target.

        `--project-dir` is required because the in-process runner inherits the
        pytest process's cwd (the e2e dir), not the dbt project dir a subprocess
        would `cd` into.
        """
        return [
            "--no-send-anonymous-usage-stats",
            "--profiles-dir",
            str(self.profiles_dir),
            "--project-dir",
            str(self.dbt_project_dir),
            "--target",
            "test",
            "--target-path",
            str(self.target_dir),
        ]

    def _write_profiles(self) -> None:
        self.profiles_dir.mkdir(parents=True, exist_ok=True)
        profiles = {
            "ingestion": {
                "target": "test",
                "outputs": {
                    "test": {
                        "type": "clickhouse",
                        # Derive from session config — `127.0.0.1` only works in
                        # host mode; in docker mode the runner reaches ClickHouse
                        # at the compose service name (`clickhouse`).
                        "host": self.cfg.ch_host,
                        "port": self.cfg.ch_http_port,
                        "schema": PROFILE_SCHEMA,
                        "user": self.cfg.ch_user,
                        "password": self.cfg.ch_password,
                        "secure": False,
                        # Match prod profile so models materialize identically
                        "engine": "ReplacingMergeTree(_version)",
                        "settings": {
                            "allow_nullable_key": 1,
                            # Correlated subqueries (LEFT ANTI JOIN in the identity
                            # seed models) are gated behind this experimental flag
                            # on CH 25.7. A model-level config() setting does NOT
                            # reach the SELECT plan in dbt-clickhouse, so it must be
                            # set at profile level. Parity with prod/bootstrap.
                            "allow_experimental_correlated_subqueries": 1,
                        },
                    }
                },
            }
        }
        (self.profiles_dir / "profiles.yml").write_text(yaml.safe_dump(profiles))
        LOG.debug("wrote test profiles.yml to %s", self.profiles_dir)

    def _parse(self) -> None:
        """Parse once at session start to warm dbt's partial-parse cache.

        Validates the project up front (fail fast) and writes both
        target/manifest.json — which `build --defer --state` reads to resolve
        unselected upstream refs — and target/partial_parse.msgpack, so each
        per-test build's parse is an incremental no-op rather than a full parse.
        """
        if self._runner is None:
            raise DbtError("dbt_runner.setup() must create the runner before _parse()")
        LOG.info("dbt parse (one-time, in-process)")
        res = self._runner.invoke(["parse", *self._base_flags()])
        if not res.success:
            raise DbtError(f"dbt parse failed: {res.exception!r}")
        manifest = self.target_dir / "manifest.json"
        if not manifest.exists():
            raise DbtError(f"dbt parse did not produce {manifest}")

    def _extract_failed_model_summary(self) -> str:
        """Read target/run_results.json and return a one-liner per failed model."""
        run_results = self.target_dir / "run_results.json"
        if not run_results.exists():
            return "(no run_results.json)"
        try:
            data = json.loads(run_results.read_text(encoding="utf-8"))
        except Exception as e:
            return f"(failed to parse run_results.json: {e})"
        failed = [
            f"  - {r.get('unique_id', '?')}: {r.get('message') or r.get('status')}"
            for r in data.get("results", [])
            if r.get("status") not in (None, "success", "pass")
        ]
        return "\n" + "\n".join(failed) if failed else "(none)"

    def cleanup(self) -> None:
        """Remove generated profiles + target. Called by session teardown."""
        if self.target_dir.exists():
            shutil.rmtree(self.target_dir, ignore_errors=True)

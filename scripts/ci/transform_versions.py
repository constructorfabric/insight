from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import cast

type Json = None | bool | int | float | str | list[Json] | dict[str, Json]

ROOT = Path(__file__).resolve().parents[2]
PROJECT = Path("src/ingestion/dbt")
INPUTS = (PROJECT, PROJECT.parent / "silver", PROJECT.parent / "gold", PROJECT.parent / "connectors")
DOCUMENTATION_KEYS = {"tags", "docs", "description", "persist_docs", "meta"}
TOKEN = re.compile(
    r"(?P<quoted>'(?:''|\\.|[^'\\])*'|\"(?:\"\"|\\.|[^\"\\])*\"|`(?:``|\\.|[^`\\])*`)"
    r"|(?P<comment>--[^\n]*|/\*.*?\*/|\{\#.*?\#\})"
    r"|(?P<space>\s+)|(?P<other>.)",
    re.DOTALL,
)
CONFIG_START = re.compile(r"\{\{[-+]?\s*config\s*\(")
CONFIG_END = re.compile(r"\s*[-+]?\}\}")


@dataclass(frozen=True)
class Model:
    name: str
    raw_sql: str
    sql: str
    config: dict[str, Json]
    version: int | None
    macros: tuple[str, ...]
    nodes: tuple[str, ...]
    tags: tuple[str, ...]


@dataclass(frozen=True)
class Macro:
    sql: str
    macros: tuple[str, ...]


@dataclass(frozen=True)
class Manifest:
    models: dict[str, Model]
    macros: dict[str, Macro]
    variables: dict[str, Json]


@dataclass(frozen=True)
class Finding:
    model: str
    before: int | None
    after: int | None
    changes: tuple[str, ...]

    def __str__(self) -> str:
        return f"{self.model}: transform_version {self.before} -> {self.after}: {', '.join(self.changes)}"


def config_end(text: str, start: int) -> int:
    depth = 1
    for token in TOKEN.finditer(text, start):
        if token.lastgroup != "other":
            continue
        if token.group() == "(":
            depth += 1
        elif token.group() == ")":
            depth -= 1
        if depth == 0:
            end = CONFIG_END.match(text, token.end())
            if end is not None:
                return end.end()
            break
    raise ValueError("cannot locate the end of a model config() expression")


def normalize(text: str, *, model_sql: bool = False) -> str:
    parts: list[str] = []
    position = 0
    while position < len(text):
        config = CONFIG_START.match(text, position) if model_sql else None
        if config is not None:
            position = config_end(text, config.end())
            if not parts or parts[-1] != " ":
                parts.append(" ")
            continue

        token = TOKEN.match(text, position)
        assert token is not None
        part = " " if token.lastgroup in {"comment", "space"} else token.group()
        if part != " " or not parts or parts[-1] != " ":
            parts.append(part)
        position = token.end()
    return "".join(parts).strip()


def object_value(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected a manifest object")
    return value


def string_list(values: Json) -> tuple[str, ...]:
    if not isinstance(values, list) or any(not isinstance(value, str) for value in values):
        raise ValueError("expected a manifest string list")
    return tuple(sorted(set(cast(list[str], values))))


def literal_calls(text: str, function: str) -> tuple[str, ...]:
    pattern = re.compile(rf"\b{re.escape(function)}\s*\(\s*(['\"])(.*?)\1\s*(?=[,)])", re.DOTALL)
    names: set[str] = set()
    for token in TOKEN.finditer(text):
        if token.lastgroup != "other":
            continue
        call = pattern.match(text, token.start())
        if call is not None:
            names.add(call.group(2))
    return tuple(sorted(names))


def contract_columns(raw_columns: Json) -> dict[str, Json]:
    columns: dict[str, Json] = {}
    for name, value in object_value(raw_columns).items():
        column = object_value(value)
        columns[name] = {key: column[key] for key in ("data_type", "constraints", "quote")}
    return columns


def read_manifest(data: dict[str, Json], variables: dict[str, Json]) -> Manifest:
    nodes = dict(object_value(data["nodes"]))
    if "disabled" in data:
        for name, variants in object_value(data["disabled"]).items():
            if not isinstance(variants, list) or len(variants) != 1:
                raise ValueError(f"{name}: expected one disabled node")
            nodes[name] = variants[0]

    models: dict[str, Model] = {}
    for name, value in nodes.items():
        node = object_value(value)
        if node["resource_type"] != "model":
            continue
        config = object_value(node["config"])
        meta = object_value(config["meta"])
        version = meta.get("transform_version")
        if "transform_version" in meta and (type(version) is not int or version <= 0):
            raise ValueError(f"{name}: transform_version must be a positive integer, got {version!r}")

        build = {key: value for key, value in config.items() if key not in DOCUMENTATION_KEYS}
        for key in ("schema", "alias", "database"):
            build[key] = node[key]
        if object_value(config["contract"])["enforced"]:
            build["constraints"] = node["constraints"]
            build["columns"] = contract_columns(node["columns"])
        raw_sql = normalize(str(node["raw_code"]))
        dependencies = object_value(node["depends_on"])
        models[name] = Model(
            name,
            raw_sql,
            normalize(str(node["raw_code"]), model_sql=True),
            build,
            cast(int | None, version),
            string_list(dependencies["macros"]),
            string_list(dependencies["nodes"]),
            string_list(node["tags"]),
        )

    macros: dict[str, Macro] = {}
    for name, value in object_value(data["macros"]).items():
        macro = object_value(value)
        dependencies = object_value(macro["depends_on"])
        macros[name] = Macro(normalize(str(macro["macro_sql"])), string_list(dependencies["macros"]))
    return Manifest(models, macros, variables)


def macro_closure(model: Model, macros: dict[str, Macro]) -> dict[str, str]:
    result: dict[str, str] = {}
    pending = list(model.macros)
    while pending:
        name = pending.pop()
        if name in result:
            continue
        macro = macros[name]
        result[name] = macro.sql
        pending.extend(macro.macros)
    return result


def contributors(model: Model, manifest: Manifest) -> tuple[str, ...]:
    tags = set(literal_calls(model.raw_sql, "union_by_tag"))
    return tuple(
        sorted(
            name
            for name, candidate in manifest.models.items()
            if name != model.name and candidate.config.get("enabled") is not False and tags.intersection(candidate.tags)
        )
    )


def variable_values(model: Model, macros: dict[str, str], variables: dict[str, Json]) -> dict[str, tuple[Json, ...]]:
    names = set(literal_calls(model.raw_sql, "var"))
    for body in macros.values():
        names.update(literal_calls(body, "var"))
    values: dict[str, tuple[Json, ...]] = {}
    for name in names:
        values[name] = (variables[name],) if name in variables else ()
    return values


def definition_changes(before: Model, after: Model, base: Manifest, head: Manifest) -> tuple[str, ...]:
    changes = ["sql"] if before.sql != after.sql else []
    if before.nodes != after.nodes:
        changes.append("dependencies")
    if contributors(before, base) != contributors(after, head):
        changes.append("contributors")
    for key in sorted(before.config.keys() | after.config.keys()):
        if before.config.get(key) != after.config.get(key):
            changes.append(f"config {key}")

    old_macros = macro_closure(before, base.macros)
    new_macros = macro_closure(after, head.macros)
    for name in sorted(old_macros.keys() | new_macros.keys()):
        if old_macros.get(name) != new_macros.get(name):
            changes.append(f"macro {name}")
    old_variables = variable_values(before, old_macros, base.variables)
    new_variables = variable_values(after, new_macros, head.variables)
    for name in sorted(old_variables.keys() | new_variables.keys()):
        if old_variables.get(name) != new_variables.get(name):
            changes.append(f"var {name}")
    return tuple(changes)


def compare(base: Manifest, head: Manifest) -> list[Finding]:
    versions = [model.version for model in (*base.models.values(), *head.models.values())]
    if base.models and head.models and all(version is None for version in versions):
        return []

    findings: list[Finding] = []
    for name, after in sorted(head.models.items()):
        before = base.models.get(name)
        if after.version is None:
            findings.append(Finding(name, before.version if before else None, None, ("missing transform_version",)))
            continue
        if before is None:
            continue

        before_version = 1 if before.version is None else before.version
        changes = definition_changes(before, after, base, head)
        if after.version < before_version:
            findings.append(Finding(name, before_version, after.version, (*changes, "version decreased")))
        elif changes and after.version == before_version:
            findings.append(Finding(name, before_version, after.version, changes))
    return findings


def git(root: Path, *arguments: str) -> str:
    return subprocess.run(["git", *arguments], cwd=root, check=True, capture_output=True, text=True).stdout.strip()


def snapshot(root: Path, destination: Path, revision: str | None) -> None:
    if revision is None:
        ignored = shutil.ignore_patterns("target", "logs", ".venv", "__pycache__", ".pytest_cache", ".ruff_cache")
        for path in INPUTS:
            shutil.copytree(root / path, destination / path, ignore=ignored)
        return

    archive = subprocess.run(["git", "archive", revision, *map(str, INPUTS)], cwd=root, check=True, capture_output=True)
    subprocess.run(["tar", "-xf", "-", "-C", str(destination)], input=archive.stdout, check=True)


def parse_snapshot(directory: Path, dbt: str) -> Manifest:
    import yaml

    project = yaml.safe_load((directory / PROJECT / "dbt_project.yml").read_text(encoding="utf-8"))
    variables = object_value(project["vars"]) if "vars" in project else {}
    profile: dict[str, Json] = {
        "transform_guard": {
            "target": "offline",
            "outputs": {
                "offline": {
                    "type": "clickhouse",
                    "host": "127.0.0.1",
                    "port": 1,
                    "schema": "default",
                    "user": "default",
                    "password": "",
                    "secure": False,
                }
            },
        }
    }
    (directory / "profiles.yml").write_text(json.dumps(profile), encoding="utf-8")
    environment = {
        **os.environ,
        "CLICKHOUSE_CLUSTER_MODE": "false",
        "DBT_SEND_ANONYMOUS_USAGE_STATS": "false",
        "DBT_TARGET_PATH": str(directory / PROJECT / "target"),
        "DBT_LOG_PATH": str(directory / "logs"),
    }
    result = subprocess.run(
        [
            dbt,
            "parse",
            "--no-partial-parse",
            "--project-dir",
            str(directory / PROJECT),
            "--profiles-dir",
            str(directory),
            "--profile",
            "transform_guard",
        ],
        env=environment,
        check=False,
        capture_output=True,
        text=True,
        timeout=180,
    )
    if result.returncode:
        raise ValueError(f"dbt parse failed:\n{result.stdout}\n{result.stderr}")
    data = json.loads((directory / PROJECT / "target/manifest.json").read_text(encoding="utf-8"))
    return read_manifest(data, variables)


def main() -> int:
    parser = argparse.ArgumentParser(description="Require a transform_version bump for changed dbt model definitions.")
    parser.add_argument("--compare-ref", required=True, help="compare against the merge base with this base revision")
    head = parser.add_mutually_exclusive_group(required=True)
    head.add_argument("--head-ref", help="head revision to inspect")
    head.add_argument("--working-tree", action="store_true", help="inspect local files, including uncommitted edits")
    parser.add_argument("--dbt", required=True, help="dbt executable with the ClickHouse adapter installed")
    args = parser.parse_args()
    started = time.monotonic()

    try:
        revision = None if args.working_tree else git(ROOT, "rev-parse", "--verify", args.head_ref)
        base = git(ROOT, "merge-base", args.compare_ref, revision or "HEAD")
        with tempfile.TemporaryDirectory(prefix="transform-versions-") as temporary:
            directory = Path(temporary)
            manifests: list[Manifest] = []
            for label, ref in (("base", base), ("head", revision)):
                destination = directory / label
                destination.mkdir()
                snapshot(ROOT, destination, ref)
                manifests.append(parse_snapshot(destination, args.dbt))
            findings = compare(*manifests)
    except (OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as error:
        sys.stderr.write(f"transform versions: error: {error}\n")
        return 1

    for finding in findings:
        sys.stderr.write(f"error: {finding}\n")
    sys.stdout.write(f"transform versions: {len(findings)} error(s), {time.monotonic() - started:.2f}s\n")
    return int(bool(findings))


if __name__ == "__main__":
    sys.exit(main())

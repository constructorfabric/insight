from __future__ import annotations

import hashlib
import json
import re
from collections.abc import Callable
from dataclasses import dataclass, replace
from typing import Any

from insight_datapath.caller import StandCaller
from insight_datapath.dataset_batches import content_tokens, overlap
from insight_datapath.fixture_loader import TestYaml
from insight_datapath.spec_runner import SpecRun

CONFIG_TABLES = frozenset(
    {
        "config.task_field_roles",
        "config.task_value_map",
        "config.field_value_map",
        "config.field_value_defaults",
    }
)

EMAIL = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")


@dataclass(frozen=True)
class Scenario:
    original: TestYaml
    seeded: TestYaml
    prefix: str
    sources: dict[str, str]

    def reader(self, prepared: SpecRun) -> SpecRun:
        to_person = {
            email.removeprefix(self.prefix): person
            for email, person in prepared.to_person_id.items()
            if email.startswith(self.prefix)
        }
        to_email = {
            person: email.removeprefix(self.prefix)
            for person, email in prepared.to_email.items()
            if email.startswith(self.prefix)
        }
        return replace(
            prepared,
            spec=self.original,
            caller=ScenarioCaller(prepared.caller, self.prefix, self.sources),
            to_person_id=to_person,
            to_email=to_email,
            _open=[],
        )


def map_strings(value: Any, transform: Callable[[str], str]) -> Any:
    if isinstance(value, str):
        return transform(value)
    if isinstance(value, list):
        return [map_strings(item, transform) for item in value]
    if isinstance(value, dict):
        return {key: map_strings(item, transform) for key, item in value.items()}
    return value


class ScenarioCaller(StandCaller):
    def __init__(self, caller: StandCaller, prefix: str, sources: dict[str, str]) -> None:
        self.caller = caller
        self.prefix = prefix
        self.sources = sources

    def call_request(self, request: dict[str, Any]) -> tuple[int, Any]:
        if request.get("body", {}).get("entity", {}).get("type") != "person":
            raise ValueError("shared datasets require explicitly selected person entities")

        def source_reference(value: str) -> str:
            for source, replacement in self.sources.items():
                if value == source or value.startswith(source + ":"):
                    return replacement + value[len(source) :]
            return value

        status, payload = self.caller.call_request(map_strings(request, source_reference))
        return status, map_strings(payload, lambda value: value.replace(self.prefix, ""))


def namespace(spec: TestYaml, *, key: str, caller_email: str) -> Scenario:
    if any(not table.startswith("bronze_") and table not in CONFIG_TABLES for table in spec.bronze):
        raise ValueError(f"{spec.name}: shared datasets cannot isolate configuration tables")
    prefix = "case-" + hashlib.sha256(key.encode()).hexdigest()[:16] + "-"
    sources = {}
    hr_ids = {}
    for table, rows in spec.bronze.items():
        for row in rows:
            if isinstance(row.get("source_id"), str) and row["source_id"]:
                sources[row["source_id"]] = prefix + row["source_id"]
            if table == "bronze_bamboohr.employees" and row.get("id"):
                hr_ids[str(row["id"])] = prefix + str(row["id"])

    for table, rows in spec.bronze.items():
        if table in CONFIG_TABLES and any(
            row.get("insight_source_id") not in sources for row in rows
        ):
            raise ValueError(f"{spec.name}: configuration must name a seeded source")
    source_pattern = (
        re.compile("|".join(re.escape(source) for source in sorted(sources, key=len, reverse=True)))
        if sources
        else None
    )

    def text(value: str) -> str:
        if value in sources:
            return sources[value]
        if value in hr_ids:
            return hr_ids[value]
        return EMAIL.sub(
            lambda match: (
                match[0] if match[0].lower() == caller_email.lower() else prefix + match[0]
            ),
            value,
        )

    def rewrite(value: Any, field: str = "") -> Any:
        if isinstance(value, dict):
            return {name: rewrite(item, name) for name, item in value.items()}
        if isinstance(value, list):
            return [rewrite(item, field) for item in value]
        if not isinstance(value, str):
            return value
        if field == "raw_data":
            try:
                return json.dumps(rewrite(json.loads(value)))
            except json.JSONDecodeError:
                return text(value)
        if field == "unique_key" and value and key.startswith("tasks/") and source_pattern:
            changed = source_pattern.sub(lambda match: sources[match[0]], value)
            return changed if changed != value else prefix + value
        if field in {"unique_key", "department", "read_id"} and value:
            return prefix + value
        return text(value)

    aliases = {
        text(person): [text(alias) for alias in alternates]
        for person, alternates in spec.identity_aliases.items()
    }
    accounts = [
        replace(entry, source_id=text(entry.source_id), person=text(entry.person))
        for entry in spec.identity_accounts
    ]
    seeded = replace(
        spec, bronze=rewrite(spec.bronze), identity_aliases=aliases, identity_accounts=accounts
    )
    return Scenario(spec, seeded, prefix, sources)


def combine(scenarios: list[Scenario]) -> TestYaml:
    if not scenarios:
        raise ValueError("a shared dataset needs at least one scenario")
    seen_tokens: set[str] = set()
    for scenario in scenarios:
        if scenario.original.path.parent.name != "git":
            continue
        tokens = content_tokens(scenario.original.bronze)
        if overlap(tokens, seen_tokens):
            raise ValueError(
                "Git scenarios with overlapping content identities need separate batches"
            )
        seen_tokens.update(tokens)
    bronze: dict[str, list[dict[str, Any]]] = {}
    schemas: dict[str, dict[str, Any]] = {}
    aliases: dict[str, list[str]] = {}
    accounts = []
    for scenario in scenarios:
        spec = scenario.seeded
        for table, rows in spec.bronze.items():
            bronze.setdefault(table, []).extend(rows)
            if table in schemas and schemas[table] != spec.schemas[table]:
                raise ValueError(f"incompatible fixture schemas for {table}")
            schemas[table] = spec.schemas[table]
        aliases.update(spec.identity_aliases)
        accounts.extend(spec.identity_accounts)
    return TestYaml(
        "shared",
        scenarios[0].original.path,
        bronze,
        schemas,
        identity_aliases=aliases,
        identity_accounts=accounts,
    )

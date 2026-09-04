"""One spec's data path on an instance, from seeded bronze to a caller that reads it.

The sequence mirrors the pipeline a connector drives in production: seed the spec's
bronze, build the staging models that read it, run any enrich step between the two
builds, build the silver classes above them, let the product mint the spec's people
and publish them, then build gold and ask for the metric as a seeded persona.

The identity inputs are rebuilt from scratch each time: the model admits only rows
above the version it already holds, and a stand's own seed has raised that past
anything a spec writes, so an incremental build would leave the previous spec's
people in place and mint against those.

A spec addresses people by email. The wire carries person ids, so requests translate
on the way out and responses on the way back, and a test asserts in the spec's terms.
"""

from __future__ import annotations

import logging
import re
from dataclasses import dataclass, field
from typing import Any

from insight_datapath.caller import StandCaller
from insight_datapath.ch_seeder import CHSeeder
from insight_datapath.dbt_runner import DbtRunner
from insight_datapath.enrich import EnrichRunner
from insight_datapath.fixture_loader import TestYaml
from insight_datapath.metric_expect import Ledger, MetricResponse
from insight_datapath.reset import clear
from insight_datapath.subjects import SubjectError, Subjects
from insight_datapath.tracked_models import TrackedModels

LOG = logging.getLogger("datapath.spec")

_EMAIL_TOKEN = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")

#: Silver models whose own dbt tests assert a completeness a single spec's seed
#: cannot satisfy, so they are materialized without running them.
_RUN_WITHOUT_TESTS = frozenset({"class_hr_working_hours"})


def all_persona_emails(
    bronze: dict[str, list[dict[str, Any]]], *, excluding: str = ""
) -> list[str]:
    """Every address of the spec's own cast.

    All of them, not only the ones a test requests: a peer pool is built from cohort
    members that are seeded as data and never asked for by name. `excluding` drops
    the caller, who appears in the spec's rows as the supervisor its people report to
    but belongs to the instance rather than to the spec.
    """
    found: set[str] = set()

    def walk(value: Any) -> None:
        if isinstance(value, str):
            found.update(_EMAIL_TOKEN.findall(value))
        elif isinstance(value, dict):
            for item in value.values():
                walk(item)
        elif isinstance(value, list):
            for item in value:
                walk(item)

    walk(bronze)
    return sorted(found - {excluding.strip().lower()})


def translate(value: Any, mapping: dict[str, str]) -> Any:
    """Deep-copy `value`, swapping every mapped string."""
    if isinstance(value, str):
        return mapping.get(value, mapping.get(value.strip().lower(), value))
    if isinstance(value, list):
        return [translate(item, mapping) for item in value]
    if isinstance(value, dict):
        return {key: translate(item, mapping) for key, item in value.items()}
    return value


@dataclass
class SpecRun:
    """A spec's built data path and the caller that reads it."""

    spec: TestYaml
    caller: StandCaller
    to_person_id: dict[str, str]
    to_email: dict[str, str]
    ledger: Ledger
    _test_name: str = ""
    _open: list[MetricResponse] = field(default_factory=list)

    def begin(self, test_name: str) -> None:
        self._test_name = test_name
        self._open = []

    def call(self, request: dict[str, Any]) -> MetricResponse:
        """Send `request` as the spec wrote it; the response reads in the spec's emails."""
        self.ledger.record_request(request.get("body") or {})
        status, payload = self.caller.call_request(translate(request, self.to_person_id))
        if status != 200:
            LOG.warning("HTTP %d; body: %r", status, payload)
        response = MetricResponse(
            status, translate(payload, self.to_email), test_name=self._test_name, ledger=self.ledger
        )
        self._open.append(response)
        return response

    def end(self) -> None:
        """Every row the test selected must have its view's required fields asserted."""
        opened, self._open = self._open, []
        for response in opened:
            response.check_complete()


def run_spec(
    spec: TestYaml,
    *,
    ch_seeder: CHSeeder,
    dbt_runner: DbtRunner,
    enrich_runner: EnrichRunner,
    subjects: Subjects,
    caller: StandCaller,
    caller_email: str,
    ledger: Ledger,
) -> SpecRun:
    """Seed, build and publish one spec; returns the caller its tests read through."""
    tracked = TrackedModels(dbt_runner, ch_seeder)
    clear(ch_seeder.cfg, ch_seeder.ledger.drain())
    ch_seeder.seed_bronze(spec.bronze, spec.schemas)

    staging, silver = dbt_runner.derive_selectors(spec.touched_tables)
    tracked.build(staging, with_ancestors=True)

    seeded_schemas = {schema for schema, _ in spec.touched_tables}
    ran_enrich_steps = []
    for step in enrich_runner.steps_for(seeded_schemas):
        source_ids = enrich_runner.discover_source_ids(step, spec.touched_tables)
        if not source_ids:
            continue
        clear(ch_seeder.cfg, dbt_runner.enrich_output_tables(step.name))
        enrich_runner.run(step, source_ids)
        ran_enrich_steps.append(step)

    silver_set = set(silver)
    for step in ran_enrich_steps:
        silver_set.update(dbt_runner.ephemeral_silver_targets(step.name))
    tracked.build(sorted(silver_set - _RUN_WITHOUT_TESTS))
    tracked.run(sorted(silver_set & _RUN_WITHOUT_TESTS))
    if "class_collab_meeting_activity" in silver_set:
        tracked.run(["class_focus_metrics"], full_refresh=True)

    tracked.run(["identity_inputs"], full_refresh=True)
    subjects.publish()
    emails = all_persona_emails(spec.bronze, excluding=caller_email)
    person_ids = subjects.person_ids(emails)
    unresolved = sorted({email.strip().lower() for email in emails} - set(person_ids))
    if unresolved:
        raise SubjectError(
            f"{spec.name}: identity resolved no person for {', '.join(unresolved)}. "
            "A spec's cast is minted from the HR rows it seeds, so an address without one "
            "never becomes a person the caller can read."
        )

    dbt_runner.run("tag:identity:map")
    dbt_runner.run("tag:gold")

    return SpecRun(
        spec=spec,
        caller=caller,
        to_person_id=person_ids,
        to_email={person_id: email for email, person_id in person_ids.items()},
        ledger=ledger,
    )

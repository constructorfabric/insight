from __future__ import annotations

import json
import os
import subprocess
import sys
from collections.abc import Mapping
from pathlib import Path

from acknowledge_connector_schedule import acknowledge, identity_matches, observation_matches
from read_connector_schedules import MAX_BYTES, MAX_ROWS
from schedule_database import connect
from schedule_failure import ScheduleFailure, failure_stage
from schedule_fields import render_schedule_fields

ACK_COLUMNS = (
    "connector",
    "source_id",
    "applied_revision",
    "applied_crons",
    "applied_timezone",
    "applied_origin",
    "suspended",
    "deployment_crons",
    "deployment_origin",
    "last_error",
    "observed_at",
)


def read_acknowledgements(env: Mapping[str, str]) -> dict[tuple[str, str], dict]:
    records = {}
    size = 0
    with connect(env) as connection, connection.cursor() as cursor:
        cursor.execute(
            f"SELECT {', '.join(ACK_COLUMNS)} FROM connector_schedule_acknowledgements LIMIT %s", (MAX_ROWS + 1,)
        )
        for row in cursor:
            record = dict(zip(ACK_COLUMNS, row, strict=True))
            size += sum(len(str(value)) for value in row)
            if len(records) >= MAX_ROWS or size > MAX_BYTES:
                raise ValueError("acknowledgements exceed limit")
            for column in ("applied_crons", "deployment_crons"):
                record[column] = json.loads(record[column]) if record[column] is not None else None
            records[(record["connector"], record["source_id"])] = record
    return records


def acknowledgement_matches(plan: dict, observed: dict, record: dict) -> bool:
    return (
        record.get("applied_revision") == plan["revision"]
        and record.get("applied_crons") == plan["crons"]
        and record.get("applied_timezone") == "UTC"
        and record.get("applied_origin") == plan["origin"]
        and record.get("suspended") == observed["spec"].get("suspend", False)
        and record.get("deployment_crons") == plan["deployment_crons"]
        and record.get("deployment_origin") == plan["deployment_origin"]
        and record.get("last_error") is None
        and record.get("observed_at") is not None
    )


def kubectl(namespace: str, stage: str, *args: str) -> dict:
    with failure_stage(stage):
        result = subprocess.run(
            ["kubectl", "-n", namespace, *args, "-o", "json"], capture_output=True, text=True, check=True, timeout=20
        )
        return json.loads(result.stdout)


def schedule_patch(plan: dict, observed: dict) -> list[dict]:
    fields = render_schedule_fields("\n".join(plan["crons"]), plan["origin"])
    patch = [
        {"op": "test", "path": "/metadata/uid", "value": observed["metadata"]["uid"]},
        {"op": "test", "path": "/metadata/resourceVersion", "value": observed["metadata"]["resourceVersion"]},
        {"op": "add", "path": "/spec/schedules", "value": fields["schedules"]},
        {"op": "add", "path": "/spec/timezone", "value": fields["timezone"]},
    ]
    if observed["metadata"].get("annotations") is None:
        patch.append({"op": "add", "path": "/metadata/annotations", "value": {}})
    patch.append(
        {
            "op": "add",
            "path": "/metadata/annotations/insight.cyberfabric.com~1schedule-origin",
            "value": fields["origin"],
        }
    )
    return patch


def apply_instance(env: Mapping[str, str], plan: dict, observed: dict | None, record: dict) -> bool:
    if observed is None:
        return acknowledge(env, {"plan": plan, "error": "next full pass creates the missing CronWorkflow"})
    if observation_matches(plan, observed) and acknowledgement_matches(plan, observed, record):
        return True

    error = None
    target = "cronworkflow.argoproj.io/" + plan["name"]
    try:
        if not observation_matches(plan, observed):
            kubectl(
                plan["namespace"],
                "cronworkflow patch",
                "patch",
                target,
                "--type=json",
                "-p",
                json.dumps(schedule_patch(plan, observed)),
            )
    except ScheduleFailure as exc:
        print(str(exc), file=sys.stderr)
        error = "schedule apply failed"
    try:
        observed = kubectl(plan["namespace"], "cronworkflow readback", "get", target) if error is None else {}
    except ScheduleFailure as exc:
        print(str(exc), file=sys.stderr)
        error = "schedule readback failed"
        observed = {}
    return acknowledge(env, {"plan": plan, "observed": observed, "error": error})


def apply_schedules(env: Mapping[str, str], plans: list[dict], workflows: dict) -> bool:
    existing = {item["metadata"]["name"]: item for item in workflows["items"]}
    prepared = []
    names = set()
    for plan in plans:
        if plan["name"] in names:
            raise ValueError("instances name the same CronWorkflow")
        names.add(plan["name"])
        fields = render_schedule_fields("\n".join(plan["crons"]), plan["origin"])
        plan = {**plan, "crons": fields["schedules"]}
        observed = existing.get(plan["name"])
        if observed is not None:
            plan["uid"] = observed["metadata"]["uid"]
            if not observed["metadata"]["resourceVersion"] or not identity_matches(plan, observed):
                raise ValueError("CronWorkflow identity mismatch")
        prepared.append((plan, observed))

    records = read_acknowledgements(env)
    succeeded = True
    for plan, observed in prepared:
        record = records.get((plan["connector"], plan["source_id"]), {})
        if not apply_instance(env, plan, observed, record):
            succeeded = False
    return succeeded


def main() -> int:
    try:
        with failure_stage("application"):
            plans = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines()]
            workflows = json.loads(Path(sys.argv[2]).read_text())
            return 0 if apply_schedules(os.environ, plans, workflows) else 1
    except ScheduleFailure as exc:
        print(str(exc), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

from __future__ import annotations

import json
import os
import sys
from collections.abc import Mapping
from datetime import UTC, datetime

from schedule_database import connect

PREFIX = "insight.cyberfabric.com/"


def identity_matches(plan: dict, observed: dict) -> bool:
    if not isinstance(observed, dict):
        return False
    metadata = observed.get("metadata", {})
    spec = observed.get("spec", {})
    if not isinstance(metadata, dict) or not isinstance(spec, dict):
        return False
    labels = metadata.get("labels", {})
    annotations = metadata.get("annotations", {})
    workflow = spec.get("workflowSpec", {})
    if not all(isinstance(value, dict) for value in (labels, annotations, workflow)):
        return False
    arguments = workflow.get("arguments", {})
    if not isinstance(arguments, dict):
        return False
    parameters = arguments.get("parameters", [])
    if not isinstance(parameters, list) or any(not isinstance(value, dict) for value in parameters):
        return False
    return (
        metadata.get("name") == plan["name"]
        and ("uid" not in plan or metadata.get("uid") == plan["uid"])
        and metadata.get("namespace") == plan["namespace"]
        and labels.get(PREFIX + "connector") == plan["connector"]
        and labels.get(PREFIX + "tenant") == plan["tenant"]
        and labels.get("app.kubernetes.io/component") == "connector-sync"
        and [p.get("value") for p in parameters if p.get("name") == "insight_source_id"] == [plan["source_id"]]
    )


def observation_matches(plan: dict, observed: dict) -> bool:
    if not identity_matches(plan, observed):
        return False
    spec = observed["spec"]
    return (
        spec.get("schedules") == plan["crons"]
        and spec.get("timezone") == "UTC"
        and observed["metadata"].get("annotations", {}).get(PREFIX + "schedule-origin") == plan["origin"]
        and type(spec.get("suspend", False)) is bool
    )


def acknowledge(env: Mapping[str, str], payload: dict) -> bool:
    plan = payload["plan"]
    error = payload.get("error")
    if error not in {
        None,
        "schedule apply failed",
        "schedule readback failed",
        "next full pass creates the missing CronWorkflow",
    }:
        error = "schedule apply failed"
    observed = payload.get("observed", {})
    if error is None and not observation_matches(plan, observed):
        error = "schedule readback mismatch"
    now = datetime.now(UTC).replace(tzinfo=None)
    columns = [
        "connector",
        "source_id",
        "deployment_crons",
        "deployment_origin",
        "attempted_revision",
        "attempted_at",
        "last_error",
    ]
    values = [
        plan["connector"],
        plan["source_id"],
        json.dumps(plan["deployment_crons"]),
        plan["deployment_origin"],
        plan["revision"],
        now,
        error,
    ]
    updates = ["attempted_revision", "attempted_at", "last_error"]
    if error is None:
        columns += [
            "applied_revision",
            "applied_crons",
            "applied_timezone",
            "applied_origin",
            "suspended",
            "observed_at",
        ]
        values += [
            plan["revision"],
            json.dumps(plan["crons"]),
            "UTC",
            plan["origin"],
            observed["spec"].get("suspend", False),
            now,
        ]
        updates = columns[2:]
    sql = (
        f"INSERT INTO connector_schedule_acknowledgements ({', '.join(columns)}) "
        f"VALUES ({', '.join(['%s'] * len(columns))}) ON DUPLICATE KEY UPDATE "
        + ", ".join(f"{column}=VALUES({column})" for column in updates)
    )
    with connect(env) as connection, connection.cursor() as cursor:
        cursor.execute(sql, tuple(values))
    return error is None


def main() -> int:
    try:
        payload = json.loads(sys.stdin.read(1024 * 1024 + 1))
        return 0 if acknowledge(os.environ, payload) else 1
    except Exception:
        print("schedule acknowledgement write failed", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())

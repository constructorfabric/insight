"""Emit one structured finding per data-quality check from dbt's artifacts.

Reads target/run_results.json and target/manifest.json relative to the cwd
(the dbt project dir), not dbt's logger, so each finding is one line in the
agreed shape — queryable as `| json | fields_event="data_quality_finding"`
(Loki flattens nested keys with `fields_`).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import insight_logging


def main() -> int:
    log = insight_logging.configure("dbt")

    run_results = json.loads(Path("target/run_results.json").read_text())
    nodes = json.loads(Path("target/manifest.json").read_text())["nodes"]
    run_id = run_results["metadata"].get("invocation_id")

    for result in run_results["results"]:
        node = nodes.get(result["unique_id"])
        if not node or node.get("resource_type") != "test":
            continue
        # `connector_quality` is the per-connector catalog: checks that need a
        # connector's own bronze/staging and so cannot live in `data_quality`,
        # which must pass on a tenant where that connector is absent. Findings
        # from both carry the same shape.
        if not ({"data_quality", "connector_quality"} & set(node.get("tags") or [])):
            continue

        config = node.get("config") or {}
        meta = config.get("meta") or {}
        level = log.info if result.get("status") == "pass" else log.warning
        level(
            "data quality finding",
            extra={
                "event": "data_quality_finding",
                "run_id": run_id,
                "check_id": node.get("name"),
                "title": meta.get("title", node.get("name")),
                "domain": meta.get("domain", "unknown"),
                "category": meta.get("category", "uncategorized"),
                "gate": config.get("severity"),
                "tier": meta.get("tier", "warn"),
                "status": result.get("status"),
                "rows_violating": result.get("failures") or 0,
                "duration_ms": round((result.get("execution_time") or 0) * 1000),
                "audit_relation": result.get("relation_name"),
                "remediation": meta.get("remediation"),
            },
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())

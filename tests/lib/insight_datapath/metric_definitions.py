"""The universe the coverage gate holds a run to: every builtin metric the product serves.

Read from the analytics catalogue at the end of a run, not written by hand, so a
metric that lands in the product without a spec is one the gate can name. The gate
compares this against the ledger of what the run asserted; a universe that was
missing would read as nothing to cover, which is the failure the gate exists to
catch, so a run that collects nothing refuses rather than writing an empty one.
"""

from __future__ import annotations

import json
from pathlib import Path

from insight_datapath import mariadb
from insight_datapath.instance import InstanceConfig

FILENAME = "metric_definitions.json"


def collect(cfg: InstanceConfig, out_dir: Path) -> Path:
    """Write the builtin metric catalogue to `out_dir`; returns the file written."""
    rows = mariadb.query(
        cfg,
        """
        SELECT
            d.metric_key,
            d.label,
            d.subject,
            d.computation_type,
            d.peer_cohort_key,
            GROUP_CONCAT(sd.dimension_key ORDER BY dd.display_order SEPARATOR ','),
            (
                SELECT GROUP_CONCAT(t.tag ORDER BY t.display_order SEPARATOR ',')
                FROM metric_definition_tags t
                WHERE t.metric_definition_id = d.id
            )
        FROM metric_definitions d
        LEFT JOIN metric_definition_dimensions dd ON dd.metric_definition_id = d.id
        LEFT JOIN metric_source_dimensions sd ON sd.id = dd.source_dimension_id
        WHERE d.tenant_id IS NULL
          AND d.origin = 'builtin'
          AND d.is_enabled = TRUE
        GROUP BY d.id, d.metric_key, d.label, d.subject, d.computation_type, d.peer_cohort_key
        ORDER BY d.metric_key
        """,
    )
    if not rows:
        raise RuntimeError("the analytics catalogue answered no builtin metrics")

    metrics = [
        {
            "metric_key": metric_key,
            "label": label,
            "subject": subject,
            "computation": computation,
            "peer_cohort_key": peer_cohort_key,
            "dimensions": dimensions.split(",") if dimensions else [],
            "tags": tags.split(",") if tags else [],
        }
        for metric_key, label, subject, computation, peer_cohort_key, dimensions, tags in rows
    ]

    out_dir.mkdir(parents=True, exist_ok=True)
    output = out_dir / FILENAME
    output.write_text(json.dumps({"metrics": metrics}, indent=2) + "\n", encoding="utf-8")
    return output

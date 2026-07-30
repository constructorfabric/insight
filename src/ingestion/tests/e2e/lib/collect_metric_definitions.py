from __future__ import annotations

import json
from pathlib import Path

from lib import mariadb
from lib.config import SessionConfig


def collect(cfg: SessionConfig, out_dir: str | Path) -> Path:
    rows = mariadb.query(
        cfg,
        """
        SELECT
            d.metric_key,
            d.label,
            d.computation_type,
            d.peer_cohort_key,
            GROUP_CONCAT(sd.dimension_key ORDER BY dd.display_order SEPARATOR ',')
        FROM metric_definitions d
        LEFT JOIN metric_definition_dimensions dd ON dd.metric_definition_id = d.id
        LEFT JOIN metric_source_dimensions sd ON sd.id = dd.source_dimension_id
        WHERE d.tenant_id IS NULL
          AND d.origin = 'builtin'
          AND d.is_enabled = TRUE
        GROUP BY d.id, d.metric_key, d.label, d.computation_type, d.peer_cohort_key
        ORDER BY d.metric_key
        """,
    )
    if not rows:
        raise RuntimeError("metric definition collection returned no builtin metrics")
    metrics = [
        {
            "metric_key": metric_key,
            "label": label,
            "computation": computation,
            "peer_cohort_key": peer_cohort_key,
            "dimensions": dimensions.split(",") if dimensions else [],
        }
        for metric_key, label, computation, peer_cohort_key, dimensions in rows
    ]
    output_dir = Path(out_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    output = output_dir / "metric_definitions.json"
    output.write_text(json.dumps({"metrics": metrics}, indent=2) + "\n", encoding="utf-8")
    return output

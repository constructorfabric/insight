from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

RECONCILE_DIR = Path(__file__).resolve().parents[1]
RENDERER = RECONCILE_DIR / "python" / "render_sync_trigger.py"
TEMPLATE = RECONCILE_DIR / "templates" / "sync-trigger.yaml.tpl"


def test_sync_trigger_omits_retired_enrich_parameter() -> None:
    result = subprocess.run(
        [
            sys.executable,
            str(RENDERER),
            "--connector",
            "jira",
            "--connection-name",
            "jira-main-example-conn",
            "--tenant",
            "example",
            "--insight-source-id",
            "main",
            "--dbt-select",
            "tag:silver,tag:jira+",
            "--bump-kind",
            "none",
            "--tpl",
            str(TEMPLATE),
        ],
        env={
            **os.environ,
            "INSIGHT_NAMESPACE": "insight",
            "ARGO_SERVICE_ACCOUNT": "insight-reconcile",
            "ARGO_INSTANCE_ID": "",
        },
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    assert "jira_enrich_image" not in result.stdout

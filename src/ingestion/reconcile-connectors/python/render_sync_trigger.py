#!/usr/bin/env python3
"""Render templates/sync-trigger.yaml.tpl with the given parameters.

CLI:
  render_sync_trigger.py
    --connector NAME
    --connection-name NAME
    --tenant SLUG
    --connector-dir PATH         # full path to connectors/<area>/<name>/
    --insight-source-id SLUG     # secret annotation insight.cyberfabric.com/source-id
    --tpl PATH

Stdout: rendered YAML (with metadata.generateName for one-shot create).
Exit:   0 success, 2 missing variables / unreadable descriptor.

The rendered Workflow targets `ingestion-pipeline` (sync → dbt-run, plus
tt-enrich-jira-run for jira), not the bare `airbyte-sync` template, so
data-affecting reconcile changes also rebuild Silver / class_* tables.
"""
import argparse, os, string, sys

import yaml  # type: ignore[import-not-found]


def _read_descriptor(connector_dir: str) -> dict:
    path = os.path.join(connector_dir, "descriptor.yaml")
    try:
        with open(path, "r", encoding="utf-8") as f:
            return yaml.safe_load(f) or {}
    except (OSError, yaml.YAMLError) as exc:
        print(f"render_sync_trigger: cannot read {path}: {exc}", file=sys.stderr)
        sys.exit(2)


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--connector", required=True)
    p.add_argument("--connection-name", required=True)
    p.add_argument("--tenant", required=True)
    p.add_argument("--connector-dir", required=True)
    p.add_argument("--insight-source-id", required=True)
    p.add_argument("--tpl", required=True)
    args = p.parse_args()

    desc = _read_descriptor(args.connector_dir)
    dbt_select = desc.get("dbt_select", "") or ""
    data_source = args.connector
    dbt_select_staging = "tag:jira-staging" if args.connector == "jira" else ""

    env = {
        "CONNECTOR": args.connector,
        "CONNECTION_NAME": args.connection_name,
        "TENANT": args.tenant,
        "INSIGHT_SOURCE_ID": args.insight_source_id,
        "DATA_SOURCE": data_source,
        "DBT_SELECT": dbt_select,
        "DBT_SELECT_STAGING": dbt_select_staging,
        "INSIGHT_NAMESPACE": os.environ["INSIGHT_NAMESPACE"],
        "ARGO_INSTANCE_ID": os.environ.get("ARGO_INSTANCE_ID", ""),
        "ARGO_SERVICE_ACCOUNT": os.environ["ARGO_SERVICE_ACCOUNT"],
    }
    with open(args.tpl, "r", encoding="utf-8") as f:
        tpl = f.read()
    try:
        rendered = string.Template(tpl).substitute(env)
    except KeyError as e:
        print(f"render_sync_trigger: missing variable {e}", file=sys.stderr)
        return 2
    if not env["ARGO_INSTANCE_ID"]:
        rendered = "\n".join(
            line for line in rendered.splitlines()
            if "workflows.argoproj.io/controller-instanceid" not in line
        ) + "\n"
    sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    sys.exit(main())

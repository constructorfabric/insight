#!/usr/bin/env python3
"""Render templates/cron-workflow.yaml.tpl with the given parameters.

CLI:
  render_cronworkflow.py
    --connector NAME
    --connection-name NAME
    --schedule "CRON"
    --tenant SLUG
    --connector-dir PATH         # full path to connectors/<area>/<name>/
    --insight-source-id SLUG     # secret annotation insight.cyberfabric.com/source-id
    --tpl PATH

Stdout: rendered YAML.
Exit:   0 success, 2 missing variables / unreadable descriptor.

Schedule precedence is resolved by the caller (Secret annotation >
descriptor.schedule > default).

`dbt_select` and `data_source` are sourced from the connector's
descriptor.yaml so the rendered CronWorkflow targets `ingestion-pipeline`
(sync → dbt-run, plus tt-enrich-jira-run for jira) instead of the bare
`airbyte-sync` template. Without this dispatch, Bronze rows would land
but Silver / class_* tables would never get rebuilt.
"""
import argparse, os, string, sys

import yaml  # type: ignore[import-not-found]


def _read_descriptor(connector_dir: str) -> dict:
    path = os.path.join(connector_dir, "descriptor.yaml")
    try:
        with open(path, "r", encoding="utf-8") as f:
            return yaml.safe_load(f) or {}
    except (OSError, yaml.YAMLError) as exc:
        print(f"render_cronworkflow: cannot read {path}: {exc}", file=sys.stderr)
        sys.exit(2)


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--connector", required=True)
    p.add_argument("--connection-name", required=True)
    p.add_argument("--schedule", required=True)
    p.add_argument("--tenant", required=True)
    p.add_argument("--connector-dir", required=True)
    p.add_argument("--insight-source-id", required=True)
    p.add_argument("--tpl", required=True)
    args = p.parse_args()

    desc = _read_descriptor(args.connector_dir)
    dbt_select = desc.get("dbt_select", "") or ""
    # The pipeline dispatches on `data_source == 'jira'` for the
    # enrich-then-silver path; everything else falls through to the
    # legacy single-dbt path. Set data_source = the connector slug so
    # operators can see what the workflow is for in Argo UI; only
    # `jira` triggers the enriched branch.
    data_source = args.connector
    dbt_select_staging = ""
    if args.connector == "jira":
        dbt_select_staging = "tag:jira-staging"

    env = {
        "CONNECTOR": args.connector,
        "CONNECTION_NAME": args.connection_name,
        "SCHEDULE": args.schedule,
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
        print(f"render_cronworkflow: missing variable {e}", file=sys.stderr)
        return 2
    # Drop the controller-instanceid label when the env var is empty —
    # otherwise we'd emit an empty label value, which Argo accepts but
    # loses meaning.
    if not env["ARGO_INSTANCE_ID"]:
        rendered = "\n".join(
            line for line in rendered.splitlines()
            if "workflows.argoproj.io/controller-instanceid" not in line
        ) + "\n"
    sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    sys.exit(main())

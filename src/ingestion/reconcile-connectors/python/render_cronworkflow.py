#!/usr/bin/env python3
"""Render templates/cron-workflow.yaml.tpl with the given parameters.

CLI:
  render_cronworkflow.py
    --connector NAME
    --connection-name NAME
    --schedule "CRON"
    --tenant SLUG
    --insight-source-id SLUG     # secret annotation insight.cyberfabric.com/source-id
    --dbt-select SEL             # descriptor.dbt_select (may be empty)
    --enrich-image REF           # descriptor.images.enrich.image (may be empty)
    --tpl PATH

Stdout: rendered YAML.
Exit:   0 success, 2 missing template variables.

Schedule precedence is resolved by the caller (Secret annotation >
descriptor.schedule > default).

The renderer is a pure stamp — it neither reads `descriptor.yaml` nor any
other file beyond the template. All descriptor-derived values are passed
by the bash caller (which already loads them via `disc_load_descriptors`),
keeping descriptor reading centralized in one place.

The rendered CronWorkflow targets `ingestion-pipeline` (sync → dbt-run,
plus tt-enrich-jira-run for jira) instead of the bare `airbyte-sync`
template. Without this dispatch, Bronze rows would land but Silver /
class_* tables would never get rebuilt.
"""
import argparse, os, string, sys


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--connector", required=True)
    p.add_argument("--connection-name", required=True)
    p.add_argument("--schedule", required=True)
    p.add_argument("--tenant", required=True)
    p.add_argument("--insight-source-id", required=True)
    p.add_argument("--dbt-select", default="")
    p.add_argument("--enrich-image", default="")
    p.add_argument("--tpl", required=True)
    args = p.parse_args()

    # The pipeline dispatches on `data_source == 'jira'` for the
    # enrich-then-silver path; everything else falls through to the
    # legacy single-dbt path. Set data_source = the connector slug so
    # operators can see what the workflow is for in Argo UI; only
    # `jira` triggers the enriched branch.
    data_source = args.connector
    dbt_select_staging = "tag:jira-staging" if args.connector == "jira" else ""

    env = {
        "CONNECTOR": args.connector,
        "CONNECTION_NAME": args.connection_name,
        "SCHEDULE": args.schedule,
        "TENANT": args.tenant,
        "INSIGHT_SOURCE_ID": args.insight_source_id,
        "DATA_SOURCE": data_source,
        "DBT_SELECT": args.dbt_select,
        "DBT_SELECT_STAGING": dbt_select_staging,
        "JIRA_ENRICH_IMAGE": args.enrich_image,
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

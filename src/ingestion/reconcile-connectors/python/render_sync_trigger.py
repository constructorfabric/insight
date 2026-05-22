#!/usr/bin/env python3
"""Render templates/sync-trigger.yaml.tpl with the given parameters.

CLI:
  render_sync_trigger.py
    --connector NAME
    --connection-name NAME
    --tenant SLUG
    --insight-source-id SLUG     # secret annotation insight.cyberfabric.com/source-id
    --dbt-select SEL             # descriptor.dbt_select (may be empty)
    --enrich-image REF           # descriptor.images.enrich.image (may be empty)
    --bump-kind KIND             # none|patch|minor|major|migration (default: none)
    --tpl PATH

Stdout: rendered YAML (with metadata.generateName for one-shot create).
Exit:   0 success, 2 missing template variables.

The renderer is a pure stamp — it neither reads `descriptor.yaml` nor any
other file beyond the template. All descriptor-derived values are passed by
the bash caller (which already loads them via `disc_load_descriptors`),
keeping descriptor reading centralized in one place.

The rendered Workflow targets `ingestion-pipeline` (sync → dbt-run, plus
tt-enrich-jira-run for jira), not the bare `airbyte-sync` template, so
data-affecting reconcile changes also rebuild Silver / class_* tables.

`--bump-kind=major` dispatches a one-shot `dbt --full-refresh` for the
connector's `dbt_select` scope. Any other value renders with
dbt_full_refresh=false.
"""
import argparse, os, string, sys


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--connector", required=True)
    p.add_argument("--connection-name", required=True)
    p.add_argument("--tenant", required=True)
    p.add_argument("--insight-source-id", required=True)
    p.add_argument("--dbt-select", default="")
    p.add_argument("--enrich-image", default="")
    p.add_argument(
        "--bump-kind",
        default="none",
        choices=["none", "patch", "minor", "major", "migration"],
    )
    p.add_argument("--tpl", required=True)
    args = p.parse_args()

    data_source = args.connector
    dbt_select_staging = "tag:jira-staging" if args.connector == "jira" else ""
    dbt_full_refresh = "true" if args.bump_kind == "major" else "false"

    env = {
        "CONNECTOR": args.connector,
        "CONNECTION_NAME": args.connection_name,
        "TENANT": args.tenant,
        "INSIGHT_SOURCE_ID": args.insight_source_id,
        "DATA_SOURCE": data_source,
        "DBT_SELECT": args.dbt_select,
        "DBT_SELECT_STAGING": dbt_select_staging,
        "DBT_FULL_REFRESH": dbt_full_refresh,
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

#!/usr/bin/env python3
"""Render templates/cron-workflow.yaml.tpl with the given parameters.

CLI:
  render_cronworkflow.py
    --connector NAME
    --connection-name NAME
    --schedule "CRON"
    --tenant SLUG
    --tpl PATH

Stdout: rendered YAML.
Exit:   0 success, 2 missing variables.

Schedule precedence is resolved by the caller (Secret annotation > descriptor.schedule > default `0 0 * * *`).
"""
import argparse, os, string, sys

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--connector", required=True)
    p.add_argument("--connection-name", required=True)
    p.add_argument("--schedule", required=True)
    p.add_argument("--tenant", required=True)
    p.add_argument("--tpl", required=True)
    args = p.parse_args()
    env = {
        "CONNECTOR": args.connector,
        "CONNECTION_NAME": args.connection_name,
        "SCHEDULE": args.schedule,
        "TENANT": args.tenant,
        "INSIGHT_NAMESPACE": os.environ.get("INSIGHT_NAMESPACE", "insight"),
    }
    with open(args.tpl, "r", encoding="utf-8") as f:
        tpl = f.read()
    try:
        sys.stdout.write(string.Template(tpl).substitute(env))
    except KeyError as e:
        print(f"render_cronworkflow: missing variable {e}", file=sys.stderr)
        return 2
    return 0

if __name__ == "__main__":
    sys.exit(main())

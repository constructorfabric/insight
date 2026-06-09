#!/usr/bin/env python3
"""
Audit: data-quality checks must read silver/gold only — never bronze.

WHY
  The data-quality catalog (tests tagged `data_quality`) runs on a schedule
  across every tenant. Bronze tables are per-connector and may be absent — a
  tenant without a given connector, or a stream that isn't synced — so a check
  that reads bronze ERRORs on those tenants (UNKNOWN_TABLE) instead of reporting
  a finding, and turns the scheduled run red on a non-issue.

  Silver and gold are present regardless of the connector set: silver class
  tables are created as empty placeholders and gold views are migration-created,
  per ADR-0007 (fresh-cluster-placeholders). So a DQ check that reads only
  silver/gold adapts to any connector combination automatically — an absent
  connector class yields an empty table and a clean pass, never an error.

  Therefore the invariant: a `data_quality`-tagged test MUST NOT reference the
  bronze layer. This script enforces it so the rule can't rot as checks are added.

WHAT IT FLAGS
  Any singular test carrying the inline `data_quality` tag whose SQL body
  references a bronze relation — `bronze_<connector>.<table>` or
  `source('bronze_…', …)`. Comments and the config() block (e.g. remediation
  prose) are ignored so only real table references count.

USAGE
  python3 audit_dq_no_bronze.py [path-to-dbt-dir]   # default: dir of this file
EXIT: non-zero if any data_quality check references bronze (usable as a CI gate).
"""
import re, glob, sys, os

ROOT = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))
os.chdir(ROOT)


def strip_comments(s):
    s = re.sub(r"\{#.*?#\}", " ", s, flags=re.S)
    s = re.sub(r"/\*.*?\*/", " ", s, flags=re.S)
    return re.sub(r"--[^\n]*", " ", s)


CONFIG_RE = re.compile(r"\{\{\s*config\((.*?)\)\s*\}\}", re.S)
TAG_RE = re.compile(r"tags\s*=\s*\[[^\]]*['\"]data_quality['\"]")
# A real bronze reference: schema-qualified `bronze_x.` or a bronze source().
BRONZE_RE = re.compile(r"\bbronze_\w+\s*\.|source\(\s*['\"]bronze_", re.I)

violations = []
for f in sorted(glob.glob("tests/**/*.sql", recursive=True)):
    text = strip_comments(open(f).read())
    cfg = CONFIG_RE.search(text)
    if not cfg or not TAG_RE.search(cfg.group(1)):
        continue  # not a data_quality check
    body = text[: cfg.start()] + text[cfg.end():]  # exclude the config() block
    if BRONZE_RE.search(body):
        violations.append(f)

if violations:
    print("data_quality checks must read silver/gold only — these reference bronze:")
    for v in violations:
        print(f"  - {v}")
    sys.exit(1)

print("OK: no data_quality check references bronze")

---
cypilot: true
type: project-rule
topic: conventions
generated-by: auto-config
version: 1.0
---

# Conventions

Document formatting and naming rules for the InsightSpec connector reference. Apply these whenever writing or reviewing spec documents.

<!-- toc -->

- [Section Headings](#section-headings)
- [Table Naming](#table-naming)
- [Table Column Format](#table-column-format)
- [Field Types](#field-types)
- [Identifiers](#identifiers)
- [Document Header](#document-header)
- [Open Questions](#open-questions)

<!-- /toc -->

## Section Headings

Use `## Source N: {Name} ({Category})` for every connector source section.

Evidence: `docs/CONNECTORS_REFERENCE.md:54` — e.g. `## Source 1: GitHub (Version Control)`

Use `## Unified Stream N: \`{name}\`` for unified cross-source streams.

Evidence: `docs/CONNECTORS_REFERENCE.md` — e.g. `## Unified Stream 1: \`class_communication_events\``

Use `### \`{table_name}\`` or `### \`{table_name}\` — {description}` for individual table sections.

Evidence: `docs/CONNECTORS_REFERENCE.md:333` — e.g. `### \`github_collection_runs\` — Connector execution log`

## Table Naming

Bronze tables: use `{source}_{entity}` format — e.g. `github_commits`, `youtrack_issue_history`.

Evidence: `docs/CONNECTORS_REFERENCE.md:42` — Bronze naming convention stated explicitly.

Silver tables: use `class_{domain}` format for all unified tables — e.g. `class_commits`, `class_task_tracker`.

Evidence: `docs/CONNECTORS_REFERENCE.md:43` — Silver steps 1 and 2 both use `class_` prefix.

Gold tables: use domain-specific derived names without prefix — e.g. `status_periods`, `throughput`.

Evidence: `docs/CONNECTORS_REFERENCE.md:45` — "domain-specific names — no raw events"

## Table Column Format

Always use exactly three columns: `| Field | Type | Description |`.

Evidence: all table definitions in `docs/CONNECTORS_REFERENCE.md` — no table uses a different column set.

## Field Types

Use these canonical types in the Type column: `String`, `Int`, `DateTime`, `String (JSON)`, `Text`, `timestamp`, `numeric`, `text`, `Int (bool)`.

Use `String (JSON)` for fields storing serialised JSON blobs.

Evidence: `docs/CONNECTORS_REFERENCE.md:179` — `| \`metadata\` | String (JSON) | Full API response |`

## Identifiers

Wrap all table names and field names in backticks inside prose and headings.

Evidence: consistent pattern throughout `docs/CONNECTORS_REFERENCE.md`.

## Document Header

The document title block uses: `Version N.NN — Month YYYY` on the second line, followed by `Based on: {PR/source references}`.

Evidence: `docs/CONNECTORS_REFERENCE.md:3–4`

Source category labels: `(Version Control)`, `(Task Tracking)`, `(Communication)`, `(AI Dev Tool)`, `(HR)`, `(CRM)`, `(Quality / Testing)`.

## Open Questions

Use `### OQ-N: {short description}` for unresolved schema questions under `## Open Questions`.

Evidence: `docs/CONNECTORS_REFERENCE.md:1569` — e.g. `### OQ-1: Git deduplication across sources`

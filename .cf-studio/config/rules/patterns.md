---
cypilot: true
type: project-rule
topic: patterns
generated-by: auto-config
version: 1.0
---

# Patterns

Recurring implementation patterns for documenting connectors and defining data source schemas. Apply when documenting a new connector or defining new data source tables.

<!-- toc -->

- [Source Section Structure](#source-section-structure)
- [Multi-Table Rationale](#multi-table-rationale)
- [Metadata Field](#metadata-field)
- [Identity Fields](#identity-fields)
- [Incremental Sync Cursor Fields](#incremental-sync-cursor-fields)
- [Cross-Source Comparison Tables](#cross-source-comparison-tables)
- [Monitoring Table Note](#monitoring-table-note)
- [Connector Descriptor Schema](#connector-descriptor-schema)

<!-- /toc -->

## Source Section Structure

Every source section must include, in order:
1. Bold **API:** line — REST/GraphQL endpoint, user identity fields (login, uuid, account_id), any structural notes
2. Optional bold **Why multiple tables:** or **Why three tables:** explanation (when source has 3+ entity tables)
3. Table definitions (`### \`{table_name}\``) in logical order: core entities first, join/detail tables second, collection_runs last

Evidence: `docs/CONNECTORS_REFERENCE.md:54–95` — GitHub source section structure.

## Multi-Table Rationale

Include a bold "Why multiple tables:" block whenever a source exposes 3+ entity tables with non-obvious relationships.

Explain the 1:N relationships that justify separate tables (e.g. a PR has many reviews, comments, and commits).

Evidence: `docs/CONNECTORS_REFERENCE.md:56–60` — GitHub "Why multiple tables" block.

## Metadata Field

Add `| \`metadata\` | String (JSON) | Full API response |` as the last field in every primary entity table.

This field stores the complete raw API response for forward compatibility.

Evidence: `docs/CONNECTORS_REFERENCE.md:179`, `221` — present in `github_repositories`, `github_commits`.

## Identity Fields

Include the source-native user identifier field in every table that references a user. Name it `{source}_user_id`, `login`, `uuid`, `account_id`, or the API's own field name — document as-is from the API.

These fields are later resolved to canonical `person_id` by the Identity Manager in Silver step 2.

Evidence: `docs/CONNECTORS_REFERENCE.md:22–26` — identity resolution pipeline.

## Incremental Sync Cursor Fields

Use `last_*` field names for incremental sync cursors. Document inline with `— cursor for incremental sync`.

Examples: `last_commit_hash` (position cursor), `last_checked_at` (time cursor), `ingestion_at`, `ingestion_date`.

Evidence: `docs/CONNECTORS_REFERENCE.md:191` — `| \`last_commit_hash\` | String | Last collected commit — cursor for incremental sync |`

## Cross-Source Comparison Tables

When a source replicates another source's table structure (e.g. Bitbucket mirrors GitHub's 9 tables), use a comparison table instead of repeating all field definitions:

```markdown
| Aspect | {Source A} | {Source B} |
|--------|------------|------------|
| {dimension} | {value} | {value} |
```

Evidence: `docs/CONNECTORS_REFERENCE.md:358–363` — GitHub vs Bitbucket comparison table.

## Monitoring Table Note

End every `{source}_collection_runs` table definition with a standalone paragraph:

`Monitoring table — not an analytics source.`

Evidence: `docs/CONNECTORS_REFERENCE.md:347`

## Connector Descriptor Schema

Every connector at `connectors/<area>/<name>/descriptor.yaml` must follow the formal schema below. `version` is the Insight semantic version (manifest config / dbt contract / audit). All connector images — CDK source images, enrich sidecars, future bootstrap/migrator containers — are declared under the map-style `images:` block per ADR-0016 (which supersedes ADR-0011 and ADR-0014). Top-level `cdk_image:` / `enrich_image:` fields are NOT allowed.

```yaml
name: <slug>                           # required, must equal directory name
type: nocode | cdk                     # required
version: "<semver-or-date>"            # required, Insight semantic version
                                       # for nocode: → declarativeManifest.description
                                       # for cdk:    metadata-only (audit, Argo labels)
images:                                # required when the connector ships ANY Dockerfile
                                       # map keyed by free-form ID; reserved keys with
                                       # runtime semantics: `cdk` (read by reconcile when
                                       # registering Airbyte CDK sources), `enrich` (read
                                       # by enrich workflow at submission time).
  <key>:                               # e.g. cdk, enrich, bootstrap
    name: <ghcr-short-name>            # GHCR image name without registry prefix or tag
    dockerfile: ./<path>               # path under connector dir, leading "./"
    context:    ./<path>               # build context under connector dir, leading "./"
    image:      "<full registry/repo:tag>"
                                       # full Docker image reference. May be empty string
                                       # "" for not-yet-published images (reconcile WARN+
                                       # skips when type=cdk and images.cdk.image is empty;
                                       # CI's first build patches the field).
schedule: "<cron>"                     # required
dbt_select: "<dbt selector>"           # required
workflow: "<sync|...>"                 # required
connection:
  namespace: "<bronze_xxx>"            # required
secret:
  required_fields:                     # required (may be empty list)
    - <key1>
    - <key2>
```

References: `docs/components/airbyte-toolkit/specs/ADR/0011-cdk-prebuilt-images.md`, `docs/components/airbyte-toolkit/specs/feature-reconcile/FEATURE.md` (DoD `cpt-insightspec-dod-reconcile-cdk-image-required`).

# YouTrack connector

This declarative connector collects the YouTrack task-tracking domain into
`bronze_youtrack`. Its dbt adapter supplies the shared task Silver classes; see [SILVER.md](SILVER.md) for history evidence, configuration and limitations.

## Configuration

Copy `credentials.yaml.example` into the gitignored
`src/ingestion/secrets/connectors/` directory and provide:

- `youtrack_base_url`, the service URL without `/api`;
- `youtrack_token`, a permanent token with access to the required projects and
  administration metadata;
- `youtrack_start_date`, the earliest date read by incremental streams.
- `youtrack_page_size` and `youtrack_activities_page_size`, optional request
  limits with defaults of 100 and 200.

The platform injects `insight_tenant_id` from the tenant configuration and
`insight_source_id` from the Secret annotation. They do not belong in
`stringData`.

The token determines visibility. Objects or activities hidden from the token
cannot be distinguished from absent data.

## Data captured

The package collects projects, users, field definitions, project field
settings, bundle values, agile boards, sprints, issue keys, issues, activities,
work items, comments, links, sprint memberships, and a full accessible issue
census.

Issue custom fields and polymorphic API values are stored as JSON strings so a
new field or value shape does not add dynamic Bronze columns. Field snapshots
retain `fieldType.id`, entity type, project settings, bundle metadata, and
observation time. The documented `fieldType.id` is retained verbatim, including
the `[*]` suffix that identifies multi-value fields. Bundle values are read
from their paginated resources as individual observations with project, field,
bundle, collection, and observation provenance. The project-field snapshot is
the bundle header, including for an empty bundle; Airbyte sync metadata provides
the full-refresh boundary for later history normalization. Metadata history
begins with the first successful connector sync.

System fields keep their documented scalar, object, or array shape inside the
entity JSON snapshots. Custom field values can be null, so their shape is
resolved later from the contemporaneous `fieldType.id` metadata instead of
guessing from a particular issue value.

Activities preserve every documented activity category and the
raw field, added, removed, and target shapes. YouTrack does not support
wildcards for response attributes or activity categories, so additions to the
public API require a connector update.

Snapshots and census records are observations only. Interpretation of missing
objects, access changes, or deletions belongs to a future Silver layer.

## Access and snapshot retention

Use a dedicated read-only service account restricted to the intended projects.
Grant only the metadata permissions needed by the selected streams; do not use
an administrator token as a shortcut. The connector has no project allowlist:
its collection boundary is the token's visibility. Restrict configuration access
and outbound destinations to the approved YouTrack service.

Custom-field, project-field, and bundle-value records are historical observations.
Their timestamp-bearing keys intentionally retain successive snapshots instead
of replacing them. Repeated reads can therefore add observations even when the
source value is unchanged. Do not remove the timestamp to deduplicate these
streams: later interpretation needs the contemporaneous field metadata.

The connector does not configure automatic expiration for these snapshots.
Before enabling scheduled collection, define a retention period and access policy
for the Bronze history in the warehouse. Include embedded entity JSON in that
policy; removing a top-level field does not remove its copy from a JSON snapshot.

## Full refresh cost

Work items use the paginated [`/api/workItems` collection](https://www.jetbrains.com/help/youtrack/devportal/resource-api-workItems.html).
This avoids listing every issue and making a separate work-item request per issue.
The stream deliberately reads all work items: `updated` can be null, and filtering
only by update time could omit records that have never been edited.

Sprint membership reads each issue's sprint collection only for issues whose
membership may have changed since the last sync. Two parents name those issues:
the `updated:` search over the incremental window, and the `SprintCategory`
activity feed over the same window. The second parent is required because a
sprint assignment does not advance the issue's `updated` timestamp, so the
search alone misses it. An issue both parents name is read twice into the same
`unique_key`. The first sync covers `youtrack_start_date` onward, so membership
of an issue untouched since then is not snapshotted — the same scope as the
other issue-scoped streams.

The per-issue request remains, so the cost follows the number of changed issues
in the window rather than the number of accessible issues.

Issue snapshots come from two streams with one record shape. `youtrack_issues`
is the `updated:` search. `youtrack_activity_issues` re-reads, in batches of 50
`idReadable` values, every issue the activity feed names in its incremental
window. It exists because some writes do not advance `updated` — sprint
assignments, and field writes by workflows and the system user — so the search
never returns those issues, and their history would have no snapshot to replay
from. The feed is read with target ids only, so its cost is one request per
activity page plus one search per batch.

## Local validation

From `src/ingestion` run:

```bash
./tools/declarative-connector/source.sh validate-strict task-tracking/youtrack
./tools/declarative-connector/source.sh validate task-tracking/youtrack
.venv/bin/pytest connectors/task-tracking/youtrack/tests
python3 ../../scripts/ci/connector_wiring.py
```

A live `check`, `discover`, and isolated read of every stream still requires a
non-production YouTrack instance and matching credentials.

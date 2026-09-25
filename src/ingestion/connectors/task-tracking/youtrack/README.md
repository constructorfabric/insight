# YouTrack connector

This declarative connector collects the YouTrack task-tracking domain into
`bronze_youtrack`. It intentionally contains no Silver transformations.

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
settings, bundle values, agile boards, sprints, issues, activities, work items,
comments, links, sprint memberships, and a full accessible issue census.

Issue custom fields and polymorphic API values are stored as JSON strings so a
new field or value shape does not add dynamic Bronze columns. Field snapshots
retain `fieldType.id`, entity type, project settings, bundle metadata, and
observation time. The documented `fieldType.id` is retained verbatim, including
the `[*]` suffix that identifies multi-value fields. Bundle values remain
grouped with their field and bundle identifiers so their cardinality and
provenance are not lost. Metadata history begins with the first successful
connector sync.

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

## Local validation

From `src/ingestion` run:

```bash
./tools/declarative-connector/source.sh validate-strict task-tracking/youtrack
./tools/declarative-connector/source.sh validate task-tracking/youtrack
.venv/bin/pytest connectors/task-tracking/youtrack/tests
python3 scripts/ci/connector_wiring.py
```

A live `check`, `discover`, and isolated read of every stream still requires a
non-production YouTrack instance and matching credentials.

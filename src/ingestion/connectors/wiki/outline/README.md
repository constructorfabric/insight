# Outline Connector

Extracts collection directory, document metadata, revision history, comments, and the user directory from the Outline (getoutline.com) RPC-style API using a Bearer API key. Works with both Outline Cloud and self-hosted instances.

## Prerequisites

1. Generate an API key in Outline: **Settings → API Keys → New API key** (the key inherits the permissions of the user who created it; use an admin or a member with read access to all target collections)
2. Note your instance URL (`https://app.getoutline.com` for cloud, or your self-hosted base URL)

## API Notes

- The Outline API is RPC-style: every endpoint is `POST {instance}/api/{method}` with a JSON body
- Pagination is `offset`/`limit` in the request body (server max `limit` is 100)
- Rate limiting returns `429` with a `Retry-After` header; the connector honours it with a 600s cap (same policy as the Confluence connector)

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-outline-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: outline
    insight.cyberfabric.com/source-id: outline-main
type: Opaque
stringData:
  outline_instance_url: "https://app.getoutline.com"
  outline_api_token: "CHANGE_ME"
  outline_start_date: "2020-01-01"
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `outline_instance_url` | Yes | Outline instance URL, no trailing slash, no `/api` suffix |
| `outline_api_token` | Yes | Outline API key (sensitive), sent as Bearer token |
| `outline_start_date` | No | Earliest date for incremental sync, YYYY-MM-DD (default: 2020-01-01) |
| `outline_page_size` | No | Results per API page (default: 100, server max: 100) |

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

Create `src/ingestion/secrets/connectors/outline.yaml` (gitignored) from the example:

```bash
cp src/ingestion/secrets/connectors/outline.yaml.example src/ingestion/secrets/connectors/outline.yaml
# Fill in real values, then apply:
kubectl apply -f src/ingestion/secrets/connectors/outline.yaml
```

## Streams

| Stream | Outline endpoint | Description | Sync Mode |
|--------|------------------|-------------|-----------|
| `wiki_spaces` | `collections.list` | Collection directory (id, name, status, URL) | Full Refresh |
| `wiki_pages` | `documents.list` | Document metadata (published + archived), incremental on `updated_at` | Incremental (client-side cursor) |
| `wiki_page_versions` | `revisions.list` | Revision history per document (substream of wiki_pages) | Full Refresh (per parent page) |
| `wiki_comments` | `comments.list` | All comments workspace-wide; threading via `parent_comment_id`, highlight anchoring via `anchor_text` | Full Refresh |
| `wiki_users` | `users.list` | Workspace user directory with emails (identity resolution) | Full Refresh |

### Mapping vs the Confluence connector

| Concept | Confluence | Outline |
|---------|-----------|---------|
| Space | space | collection |
| Page | page | document |
| Version | page version (numbered) | revision (UUID; ordinal derived in Silver via `row_number()`) |
| Comment kinds | 4 streams (footer/inline × top-level/replies) | 1 stream; reply = `parent_comment_id` set, inline = `anchor_text` non-empty |
| Page status | current / archived / trashed | current (published) / draft / archived / trashed (deleted) |
| User emails | not available (resolved via `jira_user` JOIN) | returned directly by the API (`wiki_users` + embedded `createdBy`/`updatedBy`) |

## Silver Targets

- `class_wiki_pages` — unified page metadata across Confluence and Outline
- `class_wiki_activity` — per-user per-day edit activity (derived from revisions, 30-min session collapse)
- `class_wiki_engagement` — per-page-day comment engagement metrics

## User Resolution

Emails are resolved within the connector itself — no cross-connector (`jira_user`) JOIN:

- `users.list` (the `wiki_users` stream) is the only Outline endpoint that exposes user emails.
- The user objects embedded in `documents.list` / `revisions.list` / `comments.list` responses (`createdBy`, `updatedBy`) do NOT include the email field on self-hosted instances (verified on a live instance, 2026-06-12). The connector still stamps `author_email` / `last_editor_email` from them onto bronze records in case a deployment does embed emails.
- The Silver staging models LEFT JOIN `bronze_outline.wiki_users` on the user UUID and take `coalesce(embedded_email, wiki_users.email)`. Identity Manager maps emails to `person_id` in Silver step 2.

### Identity Manager inputs

The user directory also feeds Identity Resolution through the standard macro chain (same as zoom / zulip-proxy / bamboohr / ms-entra):

```
wiki_users (bronze)
  -> outline__users_snapshot        (snapshot macro, SCD2 on name/email/role/is_suspended)
    -> outline__users_fields_history (fields_history macro, field-level change log)
      -> outline__identity_inputs    (identity_inputs_from_history macro,
                                      tagged silver:identity_inputs)
        -> identity.identity_inputs  (shared union via union_by_tag)
```

Contributed observations: `email`, `display_name` (from `name`), plus the canonical `id` binding row. Suspension (`is_suspended=true`) emits DELETE rows for all identity fields.

## Limitations

- Deleted (trashed) documents are not enumerated by `documents.list` `statusFilter`; they are only visible until the trash is emptied via the separate `documents.deleted` endpoint (not ingested)
- Drafts are NOT ingested: `statusFilter` is `[published, archived]`. Drafts are personal to the API key owner (other users' drafts are invisible), so including them would produce owner-dependent data; additionally, `statusFilter: [draft]` returns HTTP 500 on some self-hosted versions (observed on wiki.constr.dev, 2026-06-12)
- No view analytics (`views.list`, `documents.insights`) — deferred, mirroring the Confluence Phase 1 scope
- `comments.list` is called workspace-wide (no `documentId` filter); requires an Outline version that supports unfiltered listing (cloud and recent self-hosted releases do; verified live 2026-06-12)
- `wiki_page_versions` makes one `revisions.list` call per document — on a ~11k-document instance a full sync takes on the order of an hour at `default_concurrency: 4`. The other four streams are flat paginated lists and finish in minutes

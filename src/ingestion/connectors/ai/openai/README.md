# OpenAI Connector

Extracts organization users, per-endpoint API usage metrics, and cost data from the OpenAI Admin API using a Bearer token (Admin API key).

## Prerequisites

1. Go to [platform.openai.com/settings/organization/admin-keys](https://platform.openai.com/settings/organization/admin-keys)
2. Create an Admin API key with read access to organization data
3. The key must have permissions for: Users, Usage, and Costs endpoints

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-openai-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: openai
    insight.cyberfabric.com/source-id: openai-main
type: Opaque
stringData:
  openai_admin_api_key: "CHANGE_ME"
  openai_start_date: "2026-01-01"
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `openai_admin_api_key` | Yes | OpenAI Admin API key (from platform.openai.com) |
| `openai_start_date` | No | Earliest date to collect usage/costs (YYYY-MM-DD). Defaults to 90 days ago |

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

Create `src/ingestion/secrets/connectors/openai.yaml` (gitignored) from the example:

```bash
cp src/ingestion/secrets/connectors/openai.yaml.example src/ingestion/secrets/connectors/openai.yaml
# Fill in real values, then apply:
kubectl apply -f src/ingestion/secrets/connectors/openai.yaml
```

## Streams

| Stream | Description | Sync Mode |
|--------|-------------|-----------|
| `users` | Organization user roster (id, email, name, role) | Full Refresh |
| `usage_completions` | Chat/API completion usage (input/output/cached/audio tokens) | Incremental |
| `usage_embeddings` | Embedding usage (input tokens) | Incremental |
| `usage_moderations` | Moderation usage (input tokens) | Incremental |
| `usage_images` | Image generation usage (image count) | Incremental |
| `usage_audio_speeches` | Text-to-speech usage (characters) | Incremental |
| `usage_audio_transcriptions` | Speech-to-text usage (seconds) | Incremental |
| `usage_vector_stores` | Vector store usage (bytes) | Incremental |
| `usage_code_interpreter` | Code interpreter sessions | Incremental |
| `costs` | Organization costs by line item and project | Incremental |

All usage streams are grouped by `project_id`, `model`, and `user_id` (where supported) at daily bucket granularity.

## Silver Targets

- `class_ai_tool_usage` — Completions usage mapped to unified AI tool usage schema (provider=openai)
- `class_ai_cost` — Organization costs mapped to unified AI cost schema (provider=openai)

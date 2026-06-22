# GitLab Connector

Airbyte CDK source for self-hosted GitLab (REST API v4). Thin extractor: emits
source-native RECORD messages, transforms nothing downstream.

Design: [docs/components/connectors/git/gitlab/specs/DESIGN.md](../../../../../docs/components/connectors/git/gitlab/specs/DESIGN.md)

## Local development

Isolated environment via [uv](https://docs.astral.sh/uv/) — no global installs:

```bash
cd src/ingestion/connectors/git/gitlab
uv venv
uv pip install -e ".[dev]"

# spec
uv run python -m source_gitlab.source spec | jq .connectionSpecification.title

# check (fill real values first)
cp dev/config.example.json dev/config.json
uv run python -m source_gitlab.source check --config dev/config.json

# static checks
uv run mypy source_gitlab
uv run ruff check source_gitlab
```

The built image runs the same commands standalone (no Airbyte):

```bash
docker build -t source-gitlab .
docker run --rm -v "$PWD/dev:/dev" source-gitlab check --config /dev/config.json
```

## K8s Secret fields

| Field | Required | Description |
|-------|----------|-------------|
| `gitlab_url` | Yes | Self-hosted instance base URL |
| `gitlab_token` | Yes | Personal Access Token (`read_api`, `read_repository`, `read_user`) |
| `gitlab_groups` | No | Top-level group paths to collect. Empty = entire instance the token can access (use an admin/service token for full coverage) |
| `gitlab_projects` | No | Explicit project paths/IDs to add beyond the groups |
| `gitlab_start_date` | No | Backfill floor (YYYY-MM-DD) |

`insight_tenant_id` and `insight_source_id` are injected at reconcile time.

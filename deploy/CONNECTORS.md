# Configuring Insight Connectors

Connectors pull data from your tools — Jira issues, Slack messages, GitHub pull requests, and so on — into ClickHouse Bronze via Airbyte. Each connector is a single Kubernetes Secret; the reconcile loop discovers it automatically and provisions the matching Airbyte source and connection, no further steps required.

## Prerequisites

- A completed Insight install per the deployment runbook: [HELM_DEPLOY.md](./HELM_DEPLOY.md).
- The `insight-reconcile-loop` CronWorkflow present in the `insight` namespace (installed as part of that runbook).
- The `airbyte-auth-secrets` Secret mirrored into the `insight` namespace — done in Step 3 of the deployment runbook.

## Contents

<!-- toc -->

- [Prerequisites](#prerequisites)
- [Contents](#contents)
- [Anatomy of a connector Secret](#anatomy-of-a-connector-secret)
- [The 19 available connectors](#the-19-available-connectors)
- [Example Secret for every connector](#example-secret-for-every-connector)
  - [AI & coding assistants](#ai--coding-assistants)
  - [Source control & CI](#source-control--ci)
  - [Issue tracking & docs](#issue-tracking--docs)
  - [Communication & meetings](#communication--meetings)
  - [HR & identity](#hr--identity)
  - [CRM & support](#crm--support)
- [Troubleshooting](#troubleshooting)

<!-- /toc -->

## Anatomy of a connector Secret

Every connector Secret needs three things for the reconcile loop to discover and wire it up:

- **A label**, `app.kubernetes.io/part-of: insight` — the selector the reconcile loop uses to find connector Secrets.
- **Two annotations**: `insight.cyberfabric.com/connector: <name>` identifies which connector definition to use, and `insight.cyberfabric.com/source-id: <id>` names this specific source instance (the convention is `<name>-main`).
- **`stringData`** holding the connector's required fields — credentials, base URLs, and similar settings specific to that tool.

For example, the Jira connector Secret (`connectors/jira.yaml`) looks like this:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-jira-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: jira, insight.cyberfabric.com/source-id: jira-main }
type: Opaque
stringData:
  jira_instance_url: "https://your-org.atlassian.net"
  jira_email:        "svc@your-org.com"
  jira_api_token:    "ATATT-CHANGE_ME"
```

## The 19 available connectors

Replace `CHANGE_ME` (and any other placeholder) values in whichever connector files you need, under `connectors/`:

`jira`, `slack`, `github`, `gitlab`, `m365`, `zoom`, `confluence`, `zendesk`, `bamboohr`, `ms-entra`, `outline`, `hubspot`, `cursor`, `chatgpt-team`, `claude-team`, `claude-enterprise`, `bitbucket-cloud`, `zulip-proxy`, `github-directory`.

Apply all of them at once, or one at a time:

```sh
kubectl -n insight apply -f connectors/      # all 19 connectors at once
# or one at a time:
kubectl -n insight apply -f connectors/jira.yaml
```

You only need to create Secrets for the tools you actually use — an unused connector file can be left unfilled and simply not applied.

The reconcile loop scans the `insight` namespace about every 15 minutes. On a new or changed connector Secret, it provisions the matching Airbyte source and connection and starts syncing into Bronze automatically — no further steps once the Secret is applied and filled in correctly.

## Example Secret for every connector

Each block below is a complete, copy-paste-ready Secret for one connector. Fill in the `CHANGE_ME` (and any other placeholder) values, save it under `connectors/<name>.yaml`, and apply it as shown above.

One thing to know before you copy these:

- Connectors marked ⚠ are CDK connectors (built on Airbyte's Connector Development Kit). They bake their own `url_base` into the connector image, so they cannot be repointed at a mock or self-hosted endpoint.

### AI & coding assistants

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-chatgpt-team-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: chatgpt-team, insight.cyberfabric.com/source-id: chatgpt-team-main }
type: Opaque
stringData:
  chatgpt_account_id: "CHANGE_ME"
  proxy_url:          "CHANGE_ME"        # your ChatGPT admin-proxy base URL
  proxy_auth_token:   "CHANGE_ME"
  # chatgpt_org_id:   "CHANGE_ME"        # optional (subscription streams)
  # start_date:       "2026-01-01"       # optional
```

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-claude-team-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: claude-team, insight.cyberfabric.com/source-id: claude-team-main }
type: Opaque
stringData:
  claude_org_id:    "CHANGE_ME"
  proxy_url:        "CHANGE_ME"
  proxy_auth_token: "CHANGE_ME"
```

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-claude-enterprise-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: claude-enterprise, insight.cyberfabric.com/source-id: claude-enterprise-main }
type: Opaque
stringData:
  analytics_api_key: "CHANGE_ME"
```

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-cursor-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: cursor, insight.cyberfabric.com/source-id: cursor-main }
type: Opaque
stringData:
  cursor_api_key: "CHANGE_ME"
```

### Source control & CI

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-gitlab-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: gitlab, insight.cyberfabric.com/source-id: gitlab-main }
type: Opaque
stringData:
  gitlab_url:   "https://gitlab.com"
  gitlab_token: "CHANGE_ME"
```

```yaml
# ⚠ CDK connector; baked url_base
apiVersion: v1
kind: Secret
metadata:
  name: insight-bitbucket-cloud-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: bitbucket-cloud, insight.cyberfabric.com/source-id: bitbucket-cloud-main }
type: Opaque
stringData:
  bitbucket_token:      "CHANGE_ME"    # Atlassian ATCTT access token (NOT an ATATT API token)
  bitbucket_workspaces: "workspace-a,workspace-b"
```

```yaml
# GitHub org roster -> identity_inputs. Required for GitHub-brokered SSO:
# without it a GitHub login resolves to no person and the callback returns 403.
apiVersion: v1
kind: Secret
metadata:
  name: insight-github-directory-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: github-directory, insight.cyberfabric.com/source-id: github-directory-main }
type: Opaque
stringData:
  github_token:         "ghp_CHANGE_ME"   # read:org (+ user:email for member emails)
  github_organizations: '["myorg"]'       # JSON array
```

```yaml
# Declarative GitHub connector on the git-cli-proxy: commit-level data comes
# from a bare clone served by the proxy instead of one vendor API call per
# commit. Needs a reachable git-cli-proxy deployment, with git_proxy_token
# equal to the proxy's own configured token.
apiVersion: v1
kind: Secret
metadata:
  name: insight-github-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: github, insight.cyberfabric.com/source-id: github-main }
type: Opaque
stringData:
  github_token:         "ghp_CHANGE_ME"   # repo, read:org, read:project
  github_organizations: '["myorg"]'       # JSON array
  github_start_date:    "2026-01-01"
  git_proxy_url:        "http://insight-git-cli-proxy:8085"
  git_proxy_token:      "CHANGE_ME"       # must match the proxy's own Secret
```

### Issue tracking & docs

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-jira-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: jira, insight.cyberfabric.com/source-id: jira-main }
type: Opaque
stringData:
  jira_instance_url: "https://your-org.atlassian.net"
  jira_email:        "svc@your-org.com"
  jira_api_token:    "ATATT-CHANGE_ME"
```

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-confluence-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: confluence, insight.cyberfabric.com/source-id: confluence-main }
type: Opaque
stringData:
  confluence_instance_url: "https://your-org.atlassian.net/wiki"
  confluence_email:        "svc@your-org.com"
  confluence_api_token:    "ATATT-CHANGE_ME"
  confluence_start_date:   "2026-01-01"
```

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-outline-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: outline, insight.cyberfabric.com/source-id: outline-main }
type: Opaque
stringData:
  outline_instance_url: "https://your-outline-host"
  outline_api_token:    "CHANGE_ME"
  outline_start_date:   "2026-01-01"
```

### Communication & meetings

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-slack-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: slack, insight.cyberfabric.com/source-id: slack-main }
type: Opaque
stringData:
  slack_bot_token:  "xoxb-CHANGE_ME"
  slack_start_date: "2026-01-01"
```

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-zoom-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: zoom, insight.cyberfabric.com/source-id: zoom-main }
type: Opaque
stringData:
  zoom_account_id:   "CHANGE_ME"
  zoom_client_id:    "CHANGE_ME"
  zoom_client_secret: "CHANGE_ME"
```

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-m365-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: m365, insight.cyberfabric.com/source-id: m365-main }
type: Opaque
stringData:
  azure_tenant_id:     "CHANGE_ME"
  azure_client_id:     "CHANGE_ME"
  azure_client_secret: "CHANGE_ME"
```

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-zulip-proxy-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: zulip-proxy, insight.cyberfabric.com/source-id: zulip-proxy-main }
type: Opaque
stringData:
  zulip_proxy_base_url:   "CHANGE_ME"
  zulip_proxy_api_key:    "CHANGE_ME"
  zulip_proxy_start_date: "2026-01-01"
```

### HR & identity

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-bamboohr-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: bamboohr, insight.cyberfabric.com/source-id: bamboohr-main }
type: Opaque
stringData:
  bamboohr_api_key: "CHANGE_ME"
  bamboohr_domain:  "your-company"           # the <domain> in <domain>.bamboohr.com
```

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-ms-entra-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: ms-entra, insight.cyberfabric.com/source-id: ms-entra-main }
type: Opaque
stringData:
  azure_tenant_id:     "CHANGE_ME"
  azure_client_id:     "CHANGE_ME"
  azure_client_secret: "CHANGE_ME"
```

### CRM & support

```yaml
# ⚠ CDK connector; baked url_base (api.hubapi.com)
apiVersion: v1
kind: Secret
metadata:
  name: insight-hubspot-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: hubspot, insight.cyberfabric.com/source-id: hubspot-main }
type: Opaque
stringData:
  hubspot_access_token: "CHANGE_ME"
```

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-zendesk-main
  namespace: insight
  labels: { app.kubernetes.io/part-of: insight }
  annotations: { insight.cyberfabric.com/connector: zendesk, insight.cyberfabric.com/source-id: zendesk-main }
type: Opaque
stringData:
  zendesk_subdomain: "your-subdomain"        # <subdomain>.zendesk.com
  zendesk_email:     "agent@your-org.com"
  zendesk_api_token: "CHANGE_ME"
  # start_date:      "2026-01-01"            # optional
```

## Troubleshooting

| Problem | What to check |
|---------|-----------------|
| Connectors are not syncing | Confirm `airbyte-auth-secrets` was mirrored into the `insight` namespace (Step 3 of the deployment runbook, [HELM_DEPLOY.md](./HELM_DEPLOY.md)). The reconcile loop runs as an Argo `CronWorkflow` named `insight-reconcile-loop` — **not** the analytics pod — so inspect the Workflow pods it spawns: `kubectl -n insight get pods -l workflows.argoproj.io/cron-workflow=insight-reconcile-loop`, then `kubectl -n insight logs <pod>` (or `argo logs @latest -n insight` if the Argo CLI is available) |

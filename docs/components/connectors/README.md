# Connector Specifications

> Version 1.1 — March 2026

Per-source deep-dive specifications for Constructor Insight connectors. Each file expands on the corresponding source in [`../CONNECTORS_REFERENCE.md`](../CONNECTORS_REFERENCE.md) with full table schemas, identity mapping, Silver/Gold pipeline notes, and open questions.

<!-- toc -->

- [Index](#index)
  - [Version Control](#version-control)
  - [Task Tracking](#task-tracking)
  - [Collaboration](#collaboration)
  - [Wiki / Knowledge Base](#wiki--knowledge-base)
  - [Support / Helpdesk](#support--helpdesk)
  - [AI Dev Tools](#ai-dev-tools)
  - [AI Tools](#ai-tools)
  - [HR / Directory](#hr--directory)
  - [CRM](#crm)
  - [Design Tools](#design-tools)
  - [Quality / Testing](#quality--testing)
- [Unified Streams](#unified-streams)
- [How to Use](#how-to-use)

<!-- /toc -->

---

## Index

### Version Control

| Source | Spec | Status |
|--------|------|--------|
| Git (unified schema) | [`git/README.md`](git/README.md) | Draft |
| GitHub | [`git/github.md`](git/github.md) | Draft |
| Bitbucket | [`git/bitbucket.md`](git/bitbucket.md) | Draft |
| GitLab | [`git/gitlab.md`](git/gitlab.md) | Draft |

### Task Tracking

| Source | Spec | Status |
|--------|------|--------|
| Task Tracking (unified schema) | [`task-tracking/README.md`](task-tracking/README.md) | Draft |
| YouTrack | [`task-tracking/youtrack.md`](task-tracking/youtrack.md) | Proposed |
| Jira | [`task-tracking/jira.md`](task-tracking/jira.md) | Proposed |

### Collaboration

| Source | Spec | Status |
|--------|------|--------|
| Collaboration (unified schema) | [`collaboration/README.md`](collaboration/README.md) | Draft |
| Microsoft 365 | [`collaboration/m365.md`](collaboration/m365.md) | Proposed |
| Zulip | [`collaboration/zulip.md`](collaboration/zulip.md) | Proposed |
| Slack | [`collaboration/slack.md`](collaboration/slack.md) | Draft |
| Zoom | [`collaboration/zoom.md`](collaboration/zoom.md) | Draft |

### Wiki / Knowledge Base

| Source | Spec | Status |
|--------|------|--------|
| Wiki (unified schema) | [`wiki/README.md`](wiki/README.md) | Draft |
| Confluence | [`wiki/confluence.md`](wiki/confluence.md) | Draft |
| Outline | [`wiki/outline.md`](wiki/outline.md) | Draft |

### Support / Helpdesk

| Source | Spec | Status |
|--------|------|--------|
| Support (unified schema) | [`support/README.md`](support/README.md) | Draft |
| Zendesk | [`support/zendesk.md`](support/zendesk.md) | Draft |
| Jira Service Management | [`support/jsm.md`](support/jsm.md) | Draft |

### AI Dev Tools

| Source | Spec | Status |
|--------|------|--------|
| Cursor | [`ai/cursor.md`](ai/cursor.md) | Proposed |
| Windsurf | [`ai/windsurf.md`](ai/windsurf.md) | Proposed |
| GitHub Copilot | [`ai/github-copilot.md`](ai/github-copilot.md) | Proposed |
| JetBrains | [`ai/jetbrains.md`](ai/jetbrains.md) | Draft |

### AI Tools

| Source | Spec | Status |
|--------|------|--------|
| Claude Admin | [`ai/claude-admin/README.md`](ai/claude-admin/README.md) | Proposed |
| OpenAI API | [`ai/openai-api.md`](ai/openai-api.md) | Proposed |
| ChatGPT Team | [`ai/chatgpt-team.md`](ai/chatgpt-team.md) | Proposed |

### HR / Directory

| Source | Spec | Status |
|--------|------|--------|
| HR Directory (unified schema) | [`hr-directory/README.md`](hr-directory/README.md) | Draft |
| BambooHR | [`hr-directory/bamboohr.md`](hr-directory/bamboohr.md) | Proposed |
| Workday | [`hr-directory/workday.md`](hr-directory/workday.md) | Proposed |
| LDAP / Active Directory | [`hr-directory/ldap.md`](hr-directory/ldap.md) | Proposed |

### CRM

| Source | Spec | Status |
|--------|------|--------|
| CRM (unified schema) | [`crm/README.md`](crm/README.md) | Draft |
| HubSpot | [`crm/hubspot.md`](crm/hubspot.md) | Proposed |
| Salesforce | [`crm/salesforce.md`](crm/salesforce.md) | Proposed |

### Design Tools

| Source | Spec | Status |
|--------|------|--------|
| Design Tools (unified schema) | [`design/README.md`](design/README.md) | Draft |
| Figma | [`design/figma.md`](design/figma.md) | Draft |

### Quality / Testing

| Source | Spec | Status |
|--------|------|--------|
| Allure TestOps | [`allure.md`](allure.md) | Proposed |

---

## Unified Streams

| Stream | Sources | Spec |
|--------|---------|------|
| `class_communication_metrics` | M365 + Zulip + Slack + Zoom | [`collaboration/README.md`](collaboration/README.md) |
| `class_document_metrics` | M365 (OneDrive + SharePoint) | [`collaboration/README.md`](collaboration/README.md) — planned |
| `class_wiki_pages` | Confluence + Outline | [`wiki/README.md`](wiki/README.md) |
| `class_wiki_activity` | Confluence + Outline | [`wiki/README.md`](wiki/README.md) |
| `class_support_activity` | Zendesk + JSM | [`support/README.md`](support/README.md) |
| `class_design_activity` | Figma | [`design/README.md`](design/README.md) |
| Task Tracker unified schema | YouTrack + Jira | [`task-tracking/README.md`](task-tracking/README.md) |
| `class_people` + `class_org_units` | BambooHR + Workday + LDAP | [`hr-directory/README.md`](hr-directory/README.md) |
| `class_ai_dev_usage` | Cursor + Windsurf + Copilot + JetBrains + Claude Code | [`ai/`](ai/) |

---

## How to Use

- **Main reference** — [`../CONNECTORS_REFERENCE.md`](../CONNECTORS_REFERENCE.md) is the canonical index of all Bronze table schemas and the Bronze → Silver → Gold pipeline overview.
- **Per-source specs** (this directory) — expand on individual sources with additional detail: complete field lists, API notes, identity mapping, Silver channel mappings, and open questions.
- **Generate a new spec** — `/cypilot-generate Connector spec for {Source Name}`

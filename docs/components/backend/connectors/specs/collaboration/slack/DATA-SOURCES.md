# Slack data sources

This document lists the data that Slack supplies through its APIs.
The column "In Insight" shows what Insight reads now.

The data comes from the Slack documentation (November 2026).
Nobody tested these APIs on a real Slack organization.
"Unconfirmed" means that the documentation does not show this item clearly.

For the connector that Insight uses now, refer to [SPEC.md](SPEC.md).

## 1. General rules

### 1.1 Plans

| Short name | Slack plan |
|---|---|
| Any | All plans, Free included |
| Paid | Pro, Business+ or Enterprise |
| B+ | Business+ or Enterprise |
| Enterprise | Enterprise Grid or Enterprise+ |

Most of the data in this document needs the Enterprise plan.

The method `team.billing.info` (scope `team.billing:read`) gives the plan of a workspace:

| Value | Plan |
|---|---|
| `free` | Free |
| `std` | Pro |
| `plus` | Business+ |
| `enterprise` | Enterprise |
| `compliance` | Enterprise Compliance Select |

The connector reads only the data that the plan permits (#4348).

On Free and Pro, `admin.analytics.getFile` is not available. Insight gets no Slack activity on these plans now (#4349).

### 1.2 Tokens

| Token | Use |
|---|---|
| User token (`xoxp-…`) | Admin, analytics and audit APIs. An admin or an owner installs the app. Some APIs need an installation by the Org Owner at the organization level. |
| Bot token (`xoxb-…`) | Messages, threads, reactions and events. The bot must be a member of each private channel that it reads. |

The app does not need a bot user for the admin, analytics and audit APIs.

### 1.3 Limit on message history (2025)

Slack decreased the rate limit of `conversations.history` and `conversations.replies`:

- The limit is 1 request for each minute and 15 messages for each request.
- The limit applies to apps that a vendor distributes outside the Slack Marketplace.
- The limit applies to new installations from 2025-05-29.
- The limit applies to existing installations from 2026-03-03. One Slack page tells that existing installations are not affected. The sources do not agree.
- The limit does not apply to internal apps that a customer installs in its own organization.
- The limit does not apply to apps in the Slack Marketplace.

Each customer installs the Insight app in its own organization. Thus the limit does not apply to Insight.

### 1.4 Message text

These sources send the message text: message history, thread replies, search and message events.
No parameter or scope removes the text.
If Insight reads these sources, the connector must remove `text`, `blocks` and `attachments` before it writes to bronze.

The API `admin.analytics.messages.metadata` (Enterprise) does not send the text.

## 2. Data sources

"In Insight" values:

- **Yes**: Insight reads this data now.
- **Planned (#N)**: An issue gives this work.
- **No**: Insight does not read this data. No issue exists.

### 2.1 Member activity

| Source | Data | Plan | In Insight |
|---|---|---|---|
| `admin.analytics.getFile type=member` | For each member, for each day: messages, channel messages, reactions, files, calls, huddles, searches. Active flags for each client. Guest flag, billable flag, email. History of 13 months. | B+ | **Yes.** Insight reads all fields. Metrics use messages and channel messages. |
| Same file, field `slack_huddles_count` | Number of huddles for each member, for each day. No duration. No participants. | B+ | Planned (#4321). Insight stores the field. Metrics do not use it. |
| Same file, flags `is_active_workflows`, `is_active_apps` | The member used workflows or apps on that day. | B+ | No. Insight stores the flags. Metrics do not use them. |

### 2.2 Members and organization

| Source | Data | Plan | In Insight |
|---|---|---|---|
| `users.list` | Member ID, full name, display name, deactivated flag, guest flags, bot flag. Also time zone and status. The email needs the scope `users:read.email`. | Any | Planned (#4341) |
| `usergroups.list`, `usergroups.users.list` | User groups and their members | Any | Planned (#4356) |
| SCIM API | Title, department, manager, cost center, division. Groups and members. | B+ | Planned (#4356) |
| `admin.users.list` | Roles, guest flags, date of creation, date of deactivation, 2FA, SSO | Enterprise | No |

### 2.3 Channels

| Source | Data | Plan | In Insight |
|---|---|---|---|
| `admin.analytics.getFile type=public_channel` | For each public channel, for each day: messages, messages from members (no bots), posters, viewers, reactions, members, guests, external organizations. History of 13 months. Archived channels are not included. | B+ | Planned (#4350) |
| `admin.conversations.search` | Public and private channels: members, date of creation, creator, last activity, sharing flags | Enterprise | No |
| `conversations.list`, `conversations.info` | Channels, DMs and group DMs: date of creation, members, archived flag, external flag | Any | No |

The Public Channel Management permission is necessary for the channel file. By default, only the Primary Owner has this permission.

### 2.4 Messages and threads

| Source | Data | Plan | In Insight |
|---|---|---|---|
| `admin.analytics.messages.metadata` | For each message: author, time, thread, subtype, length, reactions, files. No text. | Enterprise | Planned (#4354) |
| `admin.analytics.messages.activity` | For each message: unique viewers, reactors and clickers. Views for each client. Data for each department and team. | Enterprise | No |
| `conversations.history`, `conversations.replies` | Messages and thread replies with text, time, author, subtype, reactions, reply count | Any. The Free plan shows 90 days. | Planned (#4349, #4351). Metadata only. |
| Events API: `message.*`, `reaction_added`, `member_joined_channel` | Real-time messages, reactions and channel membership. The data starts at installation. | Any | No |
| `search.messages` | Messages that agree with a query, with text. User token only. | Any | No |

The reaction objects in the API do not have a time.
To measure the time of an acknowledgement by reaction, Insight must receive the `reaction_added` events.

### 2.5 Huddles and calls

| Source | Data | Plan | In Insight |
|---|---|---|---|
| Audit Logs: `huddle_started`, `huddle_ended`, `huddle_participant_joined`, `huddle_participant_left` | Each huddle with its participants and duration | Enterprise | Planned (#4352) |
| Event `user_huddle_changed` | A member joins or leaves a huddle. Insight can calculate participants and duration. The data starts at installation. | Any | No |
| `calls.info` | Calls that an app registered. Native huddles are not included (unconfirmed). | Any | No |

### 2.6 AI

| Source | Data | Plan | In Insight |
|---|---|---|---|
| `admin.analytics.slackbot.tools`, `admin.analytics.slackbot.skills` | For each member, for each day: use of Slackbot tools and skills | Enterprise | Planned (#4353) |
| Audit Logs: `slack_ai_*` | Each AI action: summary, recap, huddle notes, file summary, translation, skill, tool call | Enterprise | Planned (#4353) |
| Audit Logs: `mcp_slack_*` | Each tool call through the Slack MCP server | Enterprise | Planned (#4353) |
| Audit Logs: `agentforce_*` | Agentforce agents: activation, deactivation, actions in Slack | Enterprise | Planned (#4353) |
| Analytics dashboard, tab AI | Totals of summaries, recaps, huddle notes, translations and search answers. No API. Manual CSV export only. | B+ | No |

Insight reads AI use from other tools: ChatGPT Team, Claude Enterprise, Claude Team and Cursor.

### 2.7 Bots and apps

| Source | Data | Plan | In Insight |
|---|---|---|---|
| `admin.apps.approved.list`, `admin.apps.requests.list` | Approved apps and requests for apps, with scopes and sensitive scopes | Enterprise | Planned (#4355) |
| `team.integrationLogs` | An app is added, removed, enabled, disabled or changed. User, channel, date. | Paid (unconfirmed) | No |
| `bots.info` | Bot ID to app ID and name | Any | No |
| Audit Logs: `app_*`, `bot_token_*` | App installations, approvals, scope changes | Enterprise | No |
| Channel file: messages minus messages from members | Bot messages for each channel, for each day. No message read is necessary. | B+ | Planned (#4350) |

### 2.8 Workflows

| Source | Data | Plan | In Insight |
|---|---|---|---|
| `admin.workflows.search` | Workflows: steps, triggers, collaborators, publish date. No runs. | Enterprise | Planned (#4355) |
| `admin.apps.activities.list` | Runs of workflows and functions, step results, errors. Retention is not documented. | Unconfirmed | Planned (#4355) |
| Audit Logs: `workflow_*` | Workflow created, published, deleted. Trigger permissions. | Enterprise | No |
| Workflow activity log (UI) | Runs and their status for 90 days. No API. No export. | Paid | No |

### 2.9 Alerts

Alert tools (PagerDuty, Grafana, Sentry, CI) send messages to channels.

| Source | Data | Plan | In Insight |
|---|---|---|---|
| `conversations.history`, `conversations.replies` with `bot_id` | Alerts for each channel and each source app. Human replies to each alert. | Any | Planned (#4351) |
| Events `message.channels`, `reaction_added` | Time of the first human reply or reaction to each alert | Any | No |

With this data, Insight can calculate:

- The number of alerts for each channel, source and day.
- The percentage of alerts that a person acknowledged.
- The time to the first human response.

### 2.10 Security and compliance

| Source | Data | Plan | In Insight |
|---|---|---|---|
| Audit Logs: logins, file downloads, exports, role changes | Each event with actor, IP and user agent. History from March 2018. | Enterprise | No |
| `team.accessLogs` | For each member and IP: first and last access, count, country | Paid | No |
| Discovery API | All messages, files, canvases, edits and deletions | Enterprise | **Do not use.** Slack permits this API for compliance only (eDiscovery, archives, DLP). Analytics is not permitted. |

### 2.11 Data without an API

The Slack analytics dashboard shows this data. No API supplies it:

- Daily, weekly and monthly active members and posting members.
- Statistics of private channels.
- Huddles for each channel.
- Use of each app: messages, members, automations.
- Totals of AI features.

## 3. Most valuable data for Insight

| Priority | Data | Source | Plan | Value |
|---|---|---|---|---|
| 1 | Alert load (#4351) | Message history and events (2.9) | Any | New metric for engineers. Internal app only. |
| 2 | Huddles with duration and participants (#4352) | Audit Logs (2.5) | Enterprise | Huddles become meetings with time. This replaces the "count only" rule in SPEC.md section 10. |
| 3 | AI use for each person, for each day (#4353) | Slackbot APIs and audit events (2.6) | Enterprise | Adds Slack to the AI tools that Insight reads now. |
| 4 | Channel activity and bot share (#4350) | Channel file (2.3) | B+ | Same API as now, different `type`. Low cost. |
| 5 | Response time and thread depth (#4354) | `messages.metadata` (2.4) | Enterprise | No message text in Insight. |

## 4. References

- Rate limits: https://docs.slack.dev/apis/web-api/rate-limits
- Plan: https://docs.slack.dev/reference/methods/team.billing.info
- 2025 limit change: https://docs.slack.dev/changelog/2025/05/29/rate-limit-changes-for-non-marketplace-apps
- Analytics file: https://docs.slack.dev/reference/methods/admin.analytics.getFile
- Message analytics: https://docs.slack.dev/reference/methods/admin.analytics.messages.metadata, https://docs.slack.dev/reference/methods/admin.analytics.messages.activity
- Slackbot analytics: https://docs.slack.dev/reference/methods/admin.analytics.slackbot.tools, https://docs.slack.dev/reference/methods/admin.analytics.slackbot.skills
- Audit Logs: https://docs.slack.dev/admins/audit-logs-api/, https://docs.slack.dev/reference/audit-logs-api/methods-actions-reference/
- Discovery API: https://slack.com/help/articles/360002079527-Discovery-API
- SCIM: https://docs.slack.dev/admins/scim-api
- Admin methods: https://docs.slack.dev/reference/methods/admin.users.list, https://docs.slack.dev/reference/methods/admin.conversations.search, https://docs.slack.dev/reference/methods/admin.workflows.search, https://docs.slack.dev/reference/methods/admin.apps.activities.list, https://docs.slack.dev/reference/methods/admin.apps.approved.list
- Team methods: https://docs.slack.dev/reference/methods/team.accessLogs, https://docs.slack.dev/reference/methods/team.integrationLogs
- Messages: https://docs.slack.dev/reference/methods/conversations.history, https://docs.slack.dev/reference/methods/conversations.replies
- Huddles: https://docs.slack.dev/reference/events/user_huddle_changed, https://docs.slack.dev/reference/methods/calls.info
- Analytics dashboard: https://slack.com/help/articles/218407447-View-your-Slack-analytics-dashboard, https://slack.com/help/articles/360057638533-Understand-the-data-in-your-Slack-analytics-dashboard

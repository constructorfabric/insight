---
status: proposed
date: 2026-11-10
---

# PRD — Connector schedules and the connectors service

## 1. Purpose

An instance admin changes how often each connector instance syncs, from the Connector health page,
and sees whether the change is in effect. Connector health, sync history and ingestion intensity are
served by a new `connectors` service, owned by ingestion, instead of analytics.

## 2. Users

| User | What they do here |
|---|---|
| Instance admin | Reads connector health; sets, changes and resets an instance's sync schedule |
| Operator | Deploys the install; sets the default schedule of an instance in the deployment configuration |

Only users with the admin role reach these pages and APIs. Everyone else gets 403; an unauthenticated
request gets 401.

## 3. Terms

| Term | Meaning |
|---|---|
| Instance | One configured source of one connector, for example `jira / main` |
| Sync job | The instance's Argo CronWorkflow; it starts each sync on its schedule |
| Saved schedule | The schedule an admin set for an instance in the product |
| Default schedule | What runs when no schedule is saved: the instance's own setting in the deployment configuration, else the connector's, else daily at 00:00 UTC |
| Reconcile pass | The existing periodic loop that creates and updates connections and sync jobs from the deployment configuration |

## 4. Requirements

### 4.1 Connector health moves to the connectors service

1. The Connector health page, an instance's sync history and ingestion intensity read from the
   `connectors` service.
2. What the page shows and how it sorts stay as they are, except for the changes in 4.6.

### 4.2 Viewing a schedule

1. Each instance shows its schedule in words and as cron, with times in UTC.
2. It shows the next three runs, in UTC.
3. A saved schedule shows who set it and when, and that it overrides the default schedule.
4. One status badge shows the state:

| Badge | Meaning |
|---|---|
| Applied | The sync job runs the saved schedule |
| Applying | Saved; the service is applying it and retries within a minute |
| Waiting for reconcile | Saved; the sync job is not yet prepared to take it; applies at the next reconcile pass, whose expected time is shown |
| Not scheduled yet | Saved; the instance has no sync job yet; the next reconcile pass creates it with this schedule |
| Error | Saved, but the cluster refused it; the service retries every minute |
| Default | No saved schedule; the default schedule runs |

5. While a save is not yet applied, the strip shows one short line: when it takes effect, or that it
   is being retried, and what runs until then.
6. The badge's tooltip carries the detail: the reason, the error message, and exact times.
7. A change to Applied or Error shows a short notification.
8. The row's next sync follows the applied schedule as soon as it is applied.

### 4.3 Changing a schedule

1. The admin chooses a preset or writes cron:
   - every hour, at a chosen minute;
   - every 2, 3, 4, 6, 8 or 12 hours, from a chosen start time;
   - every day, or every weekday, at a chosen time;
   - custom: 1 to 10 cron expressions, five fields, UTC.
2. Any frequency is allowed.
3. Before saving, the editor shows the cron that will be stored and the next three runs.
4. An invalid value shows an error next to the field, and Save stays disabled.
5. Save stores the schedule and applies it to the sync job at once.
6. The badge says Applied only after the service has read the sync job back and found the saved schedule on it.
7. If another admin changed the schedule in the meantime, the save is refused, and the editor
   reloads the current schedule and says so.

### 4.4 Resetting a schedule

1. "Reset to default" removes the saved schedule.
2. The sync job returns to the default schedule at once, and the badge says Default.

### 4.5 Keeping a saved schedule

1. A saved schedule survives reconcile passes: a pass leaves it in place.
2. If anything else changes the sync job's schedule — a pass that started before the save, or a manual
   edit — the service restores the saved schedule within a minute.
3. A schedule the cluster refused stays saved and is retried every minute until it applies.
4. The order of precedence is: saved schedule, then the instance's deployment setting, then the
   connector's, then daily at 00:00 UTC.
5. If a reconcile pass cannot read the saved schedules, it changes no sync job's schedule and reports
   the pass as failed.

### 4.6 Sync history in the expanded row

1. The schedule is shown full width at the top of the expanded row.
2. Below it:
   - a strip of up to the last 50 syncs, one small coloured bar each, with totals by status;
   - a table of the latest 10 syncs with column headers (status, start in UTC, duration,
     records), which expands to show the rest.
3. The heading states how many syncs it shows, for example "Latest 10 of 50 recorded".

## 5. Constraints

1. Schedules are stored and applied in UTC, and every time on the page is shown in UTC.
2. The service changes only the schedule, the timezone and its own marker on a sync job; it changes no
   other field.
3. The service runs as one replica and uses the install's existing database credentials; it adds no
   new secret.
4. The reconcile loop reads saved schedules through a read-only database account.
5. Each correction the service makes to a sync job is logged, and Kubernetes records the service as
   the writer.

## 6. Acceptance criteria

1. Saving `*/5 * * * *` for an instance shows Applied, and the sync job runs `*/5 * * * *` right away.
2. A reconcile pass after the save leaves `*/5 * * * *` in place.
3. A pass that started before the save and writes the old schedule is corrected within 60 seconds,
   and the badge says Applied.
4. Reset to default shows Default at once; the sync job runs the default schedule, and the next pass
   keeps it.
5. With the service's permission to change sync jobs removed, a save shows Error, and the saved
   schedule is kept. Within a minute of restoring the permission, it applies and shows Applied.
6. The saved schedule fires, the sync succeeds, and the row shows the sync as succeeded.
7. A failed sync shows as failed in the row, sorts first, and is listed in the sync history.
8. With the read-only account unable to read saved schedules, a reconcile pass changes no sync job's
   schedule and reports one failure; once access is back, the next pass is clean.
9. A non-admin gets 403 from every schedule and health API.

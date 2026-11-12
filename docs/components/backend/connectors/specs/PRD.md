---
status: proposed
date: 2026-11-10
---

# PRD — Connector schedules and the connectors service

## 1. Purpose

An instance admin changes how often each connector instance syncs, from the Connectors page,
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

### 4.1 The Connectors page and the connectors service

1. The admin page that lists connector instances is called Connectors, in the menu and in its heading.
2. The page, an instance's sync history and ingestion intensity read from the `connectors` service.
3. The table keeps its columns and sort order.

### 4.2 Viewing a schedule

1. Each instance shows its schedule in words and as cron, with times in UTC.
2. It shows the next three runs, in UTC.
3. A saved schedule shows who set it, by the admin's name ("you" for the viewer), and when.
4. One status badge shows the state:

| Badge | Meaning |
|---|---|
| Applied | The sync job runs the saved schedule |
| Waiting for reconcile | The sync job runs something other than the saved or default schedule, for example after a manual edit; the next reconcile pass corrects it, and its expected time is shown |
| Not scheduled yet | Saved; the instance has no sync job yet; the next reconcile pass creates it with this schedule |
| Default | No saved schedule; the default schedule runs |

5. While the badge says Waiting for reconcile, the strip shows one short line: when the next pass
   corrects it and what runs until then.
6. The badge's tooltip carries the detail: the reason (for example, the sync job runs a different
   schedule) and exact times.
7. A change to Applied shows a short notification.
8. The status always reflects the sync job as it is when the page reads it.
9. The row's next sync follows the applied schedule as soon as it is applied.
10. A schedule another admin saves appears on an open page within a minute, with that admin's name.
11. Viewing the page or an instance never waits for a save in progress.

### 4.3 Changing a schedule

1. The admin chooses a preset or writes cron:
   - every hour, at a chosen minute;
   - every 2, 3, 4, 6, 8 or 12 hours, from a chosen start time;
   - every day, or every weekday, at a chosen time;
   - custom: 1 to 10 cron expressions, five fields, UTC.
2. Any frequency is allowed.
3. Before saving, the editor shows the cron that will be stored and the next three runs.
4. An invalid value shows an error next to the field, and Save stays disabled.
5. Save stores the schedule and applies it to the sync job at once, or changes nothing. If the sync job
   cannot take it, the saved schedule and the sync job keep their previous values, and the editor says
   the schedule was not changed and why. An instance that has no sync job yet keeps the saved schedule,
   and the next reconcile pass creates the sync job with it.
6. The badge says Applied only after the service has read the sync job back and found the saved schedule on it.
7. If another admin changed the schedule in the meantime, the save is refused, and the editor
   reloads the current schedule and says so.

### 4.4 Resetting a schedule

1. "Reset to default" removes the saved schedule.
2. The sync job returns to the default schedule at once, and the badge says Default. If it cannot,
   nothing changes, and the editor says why.
3. Reset needs the default schedule recorded on the sync job by a reconcile pass; until then it is
   refused with that reason, and nothing changes.

### 4.5 Keeping a saved schedule

1. A saved schedule survives reconcile passes: a pass leaves it in place.
2. A reconcile pass never writes an older schedule over a newer save: it reads an instance's saved
   schedule when it updates that instance's sync job, and a save and a pass never update the same
   instance's sync job at the same time.
3. If anything else changes the sync job's schedule, for example a manual edit, the next reconcile pass
   restores the saved schedule. The service does not retry on its own.
4. The order of precedence is: saved schedule, then the instance's deployment setting, then the
   connector's, then daily at 00:00 UTC.
5. If a reconcile pass cannot read the saved schedules, it changes no sync job's schedule and reports
   the pass as failed.
6. If a pass cannot reach an instance's sync job because a save holds it for too long, it leaves that
   sync job for the next pass and reports the instance as failed.

### 4.6 Sync history in the expanded row

1. The schedule is shown full width at the top of the expanded row.
2. Below it:
   - a strip of up to the last 50 syncs, one small coloured bar each, with the share of finished syncs
     that succeeded, for example "94% succeeded";
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
5. Each change the service makes to a sync job is logged, and Kubernetes records the service as the
   writer.

## 6. Acceptance criteria

1. Saving `*/5 * * * *` for an instance shows Applied, and the sync job runs `*/5 * * * *` right away.
2. A reconcile pass after the save leaves `*/5 * * * *` in place.
3. A save made while a reconcile pass is running stays in effect: when the pass reaches the instance it
   writes the saved schedule, and the badge stays Applied. A reset made during a pass stays at Default
   the same way.
4. Reset to default shows Default at once; the sync job runs the default schedule, and the next pass
   keeps it.
5. With the service's permission to change sync jobs removed, a save says the schedule was not changed,
   with the cluster's reason; the saved schedule, the sync job and the badge keep their previous values.
6. The saved schedule fires, the sync succeeds, and the row shows the sync as succeeded.
7. A failed sync shows as failed in the row, sorts first, and is listed in the sync history.
8. With the read-only account unable to read saved schedules, a reconcile pass changes no sync job's
   schedule and reports one failure; once access is back, the next pass is clean.
9. A non-admin gets 403 from every schedule and health API.
10. With the page open for admin A, admin B saves a schedule for the same instance. Within a minute A's
    page shows B's schedule and B's name. A save from an editor A opened before B's save is refused,
    and the editor shows B's schedule.
11. A manual edit of the sync job's schedule shows Waiting for reconcile within a minute; the next pass
    restores the saved schedule, and the badge says Applied.
12. On a sync job without the recorded default schedule, Reset is refused with that reason, and nothing
    changes.
13. While a save holds an instance, the page and that instance still load.

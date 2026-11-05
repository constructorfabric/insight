import { useEffect, useRef, useState } from "react";

import type {
  AppliedSchedule,
  ConnectorHealth,
  InheritedSchedule,
  ScheduleRead,
} from "@/api/connector-health-client";
import { ConnectorScheduleHeader } from "@/components/portal/connector-schedule-header";
import { Alert } from "@/components/ui/alert";
import { Button } from "@gears-frontx/ui-kit";
import { ConnectorScheduleEditor } from "@/components/portal/connector-schedule-editor";
import {
  localStart,
  localTime,
  scheduleWords,
} from "@/lib/portal/connector-schedule-editor";
import { isScheduleOverride } from "@/lib/portal/connector-schedule";
import {
  useConnectorSchedule,
  useWriteConnectorSchedule,
} from "@/queries/connector-health";
import { useMe } from "@/queries/identity-me";

export function ConnectorSchedulePane({
  row,
  polling,
}: {
  row: ConnectorHealth;
  polling: boolean;
}) {
  const query = useConnectorSchedule(row, polling);
  const write = useWriteConnectorSchedule(row);
  const me = useMe();
  const [editing, setEditing] = useState(false);
  const editButton = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);
  const schedule = query.data?.schedule;
  const [seenStatus, setSeenStatus] = useState(schedule?.status);
  const [appliedAt, setAppliedAt] = useState<string | null>(null);
  if (schedule?.status !== seenStatus) {
    if (
      schedule?.status === "applied" &&
      (seenStatus === "pending" || seenStatus === "error")
    ) {
      setAppliedAt(schedule.applied?.observed_at ?? null);
    }
    setSeenStatus(schedule?.status);
  }

  useEffect(() => {
    if (!editing && returnFocus.current) {
      editButton.current?.focus();
      returnFocus.current = false;
    }
  }, [editing]);

  function close() {
    returnFocus.current = true;
    setEditing(false);
  }
  async function reload(): Promise<ScheduleRead> {
    const result = await query.refetch();
    if (result.error || !result.data)
      throw result.error ?? new Error("Schedule unavailable");
    return result.data;
  }

  if (editing && query.data) {
    return (
      <ConnectorScheduleEditor
        read={query.data}
        save={(etag, crons) => write.mutateAsync({ etag, crons })}
        reload={reload}
        onClose={close}
      />
    );
  }
  const request = schedule?.requested;
  const override = isScheduleOverride(request);
  const inherited = schedule?.inherited;
  const differs =
    schedule?.status === "pending" || schedule?.status === "error";
  return (
    <div className="min-w-0 space-y-3 [overflow-wrap:anywhere] whitespace-normal">
      <ConnectorScheduleHeader schedule={schedule} />
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
        <dt className="text-muted-foreground">
          {differs ? "Runs now" : "Runs"}
        </dt>
        <dd className="min-w-0">
          <ScheduleValue schedule={row.schedule} />
        </dd>
        {override && !differs && (
          <>
            <dt className="text-muted-foreground">Set by</dt>
            <dd className="min-w-0">
              {request.updated_by === me.data?.person_id
                ? "you"
                : "another admin"}
              , {localStart(request.updated_at)}
            </dd>
          </>
        )}
        {differs && request && (
          <>
            <dt className="text-muted-foreground">Requested</dt>
            <dd className="min-w-0">
              {override ? (
                <ScheduleValue
                  schedule={{
                    crons: request.crons,
                    timezone: "UTC",
                    suspended: false,
                  }}
                />
              ) : (
                <>
                  Returns to the deployment schedule
                  {inherited ? (
                    <>
                      : <DeploymentScheduleValue inherited={inherited} />
                    </>
                  ) : (
                    "."
                  )}
                </>
              )}
              <span className="text-xs text-muted-foreground">
                {" "}
                ·{" "}
                {request.updated_by === me.data?.person_id
                  ? "you"
                  : "another admin"}
                , {localTime(request.updated_at)}
              </span>
            </dd>
          </>
        )}
      </dl>
      {override && inherited && (
        <p className="text-sm text-muted-foreground">
          Deployment schedule: <DeploymentScheduleValue inherited={inherited} />
          , overridden by this request
        </p>
      )}
      <div
        aria-live="polite"
        className={
          schedule?.status === "error"
            ? "text-sm text-warning"
            : "text-sm text-muted-foreground"
        }
      >
        {schedule?.status === "pending" && (
          <Alert className="bg-primary/10 text-primary">
            Saved. Until the request is applied, the current schedule keeps
            running.
          </Alert>
        )}
        {schedule?.status === "error" && (
          <Alert variant="warning" className="bg-warning/10">
            <p>
              {schedule.error?.message ?? "The schedule could not be applied."}
            </p>
            <p>
              Attempted at {localTime(schedule.error?.attempted_at)}. Edit and
              save again to retry.
            </p>
          </Alert>
        )}
        {schedule?.status === "applied" && appliedAt && (
          <p>Applied at {localTime(appliedAt)}.</p>
        )}
      </div>
      {query.isPending && row.source_id != null && (
        <p className="text-sm text-muted-foreground">Loading schedule…</p>
      )}
      {query.isError && (
        <div className="text-sm text-destructive">
          Could not load the schedule.{" "}
          <Button variant="link" size="sm" onClick={() => void query.refetch()}>
            Retry
          </Button>
        </div>
      )}
      {schedule && (
        <div className="flex justify-end">
          <Button
            ref={editButton}
            type="button"
            size="sm"
            variant="outline"
            onClick={() => setEditing(true)}
          >
            Edit
          </Button>
        </div>
      )}
    </div>
  );
}

function DeploymentScheduleValue({
  inherited,
}: {
  inherited: InheritedSchedule;
}) {
  const source =
    inherited.origin === "annotation"
      ? "the Secret annotation"
      : inherited.origin === "descriptor"
        ? "the connector descriptor"
        : inherited.origin === "default"
          ? "the default"
          : inherited.origin;
  return (
    <>
      {scheduleWords(inherited.crons)} ({inherited.crons.join("; ")} UTC, from{" "}
      {source})
    </>
  );
}

function ScheduleValue({
  schedule,
}: {
  schedule: AppliedSchedule | null | undefined;
}) {
  if (!schedule) return <>—</>;
  if (schedule.crons.length === 0) return <>The default schedule</>;
  const words =
    schedule.timezone === "UTC" ? scheduleWords(schedule.crons) : null;
  return (
    <>
      {words && <span className="block">{words}</span>}
      <span className="font-mono text-xs text-muted-foreground">
        {schedule.crons.join("; ")} ({schedule.timezone ?? "controller time"})
        {schedule.suspended ? " · suspended" : ""}
      </span>
    </>
  );
}

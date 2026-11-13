import { useEffect, useRef, useState } from "react";

import type {
  AppliedSchedule,
  ConnectorHealth,
  ScheduleRead,
} from "@/api/connector-health-client";
import { Button, toast } from "@gears-frontx/ui-kit";
import { ConnectorScheduleHeader } from "./connector-schedule-header";
import { ConnectorScheduleStatus } from "./connector-schedule-status";
import { ConnectorScheduleEditor } from "./connector-schedule-editor";
import { ConnectorScheduleRuns } from "./connector-schedule-runs";
import { ConnectorScheduleTooltip } from "./connector-schedule-tooltip";
import {
  conciseScheduleWords,
  scheduleTransitionMessage,
} from "@/lib/portal/connector-schedule-display";
import {
  relativeScheduleTime,
  scheduleTime,
} from "@/lib/portal/connector-schedule-time";
import { instanceKey } from "@/lib/portal/connector-health";
import { isScheduleOverride } from "@/lib/portal/connector-schedule";
import {
  useConnectorSchedule,
  useWriteConnectorSchedule,
} from "@/queries/connector-health";
import { useConnectorScheduleAuthor } from "@/queries/connector-schedule-author";

export function ConnectorSchedulePane({
  row,
  polling,
  reconcileIntervalSecs,
}: {
  row: ConnectorHealth;
  polling: boolean;
  reconcileIntervalSecs?: number | null;
}) {
  const query = useConnectorSchedule(row, polling);
  const write = useWriteConnectorSchedule(row);
  const [failedRevision, setFailedRevision] = useState<number>();
  const [editing, setEditing] = useState(false);
  const editButton = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);
  const schedule = query.data?.schedule;
  const identity = instanceKey(row);
  const [seen, setSeen] = useState({
    identity,
    status: schedule?.status,
    revision: schedule?.revision,
    message: "",
  });
  if (
    identity !== seen.identity ||
    schedule?.status !== seen.status ||
    schedule?.revision !== seen.revision
  ) {
    const message =
      identity === seen.identity &&
      seen.status !== undefined &&
      seen.revision !== undefined &&
      schedule &&
      schedule.revision >= seen.revision &&
      schedule.revision !== failedRevision
        ? scheduleTransitionMessage(schedule)
        : "";
    setSeen({
      identity,
      status: schedule?.status,
      revision: schedule?.revision,
      message,
    });
  }

  useEffect(() => {
    if (seen.message)
      toast.add({
        type: "success",
        title: seen.message,
      });
  }, [seen]);

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

  const request = schedule?.requested;
  const override = isScheduleOverride(request);
  const differs = schedule?.status === "pending" || schedule?.status === null;
  const live = schedule ? schedule.live : row.schedule;
  const displayed = differs
    ? override
      ? { crons: request.crons, timezone: "UTC", suspended: false }
      : schedule?.inherited
        ? { ...schedule.inherited, timezone: "UTC", suspended: false }
        : null
    : live;
  const person = useConnectorScheduleAuthor(request?.updated_by);

  return (
    <>
      <div aria-live="polite" className="sr-only">
        <span key={`${seen.identity}/${seen.revision}/${seen.status}`}>
          {seen.message}
        </span>
      </div>
      {editing && query.data ? (
        <ConnectorScheduleEditor
          read={query.data}
          save={async (etag, crons) => {
            try {
              const result = await write.mutateAsync({ etag, crons });
              setFailedRevision(undefined);
              return result;
            } catch (error) {
              setFailedRevision(schedule?.revision);
              throw error;
            }
          }}
          reload={reload}
          onClose={close}
        />
      ) : (
        <div className="min-w-0 space-y-3 [overflow-wrap:anywhere] whitespace-normal">
          <ConnectorScheduleHeader
            schedule={schedule}
            loading={query.isPending && !schedule && row.source_id != null}
          >
            <div className="ml-auto flex min-w-0 flex-wrap items-center justify-end gap-3">
              {override && (
                <ConnectorScheduleTooltip
                  lines={[`Set by ${person}`, scheduleTime(request.updated_at)]}
                >
                  <span className="text-xs text-muted-foreground">
                    set by {person} · {relativeScheduleTime(request.updated_at)}
                  </span>
                </ConnectorScheduleTooltip>
              )}
              {schedule && (
                <Button
                  ref={editButton}
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => setEditing(true)}
                >
                  Edit
                </Button>
              )}
            </div>
          </ConnectorScheduleHeader>
          <div
            className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1 text-sm"
            data-slot="schedule-value"
          >
            <ScheduleValue
              schedule={displayed}
              defaultUnknown={differs && !override}
            />
          </div>
          {schedule && (
            <ConnectorScheduleStatus
              schedule={schedule}
              reconcileIntervalSecs={reconcileIntervalSecs}
            />
          )}
          {!differs && live?.timezone && (
            <ConnectorScheduleRuns
              crons={live.crons}
              timezone={live.timezone}
              suspended={live.suspended}
            />
          )}
          {query.isPending && row.source_id != null && (
            <p className="text-sm text-muted-foreground">Loading schedule…</p>
          )}
          {query.isError && (
            <div className="text-sm text-destructive">
              Could not load the schedule.{" "}
              <Button
                variant="link"
                size="sm"
                onClick={() => void query.refetch()}
              >
                Retry
              </Button>
            </div>
          )}
        </div>
      )}
    </>
  );
}

function ScheduleValue({
  schedule,
  defaultUnknown,
}: {
  schedule: AppliedSchedule | null | undefined;
  defaultUnknown: boolean;
}) {
  if (!schedule)
    return <span>{defaultUnknown ? "Default schedule" : "—"}</span>;
  if (!schedule.crons.length) return <span>Default schedule</span>;
  return (
    <>
      {schedule.timezone === "UTC" && (
        <span className="font-medium">
          {conciseScheduleWords(schedule.crons)}
        </span>
      )}
      <code className="rounded bg-muted px-1.5 py-0.5 text-xs text-muted-foreground">
        {schedule.crons.join("; ")}
      </code>
      <span className="text-xs text-muted-foreground">
        {schedule.timezone ?? "controller time"}
        {!schedule.timezone && schedule.suspended ? " · suspended" : ""}
      </span>
    </>
  );
}

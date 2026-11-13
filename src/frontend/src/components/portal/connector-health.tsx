import { useId, useState } from "react";

import type {
  ConnectorHealth,
  ConnectorInstanceRef,
} from "@/api/connector-health-client";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import {
  Badge,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@gears-frontx/ui-kit";
import { ConnectorSchedulePane } from "@/components/portal/connector-schedule-pane";
import { ConnectorSyncHistory } from "./connector-sync-history";
import { TONE_STYLE } from "./connector-health-tone";
import { ConnectorScheduleRequestChip } from "@/components/portal/connector-schedule-request-chip";
import {
  UNMEASURED,
  SCHEDULE_HEADING,
  describeConnector,
  describeDuration,
  describeInstance,
  describeNextRun,
  describeRecording,
  describeSchedule,
  formatRecords,
  formatStarted,
  instanceKey,
} from "@/lib/portal/connector-health";
import { useConnectorHealth } from "@/queries/connector-health";
import {
  activatesRow,
  activatesRowByKey,
} from "@/lib/identities/row-activation";
import { cn } from "@/lib/utils";

const COLUMNS = 7;

/**
 * A DOM id for one row's disclosure panel.
 *
 * The identity's own separator is not one an id may carry through a CSS
 * selector, so it is swapped for an underscore — and any underscore already in
 * the key is escaped first, or `a_b/c` and `a/b_c` would name one panel and
 * `aria-controls` would point two rows at the same disclosure.
 */
function panelIdFor(row: ConnectorHealth): string {
  const id = instanceKey(row).replaceAll("_", "%5F").replaceAll("/", "_");
  return `connector-syncs-${id}`;
}

export function ConnectorHealthPane() {
  const { data, isPending, isError, refetch } = useConnectorHealth();
  const [expanded, setExpanded] = useState<string | null>(null);

  if (isPending)
    return (
      <CenteredSpinner
        data-testid="page-manage-connector-health"
        className="min-h-[60vh]"
      />
    );
  if (isError || data === undefined) {
    return (
      <div
        data-testid="page-manage-connector-health"
        className="mx-auto w-full max-w-md p-8"
      >
        <ComingSoon variant="card" state="error" onRetry={() => refetch()} />
      </div>
    );
  }

  const recording = describeRecording(data);
  const polling = data.connectors.some(
    (row) => row.schedule_request?.last_observation?.status === "pending"
  );

  return (
    <div
      data-testid="page-manage-connector-health"
      className="flex min-w-0 flex-col gap-3 p-4 md:p-6"
    >
      <div>
        <h1 className="text-lg font-semibold tracking-tight">Connectors</h1>
        <div
          role="status"
          aria-live="polite"
          className={cn(
            "text-sm",
            recording.state === "stopped" || recording.state === "unreadable"
              ? "text-destructive"
              : "text-muted-foreground"
          )}
        >
          <p>{recording.label}</p>
          {/* Inside the region, not beside it: the warning's consequence is the
              half a reader most needs announced. */}
          {recording.detail !== "" && <p>{recording.detail}</p>}
        </div>
      </div>

      {data.schedule_read_error && (
        <p role="alert" className="text-sm text-warning">
          {data.schedule_read_error}
        </p>
      )}
      {data.connectors.length === 0 ? (
        <div className="rounded-lg border p-6 text-sm text-muted-foreground">
          {recording.state === "never_read"
            ? "No connector has been recorded yet. This page reports what has been read from the data mover, and nothing has been read."
            : "The data mover was read and reported no connector. Nothing here says a connector is missing — only that none was recorded."}
        </div>
      ) : (
        <div className="max-w-full min-w-0 overflow-x-auto rounded-lg border">
          <Table containerClassName="@container">
            <TableHeader>
              <TableRow>
                <TableHead className="whitespace-normal">Connector</TableHead>
                {/* One connector can be configured more than once; this is the
                    only cell that tells those rows apart. */}
                <TableHead className="whitespace-normal">Instance</TableHead>
                <TableHead className="px-3 whitespace-normal sm:px-4">
                  {SCHEDULE_HEADING}
                </TableHead>
                <TableHead className="whitespace-normal">State</TableHead>
                <TableHead className="whitespace-normal">
                  Last sync started
                </TableHead>
                <TableHead className="text-right whitespace-normal">
                  Duration
                </TableHead>
                <TableHead className="text-right whitespace-normal">
                  Records reported
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.connectors.map((row) => {
                const state = describeConnector(row);
                // Keyed on the whole identity, not on the row's position and
                // not on the name. The page polls, so a position can come to
                // mean a different row between renders; and two installations
                // of one connector share the name, so keying on it would open
                // the wrong one's history.
                const key = instanceKey(row);
                const open = expanded === key;
                const panelId = panelIdFor(row);
                const toggle = () => setExpanded(open ? null : key);
                return [
                  // The whole row opens the connector, by the same rule every
                  // console listing follows — `activatesRow` is shared with
                  // them, so a press on a control and the end of a text
                  // selection are told apart here exactly as they are there.
                  //
                  // The gesture goes on the row and the role does not: a `<tr>`
                  // keeps `role="row"`, which supports `aria-expanded`, so the
                  // disclosure is announced without taking the row out of its
                  // table. An inner button would be a second focus stop for the
                  // one thing the row already does.
                  <TableRow
                    key={key}
                    data-state-name={state.state}
                    data-state={open ? "selected" : undefined}
                    tabIndex={0}
                    aria-expanded={open}
                    aria-controls={panelId}
                    onClick={(event) => {
                      if (activatesRow(event)) toggle();
                    }}
                    onKeyDown={(event) => {
                      if (!activatesRowByKey(event)) return;
                      event.preventDefault();
                      toggle();
                    }}
                    className="cursor-pointer select-text"
                  >
                    <TableCell className="font-medium">
                      {row.connector}
                    </TableCell>
                    <InstanceCell instance={row} />
                    <ScheduleCell row={row} />
                    <TableCell>
                      <Badge
                        variant="secondary"
                        className={cn("font-medium", TONE_STYLE[state.tone])}
                      >
                        {state.label}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground tabular-nums">
                      {formatStarted(row.last_sync?.started_at ?? null)}
                    </TableCell>
                    <DurationCell sync={row.last_sync} asOf={data.as_of} />
                    <TableCell className="text-right text-muted-foreground tabular-nums">
                      {formatRecords(row.last_sync?.records_reported ?? null)}
                    </TableCell>
                  </TableRow>,
                  open ? (
                    <TableRow key={`syncs-${key}`}>
                      <TableCell
                        colSpan={COLUMNS}
                        id={panelId}
                        className="bg-muted/40 px-4 whitespace-normal"
                      >
                        <div className="min-w-0 [contain:inline-size]">
                          <div className="sticky left-4 grid w-[calc(100cqw-2rem)] max-w-full min-w-0 grid-cols-1 gap-4 py-2 [overflow-wrap:anywhere] whitespace-normal">
                            {row.configured && row.source_id != null && (
                              <div className="@container min-w-0 rounded-md border bg-background p-4">
                                <ConnectorSchedulePane
                                  row={row}
                                  polling={polling}
                                  reconcileIntervalSecs={
                                    data.typical_read_interval_ms == null
                                      ? null
                                      : data.typical_read_interval_ms / 1000
                                  }
                                />
                              </div>
                            )}
                            <div className="min-w-0 rounded-md border bg-background p-4">
                              <ConnectorSyncHistory
                                instance={row}
                                asOf={data.as_of}
                              />
                            </div>
                          </div>
                        </div>
                      </TableCell>
                    </TableRow>
                  ) : null,
                ];
              })}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  );
}

function InstanceCell({ instance }: { instance: ConnectorInstanceRef }) {
  const label = describeInstance(instance);

  return (
    <TableCell aria-label={label}>
      {label === UNMEASURED ? (
        UNMEASURED
      ) : (
        <>
          <span className="block">{instance.source_id || UNMEASURED}</span>
          <span
            className="block max-w-40 truncate text-xs text-muted-foreground"
            title={instance.tenant_id || UNMEASURED}
          >
            {instance.tenant_id || UNMEASURED}
          </span>
        </>
      )}
    </TableCell>
  );
}

function ScheduleCell({ row }: { row: ConnectorHealth }) {
  const request = row.schedule_request;
  const schedule =
    request?.last_observation?.status === "applied" && request.crons.length
      ? {
          crons: request.crons,
          timezone: "UTC",
          suspended: row.schedule?.suspended ?? false,
        }
      : row.schedule;
  const tooltipId = useId();
  const nextRun =
    schedule == null ? null : describeNextRun(schedule, new Date());

  return (
    <TableCell className="px-3 align-middle sm:px-4">
      <div
        data-slot="schedule-next"
        className="flex items-center gap-1.5 leading-5 whitespace-nowrap"
      >
        {schedule == null ? (
          UNMEASURED
        ) : (
          <Tooltip>
            <TooltipTrigger
              aria-describedby={tooltipId}
              render={
                <button type="button" className="cursor-help text-left" />
              }
            >
              {nextRun?.text}
            </TooltipTrigger>
            <TooltipContent id={tooltipId} role="tooltip">
              <div className="flex flex-col">
                {schedule.crons.map((cron, index) => (
                  <span key={index} className="block font-mono">
                    {cron}
                  </span>
                ))}
                <span className="block text-muted-foreground">
                  {describeSchedule(schedule)}
                </span>
                {nextRun?.detail != null && (
                  <span className="block">{nextRun.detail}</span>
                )}
              </div>
            </TooltipContent>
          </Tooltip>
        )}
        <ConnectorScheduleRequestChip row={row} />
      </div>
    </TableCell>
  );
}

/**
 * A sync still in flight is not dimmed like a settled one: its number is the
 * only thing on the row that keeps moving, and it is what an operator opened
 * the page to see.
 */
function DurationCell({
  sync,
  asOf,
}: {
  sync: ConnectorHealth["last_sync"];
  asOf: string;
}) {
  const duration = describeDuration(sync, asOf);
  return (
    <TableCell
      className={cn(
        "text-right tabular-nums",
        duration.inFlight ? "text-foreground" : "text-muted-foreground"
      )}
    >
      {duration.text}
    </TableCell>
  );
}

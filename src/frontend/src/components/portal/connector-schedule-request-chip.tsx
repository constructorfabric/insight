import type {
  ConnectorHealth,
  ScheduleStatus,
} from "@/api/connector-health-client";
import { ConnectorScheduleTooltip } from "@/components/portal/connector-schedule-tooltip";
import { scheduleTooltipLines } from "@/lib/portal/connector-schedule-tooltip";
import { scheduleStatusView } from "@/lib/portal/connector-schedule-status";
import { useConnectorSchedule } from "@/queries/connector-health";
import { useConnectorScheduleAuthor } from "@/queries/connector-schedule-author";

export function ConnectorScheduleRequestChip({
  row,
}: {
  row: ConnectorHealth;
}) {
  const status = row.schedule_request?.last_observation?.status;
  if (
    !row.configured ||
    (status !== "pending" &&
      status !== "paused" &&
      !row.schedule_request?.read_error)
  ) {
    return null;
  }

  return <RequestStatus row={row} status={status} />;
}

function RequestStatus({
  row,
  status,
}: {
  row: ConnectorHealth;
  status: ScheduleStatus | undefined;
}) {
  const { data, isError, isFetching } = useConnectorSchedule(row, false);
  const loaded = data?.schedule;
  const schedule =
    !row.schedule_request?.read_error &&
    loaded &&
    loaded.revision >= (row.schedule_request?.revision ?? 0)
      ? loaded
      : undefined;
  const person = useConnectorScheduleAuthor(schedule?.requested?.updated_by);
  const currentStatus = schedule
    ? schedule.status
    : row.schedule_request?.read_error
      ? null
      : status;
  if (
    currentStatus !== "pending" &&
    currentStatus !== "paused" &&
    currentStatus != null
  )
    return null;
  const { label, Icon, tone, iconClass } = scheduleStatusView(
    currentStatus,
    schedule?.pending_reason
  );
  return (
    <ConnectorScheduleTooltip
      label={label}
      lines={
        !schedule && isError && !isFetching && !row.schedule_request?.read_error
          ? [`${label}. Schedule details could not be loaded.`]
          : scheduleTooltipLines(
              schedule ?? row.schedule_request ?? undefined,
              person
            )
      }
    >
      <span
        data-slot="schedule-request-status"
        title={label}
        aria-hidden="true"
        className={`inline-flex size-4 shrink-0 items-center justify-center rounded-full ${tone}`}
      >
        <Icon aria-hidden="true" className={iconClass} />
      </span>
    </ConnectorScheduleTooltip>
  );
}

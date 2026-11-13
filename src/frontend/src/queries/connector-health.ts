import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseQueryResult,
} from "@tanstack/react-query";
import { useEffect, useRef } from "react";

import {
  getConnectorHealth,
  getConnectorSyncs,
  getConnectorSchedule,
  putConnectorSchedule,
  deleteConnectorSchedule,
  type ScheduleRead,
  type ScheduleStatus,
  type ConnectorHealthSummary,
  type ConnectorHealth,
  type ConnectorInstanceRef,
  type ConnectorSyncHistory,
} from "@/api/connector-health-client";
import { sessionAuthorizationScope } from "@/auth/session-scope";
import { useAuth } from "@/auth/use-auth";
import { instanceKey } from "@/lib/portal/connector-health";

const KEY = ["connector-health"] as const;

/** How often the open page asks again. Well inside the reconcile cadence, so
 * the age it prints is never more than a minute behind the truth. */
const RECHECK_MS = 60_000;

export function useConnectorHealth(): UseQueryResult<ConnectorHealthSummary> {
  const { session } = useAuth();
  const client = useQueryClient();
  const scope = sessionAuthorizationScope(session);
  return useQuery({
    queryKey: [...KEY, "summary", sessionAuthorizationScope(session)],
    queryFn: async () => {
      const summary = await getConnectorHealth();
      return {
        ...summary,
        connectors: summary.connectors.map((row) => {
          if (summary.schedule_read_error) return row;
          const read = client.getQueryData<ScheduleRead>([
            ...KEY,
            "schedule",
            instanceKey(row),
            scope,
          ]);
          const schedule = read?.schedule;
          if (
            !schedule?.live ||
            (schedule.status !== "applied" && schedule.status !== "inherited")
          )
            return row;
          if (schedule.revision < (row.schedule_request?.revision ?? 0))
            return row;
          if (
            Date.parse(schedule.live.observed_at) <=
            Date.parse(
              row.schedule_request?.last_observation?.observed_at ??
                summary.as_of
            )
          )
            return row;
          return {
            ...row,
            schedule: schedule.live,
            schedule_request:
              schedule.revision > (row.schedule_request?.revision ?? 0)
                ? {
                    revision: schedule.revision,
                    crons: schedule.requested?.crons ?? [],
                    last_observation: {
                      status: schedule.status,
                      observed_at: schedule.live.observed_at,
                    },
                  }
                : row.schedule_request,
          };
        }),
      };
    },
    // The answer is only as fresh as the last sweep, and an operator opening
    // this page during an incident wants what is recorded now — not what was
    // recorded when they last looked.
    staleTime: 0,
    refetchOnMount: "always",
    // INVARIANT: the page's own freshness line is the gap between two stamps
    // the SERVER sent, so it does not move on its own. Without this the age
    // freezes the moment the page renders, and an operator watching an incident
    // would never see recording stop.
    refetchInterval: (query) =>
      query.state.data?.connectors.some((row) =>
        scheduleNeedsPolling(row.schedule_request?.last_observation?.status)
      )
        ? 30_000
        : RECHECK_MS,
  });
}

export function scheduleNeedsPolling(
  status: ScheduleStatus | null | undefined
): boolean {
  return status === "pending";
}

export function useConnectorSchedule(
  instance: ConnectorInstanceRef & Pick<ConnectorHealth, "schedule_request">,
  polling: boolean
): UseQueryResult<ScheduleRead> {
  const { session } = useAuth();
  const query = useQuery({
    queryKey: [
      ...KEY,
      "schedule",
      instanceKey(instance),
      sessionAuthorizationScope(session),
    ],
    queryFn: () => getConnectorSchedule(instance),
    enabled: instance.source_id != null,
    staleTime: 0,
    refetchOnMount: "always",
    refetchInterval: (query) => schedulePollInterval(query.state.data, polling),
  });
  const checked = useRef("");
  const hint = instance.schedule_request;
  const identity = `${instanceKey(instance)}/${sessionAuthorizationScope(session)}`;
  const revision = hint?.revision;
  const observedAt = hint?.last_observation?.observed_at;
  const { data, isFetching, refetch } = query;
  useEffect(() => {
    if (!data || isFetching || revision === undefined) return;
    const newer =
      revision > data.schedule.revision ||
      (revision === data.schedule.revision &&
        observedAt !== undefined &&
        Date.parse(observedAt) >
          (data.schedule.live
            ? Date.parse(data.schedule.live.observed_at)
            : 0));
    const stamp = `${identity}/${revision}/${observedAt}`;
    if (newer && checked.current !== stamp) {
      checked.current = stamp;
      void refetch();
    }
  }, [identity, revision, observedAt, data, isFetching, refetch]);
  return query;
}

export function useWriteConnectorSchedule(instance: ConnectorInstanceRef) {
  const client = useQueryClient();
  const { session } = useAuth();
  return useMutation({
    mutationFn: ({ etag, crons }: { etag: string; crons: string[] | null }) =>
      crons === null
        ? deleteConnectorSchedule(instance, etag)
        : putConnectorSchedule(instance, etag, crons),
    onSuccess: async (read) => {
      client.setQueryData(
        [
          ...KEY,
          "schedule",
          instanceKey(instance),
          sessionAuthorizationScope(session),
        ],
        read
      );
      client.setQueryData<ConnectorHealthSummary>(
        [...KEY, "summary", sessionAuthorizationScope(session)],
        (summary) =>
          summary && {
            ...summary,
            connectors: summary.connectors.map((row) =>
              instanceKey(row) === instanceKey(instance)
                ? {
                    ...row,
                    schedule:
                      (read.schedule.status === "applied" ||
                        read.schedule.status === "inherited") &&
                      read.schedule.live
                        ? read.schedule.live
                        : row.schedule,
                    schedule_request: {
                      revision: read.schedule.revision,
                      crons: read.schedule.requested?.crons ?? [],
                      last_observation:
                        read.schedule.live && read.schedule.status
                          ? {
                              status: read.schedule.status,
                              observed_at: read.schedule.live.observed_at,
                            }
                          : null,
                    },
                  }
                : row
            ),
          }
      );
      await Promise.all([
        client.invalidateQueries({ queryKey: [...KEY, "summary"] }),
        client.invalidateQueries({
          queryKey: [...KEY, "schedule", instanceKey(instance)],
        }),
      ]);
    },
  });
}

export function schedulePollInterval(
  read: ScheduleRead | undefined,
  polling: boolean
): number | false {
  return polling || scheduleNeedsPolling(read?.schedule.status)
    ? 30_000
    : false;
}

/**
 * One installation's history, keyed by the whole identity.
 *
 * Keying on the connector name alone would serve one installation's window
 * from the other's cache entry, because two installations share the name.
 */
export function useConnectorSyncs(
  instance: ConnectorInstanceRef | null
): UseQueryResult<ConnectorSyncHistory> {
  const { session } = useAuth();
  return useQuery({
    queryKey: [
      ...KEY,
      "syncs",
      instance === null ? null : instanceKey(instance),
      sessionAuthorizationScope(session),
    ],
    queryFn: () => getConnectorSyncs(instance as ConnectorInstanceRef),
    enabled: instance !== null,
    staleTime: 0,
  });
}

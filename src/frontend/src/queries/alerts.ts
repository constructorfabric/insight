import {
  infiniteQueryOptions,
  queryOptions,
  useMutation,
  useQueryClient,
  type InfiniteData,
  type QueryClient,
} from "@tanstack/react-query";

import type {
  Alert,
  AlertDraft,
  AlertPage,
  NotificationPage,
} from "@/api/alerts-client";
import {
  createAlert,
  deleteAlert,
  fetchAlert,
  fetchAlertDestinations,
  fetchAlertNotifications,
  fetchAlerts,
  replaceAlert,
  setAlertEnabled,
} from "@/api/alerts-client";
import { isRevisionConflict } from "@/lib/alerts/conflict";

const ALERTS_PREFIX = ["alerts"] as const;

/** How many alerts the list asks for at a time. */
const PAGE_SIZE = 50;

/** How many notifications the history asks for at a time. */
export const NOTIFICATION_PAGE_SIZE = 20;

/**
 * How often an open alert re-reads what its checks found. Checks run at most
 * once a minute, so this keeps the page within one check of the truth.
 */
const LIVE_REFRESH_MS = 30_000;

// INVARIANT: alert reads are live state, not the hour-fresh analytics the
// app's client defaults are tuned for.
const LIVE = { staleTime: 0, refetchOnWindowFocus: true } as const;

/**
 * The alerts, a page at a time.
 *
 * The needle is part of the key, so a search is its own cached answer rather
 * than overwriting the list.
 */
export function alertPagesQuery(search = "") {
  return infiniteQueryOptions({
    ...LIVE,
    queryKey: [...ALERTS_PREFIX, "list", search],
    queryFn: ({ pageParam }) =>
      fetchAlerts({ search, limit: PAGE_SIZE, offset: pageParam }),
    initialPageParam: 0,
    getNextPageParam: (last: AlertPage, pages: AlertPage[]) => {
      const read = pages.reduce((count, page) => count + page.alerts.length, 0);
      return read < last.total ? read : undefined;
    },
  });
}

export function alertQuery(id: string) {
  return queryOptions({
    ...LIVE,
    refetchInterval: LIVE_REFRESH_MS,
    queryKey: [...ALERTS_PREFIX, "one", id],
    queryFn: () => fetchAlert(id),
  });
}

/**
 * An alert's notifications, newest first.
 *
 * The service reports no total, so a page shorter than asked for is the end.
 */
export function alertNotificationsQuery(id: string) {
  return infiniteQueryOptions({
    ...LIVE,
    refetchInterval: LIVE_REFRESH_MS,
    queryKey: [...ALERTS_PREFIX, "notifications", id],
    queryFn: ({ pageParam }) =>
      fetchAlertNotifications(id, {
        limit: NOTIFICATION_PAGE_SIZE,
        offset: pageParam,
      }),
    initialPageParam: 0,
    getNextPageParam: (last: NotificationPage) =>
      last.notifications.length < NOTIFICATION_PAGE_SIZE
        ? undefined
        : last.offset + last.notifications.length,
  });
}

export function alertDestinationsQuery() {
  return queryOptions({
    queryKey: [...ALERTS_PREFIX, "destinations"],
    queryFn: fetchAlertDestinations,
  });
}

/** Every cached read about alerts: the list, each alert and its history. */
function invalidateAlerts(queryClient: QueryClient) {
  return queryClient.invalidateQueries({ queryKey: ALERTS_PREFIX });
}

/**
 * What a write answered, put where it is read: the alert itself, and its row
 * in every list page cached, so a toggle does not flick back while the list is
 * read again.
 */
function remember(queryClient: QueryClient, alert: Alert) {
  queryClient.setQueryData(alertQuery(alert.id).queryKey, alert);
  queryClient.setQueriesData<InfiniteData<AlertPage>>(
    { queryKey: [...ALERTS_PREFIX, "list"] },
    (cached) =>
      cached && {
        ...cached,
        pages: cached.pages.map((page) => ({
          ...page,
          alerts: page.alerts.map((row) =>
            row.id === alert.id
              ? {
                  ...row,
                  name: alert.name,
                  metric: alert.metric,
                  enabled: alert.enabled,
                }
              : row
          ),
        })),
      }
  );
}

export function useCreateAlert() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (draft: AlertDraft) => createAlert(draft),
    onSuccess: (alert) => {
      remember(queryClient, alert);
      return invalidateAlerts(queryClient);
    },
  });
}

export function useReplaceAlert() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      id,
      draft,
    }: {
      id: string;
      draft: AlertDraft & { expected_revision: number };
    }) => replaceAlert(id, draft),
    onSuccess: (alert) => {
      remember(queryClient, alert);
      return invalidateAlerts(queryClient);
    },
  });
}

/**
 * Turns an alert's checks on or off.
 *
 * A caller that holds the alert passes its revision. One that holds only the
 * list's summary, which carries none, leaves it out and the alert is read
 * first; a change made in between is refused with 409 rather than overwritten.
 */
export function useSetAlertEnabled() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      id,
      enabled,
      revision,
    }: {
      id: string;
      enabled: boolean;
      revision?: number;
    }) => {
      const expected = revision ?? (await fetchAlert(id)).revision;
      return setAlertEnabled(id, enabled, expected);
    },
    onSuccess: (alert) => {
      remember(queryClient, alert);
      return invalidateAlerts(queryClient);
    },
    // INVARIANT: after a conflict the cached revision is stale; re-read it so a retry sends the current one.
    onError: (error, { id }) =>
      isRevisionConflict(error)
        ? queryClient.invalidateQueries({ queryKey: alertQuery(id).queryKey })
        : undefined,
  });
}

export function useDeleteAlert() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => deleteAlert(id),
    onSuccess: (_done, id) => {
      queryClient.removeQueries({ queryKey: alertQuery(id).queryKey });
      queryClient.removeQueries({
        queryKey: alertNotificationsQuery(id).queryKey,
      });
      return invalidateAlerts(queryClient);
    },
  });
}

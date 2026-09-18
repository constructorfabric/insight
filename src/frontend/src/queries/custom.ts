import {
  infiniteQueryOptions,
  queryOptions,
  useMutation,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";

import type {
  ChatTurn,
  DefinitionKind,
  NamePage,
  PageRequest,
} from "@/api/custom-client";
import {
  deleteDefinition,
  fetchDashboard,
  fetchDashboardNames,
  fetchMetric,
  fetchMetricNames,
  fetchWidget,
  fetchWidgetNames,
  renameDefinition,
  runMetric,
  sendChat,
} from "@/api/custom-client";
import type { RunOptions } from "@/api/custom-client";

const WIDGET_QUERY_PREFIX = ["custom", "widget"] as const;
const NAME_PAGES_PREFIX = ["custom", "names"] as const;

/** How many names a catalogue asks for at a time. */
const PAGE_SIZE = 50;

/** The most the service will answer with, for the lists that want everything. */
const MAX_PAGE = 200;

const FETCH_NAMES: Record<
  DefinitionKind,
  (page: PageRequest) => Promise<NamePage>
> = {
  metrics: fetchMetricNames,
  widgets: fetchWidgetNames,
  dashboards: fetchDashboardNames,
};

/**
 * A catalogue, a page at a time.
 *
 * `total` counts the matches rather than the page, so the list can say what
 * is behind it and stop asking once it has them all.
 */
export function definitionPagesQuery(kind: DefinitionKind, search = "") {
  return infiniteQueryOptions({
    // The needle is part of the key, so a search is its own cached answer
    // rather than overwriting the list everyone else is reading.
    queryKey: [...NAME_PAGES_PREFIX, kind, search],
    queryFn: ({ pageParam }) =>
      FETCH_NAMES[kind]({ search, limit: PAGE_SIZE, offset: pageParam }),
    initialPageParam: 0,
    getNextPageParam: (last: NamePage, pages: NamePage[]) => {
      const read = pages.reduce((count, page) => count + page.names.length, 0);
      return read < last.total ? read : undefined;
    },
  });
}

/** Every dashboard in one answer, for the rail that lists them all. */
export function dashboardNamesQuery(search = "") {
  return queryOptions({
    queryKey: ["custom", "dashboard-names", search],
    queryFn: () => fetchDashboardNames({ search, limit: MAX_PAGE }),
  });
}

export function dashboardQuery(name: string) {
  return queryOptions({
    queryKey: ["custom", "dashboard", name],
    queryFn: () => fetchDashboard(name),
  });
}

export function metricQuery(name: string) {
  return queryOptions({
    queryKey: ["custom", "metric", name],
    queryFn: () => fetchMetric(name),
  });
}

export function widgetQuery(name: string) {
  return queryOptions({
    queryKey: [...WIDGET_QUERY_PREFIX, name],
    queryFn: () => fetchWidget(name),
  });
}

/**
 * One metric read over one window.
 *
 * The window, the zone and the bucket mode are all in the key: without them
 * switching from a year to yesterday serves the year's rows out of cache,
 * which is a wrong number rather than an error.
 */
export function metricResultQuery(name: string, options?: RunOptions) {
  return queryOptions({
    queryKey: [
      "custom",
      "metric-result",
      name,
      options?.range ?? null,
      options?.bucket ?? null,
    ],
    queryFn: () => runMetric(name, options),
  });
}

export function useRemoveDefinition() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ kind, name }: { kind: DefinitionKind; name: string }) =>
      deleteDefinition(kind, name),
    // Every catalogue and the pane read these lists.
    onSuccess: () => invalidateDashboardList(queryClient),
  });
}

export function useRenameDefinition() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      kind,
      name,
      to,
    }: {
      kind: DefinitionKind;
      name: string;
      to: string;
    }) => renameDefinition(kind, name, to),
    // A rename moves a body to a new key and rewrites its dependents, so
    // every cached definition is suspect, not just the lists.
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["custom"] }),
  });
}

export function useSendChat() {
  return useMutation({
    mutationFn: ({ message, history }: { message: string; history: ChatTurn[] }) =>
      sendChat(message, history),
  });
}

export function invalidateDashboardList(queryClient: QueryClient) {
  return Promise.all([
    queryClient.invalidateQueries({
      queryKey: ["custom", "dashboard-names"],
    }),
    // Every catalogue page reads these, and a chat that built a dashboard
    // built its metric and widgets too.
    queryClient.invalidateQueries({ queryKey: NAME_PAGES_PREFIX }),
  ]);
}

export function invalidateDashboardPage(
  queryClient: QueryClient,
  name: string
) {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: dashboardQuery(name).queryKey }),
    queryClient.invalidateQueries({ queryKey: WIDGET_QUERY_PREFIX }),
  ]);
}

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
  EditableKind,
  NamePage,
  PageRequest,
} from "@/api/custom-client";
import {
  deleteDataset,
  deleteDefinition,
  fetchDashboard,
  fetchDashboardNames,
  fetchDataset,
  fetchDatasetDependents,
  fetchDatasetNames,
  fetchDatasetRecords,
  fetchMetric,
  fetchMetricNames,
  fetchWidget,
  fetchWidgetNames,
  putDataset,
  putDefinition,
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

const DATASET_PREFIX = ["custom", "dataset"] as const;

const FETCH_NAMES: Record<
  EditableKind,
  (page: PageRequest) => Promise<NamePage>
> = {
  metrics: fetchMetricNames,
  widgets: fetchWidgetNames,
  dashboards: fetchDashboardNames,
  datasets: fetchDatasetNames,
};

/**
 * A catalogue, a page at a time.
 *
 * `total` counts the matches rather than the page, so the list can say what
 * is behind it and stop asking once it has them all.
 */
export function definitionPagesQuery(kind: EditableKind, search = "") {
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

// Each kind hands its body back in its own envelope; the editor writes the body,
// so what it opens must be what a save would send.
export function definitionBodyQuery(kind: EditableKind, name: string) {
  return queryOptions({
    queryKey: ["custom", "body", kind, name],
    // INVARIANT: the editor waits for an answer fetched after it mounted and
    // seeds itself from that one. The client keeps answers fresh for an hour,
    // so without this a reopened editor would wait for a fetch that never comes.
    refetchOnMount: "always",
    // Once seeded, a refetch underneath it would be neither shown nor wanted.
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    queryFn: async (): Promise<Record<string, unknown>> => {
      if (kind === "datasets") {
        const dataset = await fetchDataset(name);
        return dataset.declaration as unknown as Record<string, unknown>;
      }
      if (kind === "metrics") {
        const metric = await fetchMetric(name);
        return metric.definition as unknown as Record<string, unknown>;
      }
      const body =
        kind === "widgets"
          ? await fetchWidget(name)
          : await fetchDashboard(name);
      return body as unknown as Record<string, unknown>;
    },
  });
}

// Every name of a kind in one answer, for a form to offer a reference from.
export function catalogueNamesQuery(kind: EditableKind) {
  return queryOptions({
    queryKey: ["custom", "all-names", kind],
    queryFn: () => FETCH_NAMES[kind]({ limit: MAX_PAGE }),
    select: (page: NamePage) => page.names,
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

export function datasetQuery(name: string) {
  return queryOptions({
    queryKey: [...DATASET_PREFIX, name],
    queryFn: () => fetchDataset(name),
  });
}

/** The latest records a dataset holds, as a reader sees them on its page. */
export function datasetRecordsQuery(name: string) {
  return queryOptions({
    queryKey: [...DATASET_PREFIX, name, "records"],
    queryFn: () => fetchDatasetRecords(name),
  });
}

/** Every metric that reads this dataset, which a removal would break. */
export function datasetDependentsQuery(name: string) {
  return queryOptions({
    queryKey: [...DATASET_PREFIX, name, "dependents"],
    queryFn: () => fetchDatasetDependents(name),
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

/**
 * Takes a dataset away, with the records it holds.
 *
 * Every catalogue is invalidated, not only the datasets one: a metric that
 * read it is now unreadable, whatever the list still says.
 */
export function useRemoveDataset() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (name: string) => deleteDataset(name),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["custom"] }),
  });
}

/**
 * Stores a definition of any kind, as the document the editor holds says it.
 *
 * A dataset is written through its own path because it answers with what was
 * stored and has a lifecycle behind it; the rest share one.
 */
export function useStoreDefinition() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      kind,
      name,
      body,
    }: {
      kind: EditableKind;
      name: string;
      body: unknown;
    }) =>
      kind === "datasets"
        ? putDataset(name, body).then(() => undefined)
        : putDefinition(kind, name, body),
    // A stored definition changes what every other one may read, and the rail
    // lists what is there.
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["custom"] }),
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
    mutationFn: ({
      message,
      history,
    }: {
      message: string;
      history: ChatTurn[];
    }) => sendChat(message, history),
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

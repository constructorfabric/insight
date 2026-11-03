import {
  infiniteQueryOptions,
  queryOptions,
  useMutation,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";

import type {
  ChatTurn,
  DashboardAccess,
  DashboardListed,
  DatasetListed,
  DashboardRead,
  DrilldownPage,
  DrilldownRequest,
  RecordPage,
  DefinitionKind,
  EditableKind,
  FolderFilter,
  Listed,
  MetricListed,
  WidgetListed,
  GrantLevel,
  GrantTarget,
  Grantee,
  ListedPage,
  OrphanedDashboard,
  OrphanedFolder,
  OrphanKind,
  OwnedFilter,
  PageRequest,
} from "@/api/custom-client";
import {
  addToFolder,
  createDefinition,
  deleteDataset,
  deleteDefinition,
  deleteFolder,
  deleteGrant,
  duplicateDashboard,
  fetchDashboardDrilldownPage,
  fetchDashboardRead,
  fetchDashboardWidgets,
  fetchDashboards,
  fetchDataset,
  fetchDatasetDependents,
  fetchDatasetNames,
  fetchDatasetRecords,
  fetchDefinition,
  fetchDependents,
  fetchFolders,
  fetchGrants,
  fetchMetric,
  fetchMetrics,
  fetchOrphanedDashboards,
  fetchOrphanedFolders,
  fetchPins,
  fetchTags,
  fetchWidget,
  fetchWidgets,
  pinDashboard,
  putDataset,
  putDefinition,
  putGrant,
  removeFromFolder,
  renameDashboard,
  renameFolder,
  runDashboardWidget,
  runMetric,
  sendChat,
  setDashboardTags,
  transferOrphan,
  unpinDashboard,
} from "@/api/custom-client";
import type { RunOptions } from "@/api/custom-client";

const WIDGET_QUERY_PREFIX = ["custom", "widget"] as const;
const WIDGET_RESULT_PREFIX = ["custom", "dashboard-widget-result"] as const;
const NAME_PAGES_PREFIX = ["custom", "names"] as const;
const FOLDERS_PREFIX = ["custom", "folders"] as const;
const TAGS_PREFIX = ["custom", "tags"] as const;
const PINS_PREFIX = ["custom", "pins"] as const;

/** How many names a catalogue asks for at a time. */
const PAGE_SIZE = 50;

/** The most the service will answer with, for the lists that want everything. */
const MAX_PAGE = 200;

const DATASET_PREFIX = ["custom", "dataset"] as const;
const GRANTS_PREFIX = ["custom", "grants"] as const;
const ORPHANED_PREFIX = ["custom", "orphaned"] as const;
const DASHBOARD_PREFIX = ["custom", "dashboard"] as const;

interface Catalogued {
  metrics: MetricListed;
  widgets: WidgetListed;
  dashboards: DashboardListed;
  datasets: DatasetListed;
}

async function fetchDatasetPage(
  page: PageRequest
): Promise<ListedPage<DatasetListed>> {
  const read = await fetchDatasetNames(page);

  return {
    items: read.items.map((item) => ({ ...item, id: item.name })),
    total: read.total,
  };
}

const FETCH_PAGE: {
  [K in EditableKind]: (
    page: PageRequest
  ) => Promise<ListedPage<Catalogued[K]>>;
} = {
  metrics: fetchMetrics,
  widgets: fetchWidgets,
  dashboards: fetchDashboards,
  datasets: fetchDatasetPage,
};

/** What narrows a catalogue besides a search. Only dashboards are narrowed. */
export interface Narrowing {
  folder?: FolderFilter;
  tags?: string[];
  owned?: OwnedFilter;
}

/**
 * A catalogue, a page at a time.
 *
 * `total` counts the matches rather than the page, so the list can say what
 * is behind it and stop asking once it has them all.
 */
export function definitionPagesQuery<K extends EditableKind>(
  kind: K,
  search = "",
  { folder, tags = [], owned }: Narrowing = {}
) {
  return infiniteQueryOptions({
    // The needle is part of the key, so a search is its own cached answer
    // rather than overwriting the list everyone else is reading.
    queryKey: [
      ...NAME_PAGES_PREFIX,
      kind,
      search,
      folder ?? null,
      tags,
      owned ?? null,
    ],
    queryFn: ({ pageParam }) =>
      FETCH_PAGE[kind]({
        search,
        limit: PAGE_SIZE,
        offset: pageParam,
        ...(folder ? { folder } : {}),
        ...(tags.length > 0 ? { tags } : {}),
        ...(owned ? { owned } : {}),
      }),
    initialPageParam: 0,
    getNextPageParam: (
      last: ListedPage<Catalogued[K]>,
      pages: ListedPage<Catalogued[K]>[]
    ) => {
      const read = pages.reduce((count, page) => count + page.items.length, 0);
      return read < last.total ? read : undefined;
    },
  });
}

/** The caller's own dashboards, most recently changed first. */
export function recentDashboardsQuery(count: number) {
  return queryOptions({
    queryKey: [...NAME_PAGES_PREFIX, "dashboards", "recent", count],
    queryFn: () => fetchDashboards({ sort: "recent", limit: count }),
  });
}

export interface EditedDefinition {
  name: string;
  document: Record<string, unknown>;
  /** Only a dashboard has one. */
  access?: DashboardAccess;
}

// Each kind hands its body back in its own envelope; the editor writes the body,
// so what it opens must be what a save would send.
export function definitionBodyQuery(kind: EditableKind, id: string) {
  return queryOptions({
    queryKey: ["custom", "body", kind, id],
    // INVARIANT: the editor waits for an answer fetched after it mounted and
    // seeds itself from that one. The client keeps answers fresh for an hour,
    // so without this a reopened editor would wait for a fetch that never comes.
    refetchOnMount: "always",
    // Once seeded, a refetch underneath it would be neither shown nor wanted.
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    queryFn: async (): Promise<EditedDefinition> => {
      if (kind === "datasets") {
        const dataset = await fetchDataset(id);
        return {
          name: dataset.name,
          document: dataset.declaration as unknown as Record<string, unknown>,
        };
      }
      const read = await fetchDefinition<Record<string, unknown>>(kind, id);
      const edited: EditedDefinition = { name: read.name, document: read.body };
      return read.access ? { ...edited, access: read.access } : edited;
    },
  });
}

export function catalogueQuery(kind: EditableKind) {
  return queryOptions({
    queryKey: ["custom", "catalogue", kind],
    queryFn: () => FETCH_PAGE[kind]({ limit: MAX_PAGE }),
    select: (page: ListedPage): Listed[] => page.items,
  });
}

export function dashboardReadQuery(id: string) {
  return queryOptions({
    queryKey: [...DASHBOARD_PREFIX, id],
    queryFn: () => fetchDashboardRead(id),
  });
}

export function dashboardQuery(id: string) {
  return queryOptions({
    ...dashboardReadQuery(id),
    select: (read: DashboardRead) => read.body,
  });
}

export function dashboardTagsQuery(id: string) {
  return queryOptions({
    ...dashboardReadQuery(id),
    select: (read: DashboardRead) => read.tags,
  });
}

export function tagsQuery() {
  return queryOptions({
    queryKey: TAGS_PREFIX,
    queryFn: fetchTags,
  });
}

export function pinsQuery() {
  return queryOptions({
    queryKey: PINS_PREFIX,
    queryFn: fetchPins,
  });
}

export function foldersQuery() {
  return queryOptions({
    queryKey: FOLDERS_PREFIX,
    queryFn: fetchFolders,
  });
}

export function grantsQuery(target: GrantTarget) {
  return queryOptions({
    queryKey: [...GRANTS_PREFIX, target.kind, target.id],
    queryFn: () => fetchGrants(target),
  });
}

const FETCH_ORPHANS: {
  dashboards: typeof fetchOrphanedDashboards;
  folders: typeof fetchOrphanedFolders;
} = {
  dashboards: fetchOrphanedDashboards,
  folders: fetchOrphanedFolders,
};

export function orphanedPagesQuery<K extends OrphanKind>(kind: K) {
  type Orphan = K extends "dashboards" ? OrphanedDashboard : OrphanedFolder;

  return infiniteQueryOptions({
    queryKey: [...ORPHANED_PREFIX, kind],
    queryFn: ({ pageParam }) =>
      FETCH_ORPHANS[kind]({ limit: PAGE_SIZE, offset: pageParam }) as Promise<
        ListedPage<Orphan>
      >,
    initialPageParam: 0,
    getNextPageParam: (
      last: ListedPage<Orphan>,
      pages: ListedPage<Orphan>[]
    ) => {
      const read = pages.reduce((count, page) => count + page.items.length, 0);
      return read < last.total ? read : undefined;
    },
  });
}

export function dashboardWidgetsQuery(id: string) {
  return queryOptions({
    queryKey: [...WIDGET_QUERY_PREFIX, "of", id],
    queryFn: () => fetchDashboardWidgets(id),
  });
}

export function metricQuery(id: string) {
  return queryOptions({
    queryKey: ["custom", "metric", id],
    queryFn: () => fetchMetric(id),
  });
}

export function datasetQuery(name: string) {
  return queryOptions({
    queryKey: [...DATASET_PREFIX, name],
    queryFn: () => fetchDataset(name),
  });
}

/** The latest records a dataset holds, as a reader sees them on its page. */
/**
 * What a page of records is assumed to hold before one has come back.
 *
 * INVARIANT: only the first read uses it. Every page after steps by the size
 * the service reports, because the cap is an installation's setting.
 */
export const PREVIEW_ROWS = 20;

export function datasetRecordsQuery(name: string, page: RecordPage) {
  return queryOptions({
    queryKey: [
      ...DATASET_PREFIX,
      name,
      "records",
      page.limit ?? null,
      page.offset ?? 0,
      page.orderBy ?? null,
      page.descending ?? false,
    ],
    queryFn: () => fetchDatasetRecords(name, page),
    // A page read while the reader is on the one before it should not blank
    // the table: the old page stays until the new one is in.
    placeholderData: (previous) => previous,
  });
}

/** Every definition that names this one, which a removal would break. */
export function dependentsQuery(kind: DefinitionKind, id: string) {
  return queryOptions({
    queryKey: ["custom", "dependents", kind, id],
    queryFn: () => fetchDependents(kind, id),
  });
}

export function datasetDependentsQuery(name: string) {
  return queryOptions({
    queryKey: [...DATASET_PREFIX, name, "dependents"],
    queryFn: () => fetchDatasetDependents(name),
  });
}

export function widgetQuery(id: string) {
  return queryOptions({
    queryKey: [...WIDGET_QUERY_PREFIX, id],
    queryFn: () => fetchWidget(id),
  });
}

/**
 * One metric read over one window.
 *
 * The window, the zone and the bucket mode are all in the key: without them
 * switching from a year to yesterday serves the year's rows out of cache,
 * which is a wrong number rather than an error.
 */
export function metricResultQuery(id: string, options?: RunOptions) {
  return queryOptions({
    queryKey: [
      "custom",
      "metric-result",
      id,
      options?.range ?? null,
      options?.bucket ?? null,
    ],
    queryFn: () => runMetric(id, options),
  });
}

export function dashboardWidgetResultQuery(
  dashboard: string,
  widget: string,
  options?: RunOptions
) {
  return queryOptions({
    queryKey: [
      ...WIDGET_RESULT_PREFIX,
      dashboard,
      widget,
      options?.range ?? null,
      options?.bucket ?? null,
    ],
    queryFn: () => runDashboardWidget(dashboard, widget, options),
  });
}

/** How many rows one page of a drilldown asks for. */
const DRILLDOWN_PAGE_ROWS = 100;

/**
 * The rows behind a dashboard's widget, one ordered page at a time.
 *
 * The window and the order are in the key: a re-order is its own answer,
 * and the previous one is kept on screen until it arrives rather than
 * replaced by a spinner under the header the reader just clicked.
 */
export function drilldownPagesQuery(
  dashboard: string,
  widget: string,
  view: Omit<DrilldownRequest, "cursor" | "limit">
) {
  return infiniteQueryOptions({
    queryKey: [
      "custom",
      "drilldown",
      dashboard,
      widget,
      view.range ?? null,
      view.bucket ?? null,
      view.sort ?? null,
    ],
    queryFn: ({ pageParam, signal }) =>
      fetchDashboardDrilldownPage(
        dashboard,
        widget,
        { ...view, limit: DRILLDOWN_PAGE_ROWS, cursor: pageParam },
        signal
      ),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last: DrilldownPage) => last.next_cursor ?? undefined,
    placeholderData: (previous) => previous,
  });
}

export function useRemoveDefinition() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ kind, id }: { kind: DefinitionKind; id: string }) =>
      deleteDefinition(kind, id),
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

export interface StoredDefinition {
  kind: EditableKind;
  id?: string;
  name: string;
  body: unknown;
}

async function storeDefinition({
  kind,
  id,
  name,
  body,
}: StoredDefinition): Promise<string> {
  if (kind === "datasets") {
    await putDataset(name, body);
    return name;
  }
  if (id === undefined) return createDefinition(kind, name, body);

  await putDefinition(kind, id, name, body);
  return id;
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
    mutationFn: storeDefinition,
    // A stored definition changes what every other one may read, and the rail
    // lists what is there.
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["custom"] }),
  });
}

export function useRenameDashboard() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, to }: { id: string; to: string }) =>
      renameDashboard(id, to),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["custom"] }),
  });
}

export async function grantViewers(
  target: GrantTarget,
  grantees: readonly Grantee[]
): Promise<{ refused: unknown } | null> {
  const shares = await Promise.allSettled(
    grantees.map((grantee) => putGrant(target, grantee, "viewer"))
  );
  const refused = shares.find(
    (share): share is PromiseRejectedResult => share.status === "rejected"
  );

  return refused ? { refused: refused.reason } : null;
}

export function useRenameFolder() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, name }: { id: string; name: string }) =>
      renameFolder(id, name),
    onSettled: () =>
      Promise.all([
        invalidateDashboardList(queryClient),
        queryClient.invalidateQueries({ queryKey: DASHBOARD_PREFIX }),
      ]),
  });
}

export function useDeleteFolder() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (id: string) => deleteFolder(id),
    onSettled: () =>
      Promise.all([
        invalidateDashboardList(queryClient),
        queryClient.invalidateQueries({ queryKey: DASHBOARD_PREFIX }),
      ]),
  });
}

export interface FolderEntry {
  folder: string;
  dashboard: string;
}

function useFolderEntry<T>(
  change: (folder: string, dashboard: string) => Promise<T>
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ folder, dashboard }: FolderEntry) =>
      change(folder, dashboard),
    onSettled: (_done, _error, { dashboard }) =>
      Promise.all([
        invalidateDashboardList(queryClient),
        queryClient.invalidateQueries({
          queryKey: dashboardReadQuery(dashboard).queryKey,
        }),
      ]),
  });
}

export function useAddToFolder() {
  return useFolderEntry(addToFolder);
}

export function useRemoveFromFolder() {
  return useFolderEntry(removeFromFolder);
}

export function useSetDashboardTags() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, tags }: { id: string; tags: string[] }) =>
      setDashboardTags(id, tags),
    onSettled: (_done, _error, { id }) =>
      Promise.all([
        invalidateDashboardList(queryClient),
        queryClient.invalidateQueries({
          queryKey: dashboardReadQuery(id).queryKey,
        }),
      ]),
  });
}

export function useDuplicateDashboard() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, to }: { id: string; to: string }) =>
      duplicateDashboard(id, to),
    onSettled: () => invalidateDashboardList(queryClient),
  });
}

export function useSetPinned() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, pinned }: { id: string; pinned: boolean }) =>
      pinned ? pinDashboard(id) : unpinDashboard(id),
    onSettled: () => queryClient.invalidateQueries({ queryKey: PINS_PREFIX }),
  });
}

/** A grant can change the caller's own access too, so their lists and reads are refreshed. */
function invalidateGrants(queryClient: QueryClient, target: GrantTarget) {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: grantsQuery(target).queryKey }),
    queryClient.invalidateQueries({ queryKey: DASHBOARD_PREFIX }),
    invalidateDashboardList(queryClient),
  ]);
}

export function usePutGrant(target: GrantTarget) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ grantee, level }: { grantee: Grantee; level: GrantLevel }) =>
      putGrant(target, grantee, level),
    onSettled: () => invalidateGrants(queryClient, target),
  });
}

export function useDeleteGrant(target: GrantTarget) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (grantee: Grantee) => deleteGrant(target, grantee),
    onSettled: () => invalidateGrants(queryClient, target),
  });
}

export function useTransferOrphan() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      kind,
      id,
      to,
    }: {
      kind: OrphanKind;
      id: string;
      to: string;
    }) => transferOrphan(kind, id, to),
    onSettled: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: ORPHANED_PREFIX }),
        queryClient.invalidateQueries({ queryKey: DASHBOARD_PREFIX }),
        invalidateDashboardList(queryClient),
      ]),
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
  // Every catalogue page reads these, and a chat that built a dashboard
  // built its metric and widgets too.
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: NAME_PAGES_PREFIX }),
    queryClient.invalidateQueries({ queryKey: FOLDERS_PREFIX }),
    queryClient.invalidateQueries({ queryKey: TAGS_PREFIX }),
    queryClient.invalidateQueries({ queryKey: PINS_PREFIX }),
  ]);
}

export function invalidateDashboardPage(queryClient: QueryClient, id: string) {
  return Promise.all([
    queryClient.invalidateQueries({
      queryKey: dashboardReadQuery(id).queryKey,
    }),
    queryClient.invalidateQueries({ queryKey: WIDGET_QUERY_PREFIX }),
    queryClient.invalidateQueries({ queryKey: WIDGET_RESULT_PREFIX }),
  ]);
}

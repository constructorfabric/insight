import { fetchWithAuth } from "@/api/fetch-with-auth";

export const BASE =
  (import.meta.env.VITE_API_BASE_V3 as string | undefined) ?? "/api/v3/v1";

export type * from "@/api/custom-types";
import type {
  Dashboard,
  DashboardListed,
  DashboardShare,
  Dataset,
  DatasetRecords,
  Dependents,
  DrilldownPage,
  DrilldownRequest,
  Listed,
  ListedPage,
  RecordPage,
  MetricResult,
  OrphanedDashboard,
  StoredMetric,
  StoredWidget,
  Widget,
  ChatReply,
  ChatTurn,
  DashboardRead,
  DashboardResponse,
  DefinitionResponse,
  Folder,
  FolderFilter,
  FolderList,
  MetricDefinition,
  PinList,
  TableList,
  TableSchema,
  TagList,
} from "@/api/custom-types";

export const JSON_HEADERS = { "Content-Type": "application/json" };

export class CustomApiError extends Error {
  status: number;
  body: unknown;

  constructor(status: number, body: unknown) {
    super(`Custom API ${status}`);
    this.name = "CustomApiError";
    this.status = status;
    this.body = body;
  }
}

/** Refuses with what the service said, or with nothing when it said nothing. */
export async function ensureOk(res: Response): Promise<void> {
  if (!res.ok) {
    throw new CustomApiError(res.status, await res.json().catch(() => null));
  }
}

export async function readJson<T>(res: Response): Promise<T> {
  await ensureOk(res);
  return (await res.json()) as T;
}

/**
 * One path segment: a definition's id, a dataset's, database's or table's name.
 *
 * INVARIANT: an empty name is never a request. It would address the
 * collection above it with a trailing slash, which no service routes — and an
 * unrouted path answers 401, which reads at the caller as a session that
 * ended rather than as the bug it is.
 */
function named(name: string): string {
  if (name === "") {
    throw new CustomApiError(400, { detail: "A name is required." });
  }
  return encodeURIComponent(name);
}

/** What a definition is, in the API's path segments. */
export type DefinitionKind = "metrics" | "widgets" | "dashboards";

/** Every kind the Custom zone holds, including the one with a lifecycle. */
export type EditableKind = DefinitionKind | "datasets";

/**
 * Removes a definition.
 *
 * Refused while something still draws it — a widget's metric, a dashboard's
 * widget — and the reply names what does, which is what the caller is shown.
 */
export async function deleteDefinition(
  kind: DefinitionKind,
  id: string
): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/${kind}/${named(id)}`, {
    method: "DELETE",
  });
  await ensureOk(res);
}

export async function fetchDefinition<T = unknown>(
  kind: DefinitionKind,
  id: string
): Promise<DefinitionResponse<T>> {
  const res = await fetchWithAuth(`${BASE}/${kind}/${named(id)}`);
  return readJson<DefinitionResponse<T>>(res);
}

/** Stores a new definition and answers with the id the service gave it. */
export async function createDefinition(
  kind: DefinitionKind,
  name: string,
  body: unknown
): Promise<string> {
  const res = await fetchWithAuth(`${BASE}/${kind}`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify({ name, body }),
  });
  const made = await readJson<{ id: string }>(res);

  return made.id;
}

/** Replaces what a definition is called and what it holds. */
export async function putDefinition(
  kind: DefinitionKind,
  id: string,
  name: string,
  body: unknown
): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/${kind}/${named(id)}`, {
    method: "PUT",
    headers: JSON_HEADERS,
    body: JSON.stringify({ name, body }),
  });
  await ensureOk(res);
}

/**
 * Gives a definition a new name and keeps what it holds.
 *
 * Nothing points at a definition by its name, so nothing else changes.
 */
export async function renameDefinition(
  kind: DefinitionKind,
  id: string,
  to: string
): Promise<void> {
  const stored = await fetchDefinition(kind, id);
  await putDefinition(kind, id, to, stored.body);
}

/** The longest name a metric, widget or dashboard may be given. */
export const DEFINITION_NAME_MAX = 128;

/** Which dashboards a list narrows to by who owns them. */
export type OwnedFilter = "me" | "shared";

/** Which slice of a catalogue to ask for. The service caps limit at 200. */
export interface PageRequest {
  search?: string;
  limit?: number;
  offset?: number;
  folder?: FolderFilter;
  tags?: string[];
  owned?: OwnedFilter;
}

function pageQuery({
  search = "",
  limit,
  offset,
  folder,
  tags = [],
  owned,
}: PageRequest): string {
  const query = new URLSearchParams();
  if (search) query.set("q", search);
  if (limit !== undefined) query.set("limit", String(limit));
  if (offset) query.set("offset", String(offset));
  if (folder) query.set("folder", folder);
  for (const tag of tags) query.append("tag", tag);
  if (owned) query.set("owned", owned);
  const asked = query.toString();

  return asked ? `?${asked}` : "";
}

export async function fetchDashboards(
  page: PageRequest = {}
): Promise<ListedPage<DashboardListed>> {
  const res = await fetchWithAuth(`${BASE}/dashboards${pageQuery(page)}`);
  return readJson<ListedPage<DashboardListed>>(res);
}

export async function fetchDashboardFolder(id: string): Promise<Folder | null> {
  const read = await fetchDefinition<Dashboard>("dashboards", id);

  return read.folder ?? null;
}

export async function fetchDashboardRead(id: string): Promise<DashboardRead> {
  const res = await fetchWithAuth(`${BASE}/dashboards/${named(id)}`);
  const read = await readJson<DashboardResponse>(res);
  const shown: DashboardRead = {
    name: read.name,
    body: read.body,
    tags: read.tags ?? [],
    ownerPersonId: read.owner_person_id,
    access: read.access,
  };

  return read.updated_at ? { ...shown, updatedAt: read.updated_at } : shown;
}

export const TAG_NAME_MAX = 32;
export const DASHBOARD_TAGS_MAX = 10;

export async function fetchTags(): Promise<TagList> {
  const res = await fetchWithAuth(`${BASE}/tags`);
  return readJson<TagList>(res);
}

export async function setDashboardTags(
  id: string,
  tags: string[]
): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/dashboards/${named(id)}/tags`, {
    method: "PUT",
    headers: JSON_HEADERS,
    body: JSON.stringify({ tags }),
  });
  await ensureOk(res);
}

/** Copies a dashboard under a new name and answers with the copy's id. */
export async function duplicateDashboard(
  id: string,
  name: string
): Promise<string> {
  const res = await fetchWithAuth(`${BASE}/dashboards/${named(id)}/duplicate`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify({ name }),
  });
  const made = await readJson<{ id: string }>(res);

  return made.id;
}

export async function fetchPins(): Promise<Listed[]> {
  const res = await fetchWithAuth(`${BASE}/pins`);
  const read = await readJson<PinList>(res);

  return read.pins;
}

export async function pinDashboard(id: string): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/pins/${named(id)}`, {
    method: "PUT",
  });
  await ensureOk(res);
}

export async function unpinDashboard(id: string): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/pins/${named(id)}`, {
    method: "DELETE",
  });
  await ensureOk(res);
}

/** Who a dashboard is shared with. Only its owner may ask. */
export async function fetchShares(id: string): Promise<DashboardShare[]> {
  const res = await fetchWithAuth(`${BASE}/dashboards/${named(id)}/shares`);
  const read = await readJson<{ items: DashboardShare[] }>(res);

  return read.items;
}

/** Shares a dashboard for viewing with one administrator; sharing twice changes nothing. */
export async function shareDashboard(
  id: string,
  personId: string
): Promise<void> {
  const res = await fetchWithAuth(
    `${BASE}/dashboards/${named(id)}/shares/${named(personId)}`,
    { method: "PUT" }
  );
  await ensureOk(res);
}

export async function unshareDashboard(
  id: string,
  personId: string
): Promise<void> {
  const res = await fetchWithAuth(
    `${BASE}/dashboards/${named(id)}/shares/${named(personId)}`,
    { method: "DELETE" }
  );
  await ensureOk(res);
}

/** One page of the dashboards whose owner is no longer an administrator. */
export async function fetchOrphanedDashboards(
  page: Pick<PageRequest, "limit" | "offset"> = {}
): Promise<ListedPage<OrphanedDashboard>> {
  const res = await fetchWithAuth(
    `${BASE}/orphaned-dashboards${pageQuery(page)}`
  );
  return readJson<ListedPage<OrphanedDashboard>>(res);
}

/** Makes an administrator the owner of an orphaned dashboard. */
export async function transferDashboard(
  id: string,
  to: string
): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/dashboards/${named(id)}/transfer`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify({ to }),
  });
  await ensureOk(res);
}

export const FOLDER_NAME_MAX = 64;

export async function fetchFolders(): Promise<FolderList> {
  const res = await fetchWithAuth(`${BASE}/folders`);
  return readJson<FolderList>(res);
}

export async function createFolder(name: string): Promise<Folder> {
  const res = await fetchWithAuth(`${BASE}/folders`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify({ name }),
  });
  return readJson<Folder>(res);
}

export async function renameFolder(id: string, name: string): Promise<Folder> {
  const res = await fetchWithAuth(`${BASE}/folders/${named(id)}`, {
    method: "PATCH",
    headers: JSON_HEADERS,
    body: JSON.stringify({ name }),
  });
  return readJson<Folder>(res);
}

export async function deleteFolder(id: string): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/folders/${named(id)}`, {
    method: "DELETE",
  });
  await ensureOk(res);
}

export async function moveDashboard(
  id: string,
  folder: string | null
): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/dashboards/${named(id)}/folder`, {
    method: "PUT",
    headers: JSON_HEADERS,
    body: JSON.stringify({ folder }),
  });
  await ensureOk(res);
}

export async function fetchMetrics(
  page: PageRequest = {}
): Promise<ListedPage> {
  const res = await fetchWithAuth(`${BASE}/metrics${pageQuery(page)}`);
  return readJson<ListedPage>(res);
}

export async function fetchMetric(id: string): Promise<StoredMetric> {
  const read = await fetchDefinition<MetricDefinition>("metrics", id);

  return { name: read.name, definition: read.body, clock: read.clock };
}

export async function fetchWidgets(
  page: PageRequest = {}
): Promise<ListedPage> {
  const res = await fetchWithAuth(`${BASE}/widgets${pageQuery(page)}`);
  return readJson<ListedPage>(res);
}

export async function fetchWidget(id: string): Promise<StoredWidget> {
  const read = await fetchDefinition<Widget>("widgets", id);

  return { name: read.name, widget: read.body };
}

/** One page of a catalogue of names, as the dataset catalogue answers. */
export interface NamePage {
  names: string[];
  total: number;
}

export async function fetchDatasetNames(
  page: PageRequest = {}
): Promise<NamePage> {
  const res = await fetchWithAuth(`${BASE}/datasets${pageQuery(page)}`);
  return readJson<NamePage>(res);
}

export async function fetchDataset(name: string): Promise<Dataset> {
  const res = await fetchWithAuth(`${BASE}/datasets/${named(name)}`);
  return readJson<Dataset>(res);
}

/** Every warehouse table a metric may read, or one database's. */
export async function fetchTables(database?: string): Promise<TableList> {
  const query = database ? `?database=${encodeURIComponent(database)}` : "";
  const res = await fetchWithAuth(`${BASE}/tables${query}`);
  return readJson<TableList>(res);
}

export async function fetchTable(
  database: string,
  table: string
): Promise<TableSchema> {
  const res = await fetchWithAuth(
    `${BASE}/tables/${named(database)}/${named(table)}`
  );
  return readJson<TableSchema>(res);
}

/** One page of the records a dataset holds, sized by the service. */
export async function fetchDatasetRecords(
  name: string,
  page: RecordPage
): Promise<DatasetRecords> {
  const query = new URLSearchParams();
  if (page.limit !== undefined) query.set("limit", String(page.limit));
  if (page.offset) query.set("offset", String(page.offset));
  // The direction stands on its own: with no field named the service orders
  // by the instant a record arrived, and honours the direction there too.
  if (page.orderBy) query.set("order_by", page.orderBy);
  if (page.descending !== undefined) {
    query.set("direction", page.descending ? "desc" : "asc");
  }
  const res = await fetchWithAuth(
    `${BASE}/datasets/${named(name)}/records?${query}`
  );

  return readJson<DatasetRecords>(res);
}

/** Every definition that names this one: what a removal would break. */
export async function fetchDependents(
  kind: DefinitionKind,
  id: string
): Promise<Dependents> {
  const res = await fetchWithAuth(`${BASE}/${kind}/${named(id)}/dependents`);
  return readJson<Dependents>(res);
}

/**
 * Every metric that reads this dataset.
 *
 * An exact lookup over what each body names, so a metric merely mentioning
 * the name in a label is not one of them.
 */
export async function fetchDatasetDependents(name: string): Promise<string[]> {
  const res = await fetchWithAuth(`${BASE}/datasets/${named(name)}/dependents`);
  const read = await readJson<{ metrics: string[] }>(res);

  return read.metrics;
}

/**
 * Declares a dataset, or replaces the declaration of one that stands.
 *
 * A replacement touches no record: it is refused where a metric reading the
 * dataset would break or quietly start answering something else.
 */
export async function putDataset(
  name: string,
  declaration: unknown
): Promise<Dataset> {
  const res = await fetchWithAuth(`${BASE}/datasets/${named(name)}`, {
    method: "PUT",
    headers: JSON_HEADERS,
    body: JSON.stringify(declaration),
  });
  return readJson<Dataset>(res);
}

/**
 * Takes a dataset away, with the records it holds.
 *
 * Refused while a metric reads it, and the reply names every one.
 */
export async function deleteDataset(name: string): Promise<void> {
  const res = await fetchWithAuth(`${BASE}/datasets/${named(name)}`, {
    method: "DELETE",
  });
  await ensureOk(res);
}

/**
 * What a reader asked one run for. Every field is optional and an absent one
 * means the server's own default: no window, UTC, bucketed.
 */
export interface RunOptions {
  range?: string;
  bucket?: boolean;
}

export async function runMetric(
  id: string,
  options?: RunOptions
): Promise<MetricResult> {
  const res = await fetchWithAuth(`${BASE}/metrics/${named(id)}/run`, {
    method: "POST",
    ...(options
      ? { headers: JSON_HEADERS, body: JSON.stringify(options) }
      : {}),
  });
  return readJson<MetricResult>(res);
}

/** One ordered page of a metric's rows. The signal cancels a page the reader has moved past. */
export async function fetchDrilldownPage(
  id: string,
  request: DrilldownRequest,
  signal?: AbortSignal
): Promise<DrilldownPage> {
  const res = await fetchWithAuth(`${BASE}/metrics/${named(id)}/drilldown`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(request),
    signal,
  });
  return readJson<DrilldownPage>(res);
}

export async function sendChat(
  message: string,
  history: ChatTurn[] = []
): Promise<ChatReply> {
  const res = await fetchWithAuth(`${BASE}/chat`, {
    method: "POST",
    headers: JSON_HEADERS,
    // The service keeps no session, so the thread travels with every turn.
    body: JSON.stringify({ message, history }),
  });
  return readJson<ChatReply>(res);
}

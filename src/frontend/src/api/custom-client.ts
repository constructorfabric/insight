import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_API_BASE_V3 as string | undefined) ?? "/api/v3/v1";

export type * from "@/api/custom-types";
import type {
  Dashboard,
  Dataset,
  DatasetRecords,
  Holder,
  RecordPage,
  MetricResult,
  StoredMetric,
  Widget,
  ChatReply,
  ChatTurn,
  DefinitionResponse,
  MetricDefinition,
} from "@/api/custom-types";

const JSON_HEADERS = { "Content-Type": "application/json" };

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
async function ensureOk(res: Response): Promise<void> {
  if (!res.ok) {
    throw new CustomApiError(res.status, await res.json().catch(() => null));
  }
}

async function readJson<T>(res: Response): Promise<T> {
  await ensureOk(res);
  return (await res.json()) as T;
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
  name: string
): Promise<void> {
  const res = await fetchWithAuth(
    `${BASE}/${kind}/${encodeURIComponent(name)}`,
    {
      method: "DELETE",
    }
  );
  await ensureOk(res);
}

/** The new name, and what the service pointed at it. */
export interface Renamed {
  name: string;
  rewritten: string[];
}

/**
 * Renames a definition, and everything that named the old one.
 *
 * Refused when the new name is taken — a rename that silently replaced
 * another definition would lose it.
 */
export async function renameDefinition(
  kind: DefinitionKind,
  name: string,
  to: string
): Promise<Renamed> {
  const res = await fetchWithAuth(
    `${BASE}/${kind}/${encodeURIComponent(name)}/rename`,
    { method: "POST", headers: JSON_HEADERS, body: JSON.stringify({ to }) }
  );
  await ensureOk(res);

  return (await res.json()) as Renamed;
}

/** One page of a catalogue, and how many names it is a page of. */
export interface NamePage {
  names: string[];
  total: number;
}

/** Which slice of a catalogue to ask for. The service caps limit at 200. */
export interface PageRequest {
  search?: string;
  limit?: number;
  offset?: number;
}

function pageQuery({ search = "", limit, offset }: PageRequest): string {
  const query = new URLSearchParams();
  if (search) query.set("q", search);
  if (limit !== undefined) query.set("limit", String(limit));
  if (offset) query.set("offset", String(offset));
  const asked = query.toString();

  return asked ? `?${asked}` : "";
}

export async function fetchDashboardNames(
  page: PageRequest = {}
): Promise<NamePage> {
  const res = await fetchWithAuth(`${BASE}/dashboards${pageQuery(page)}`);
  return readJson<NamePage>(res);
}

export async function fetchDashboard(name: string): Promise<Dashboard> {
  const res = await fetchWithAuth(
    `${BASE}/dashboards/${encodeURIComponent(name)}`
  );
  const read = await readJson<DefinitionResponse<Dashboard>>(res);

  return read.body;
}

export async function fetchMetricNames(
  page: PageRequest = {}
): Promise<NamePage> {
  const res = await fetchWithAuth(`${BASE}/metrics${pageQuery(page)}`);
  return readJson<NamePage>(res);
}

export async function fetchMetric(name: string): Promise<StoredMetric> {
  const res = await fetchWithAuth(
    `${BASE}/metrics/${encodeURIComponent(name)}`
  );
  const read = await readJson<DefinitionResponse<MetricDefinition>>(res);

  return { definition: read.body, clock: read.clock };
}

export async function fetchWidgetNames(
  page: PageRequest = {}
): Promise<NamePage> {
  const res = await fetchWithAuth(`${BASE}/widgets${pageQuery(page)}`);
  return readJson<NamePage>(res);
}

export async function fetchWidget(name: string): Promise<Widget> {
  const res = await fetchWithAuth(
    `${BASE}/widgets/${encodeURIComponent(name)}`
  );
  const read = await readJson<DefinitionResponse<Widget>>(res);

  return read.body;
}

export async function fetchDatasetNames(
  page: PageRequest = {}
): Promise<NamePage> {
  const res = await fetchWithAuth(`${BASE}/datasets${pageQuery(page)}`);
  return readJson<NamePage>(res);
}

export async function fetchDataset(name: string): Promise<Dataset> {
  const res = await fetchWithAuth(
    `${BASE}/datasets/${encodeURIComponent(name)}`
  );
  return readJson<Dataset>(res);
}

/** The latest records the dataset holds, newest first, capped by the service. */
export async function fetchDatasetRecords(
  name: string,
  page: RecordPage
): Promise<DatasetRecords> {
  const query = new URLSearchParams({ limit: String(page.limit) });
  if (page.offset) query.set("offset", String(page.offset));
  if (page.orderBy) {
    query.set("order_by", page.orderBy);
    query.set("direction", page.descending ? "desc" : "asc");
  }
  const res = await fetchWithAuth(
    `${BASE}/datasets/${encodeURIComponent(name)}/records?${query}`
  );

  return readJson<DatasetRecords>(res);
}

/** Every definition that names this one: what a removal would break. */
export async function fetchDependents(
  kind: DefinitionKind,
  name: string
): Promise<Holder[]> {
  const res = await fetchWithAuth(
    `${BASE}/${kind}/${encodeURIComponent(name)}/dependents`
  );
  const read = await readJson<{ holders: Holder[] }>(res);

  return read.holders;
}

/**
 * Every metric that reads this dataset.
 *
 * An exact lookup over what each body names, so a metric merely mentioning
 * the name in a label is not one of them.
 */
export async function fetchDatasetDependents(name: string): Promise<string[]> {
  const res = await fetchWithAuth(
    `${BASE}/datasets/${encodeURIComponent(name)}/dependents`
  );
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
  const res = await fetchWithAuth(
    `${BASE}/datasets/${encodeURIComponent(name)}`,
    { method: "PUT", headers: JSON_HEADERS, body: JSON.stringify(declaration) }
  );
  return readJson<Dataset>(res);
}

/** Stores a definition of any other kind, as the document says it. */
export async function putDefinition(
  kind: DefinitionKind,
  name: string,
  body: unknown
): Promise<void> {
  const res = await fetchWithAuth(
    `${BASE}/${kind}/${encodeURIComponent(name)}`,
    { method: "PUT", headers: JSON_HEADERS, body: JSON.stringify(body) }
  );
  await ensureOk(res);
}

/**
 * Takes a dataset away, with the records it holds.
 *
 * Refused while a metric reads it, and the reply names every one.
 */
export async function deleteDataset(name: string): Promise<void> {
  const res = await fetchWithAuth(
    `${BASE}/datasets/${encodeURIComponent(name)}`,
    { method: "DELETE" }
  );
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
  name: string,
  options?: RunOptions
): Promise<MetricResult> {
  const res = await fetchWithAuth(
    `${BASE}/metrics/${encodeURIComponent(name)}/run`,
    {
      method: "POST",
      ...(options
        ? { headers: JSON_HEADERS, body: JSON.stringify(options) }
        : {}),
    }
  );
  return readJson<MetricResult>(res);
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

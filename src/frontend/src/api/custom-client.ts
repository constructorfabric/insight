import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_API_BASE_V3 as string | undefined) ?? "/api/v3/v1";

/** What every widget carries, whatever it draws. */
interface WidgetBase {
  metric: string;
  /** The heading a reader sees. Without it the card shows the identifier. */
  title?: string;
  /**
   * A metric for the drilldown to run instead of this widget's own.
   *
   * A chart draws an aggregate; the rows a reader wants underneath are the
   * facts that went into it — the commits, not the counts. Only the author of
   * the widget knows which query that is, so they name it.
   */
  detail?: string;
}

export interface TableWidget extends WidgetBase {
  type: "table";
  columns: string[];
}

/** A line, a bar and an area all read one column against another. */
export interface SeriesWidget extends WidgetBase {
  type: "line" | "bar" | "area";
  x: string;
  y: string;
}

export interface StatWidget extends WidgetBase {
  type: "stat";
  value: string;
  label?: string;
}

export interface PieWidget extends WidgetBase {
  type: "pie";
  label: string;
  value: string;
}

export type Widget = TableWidget | SeriesWidget | StatWidget | PieWidget;

/**
 * A metric's stored query, as the service interprets it.
 *
 * A field reads either a key inside an ingested payload (`json`) or a real
 * column of a table on the stand (`column`) — never both, and a lone `count`
 * needs neither.
 */
export interface MetricDefinition {
  database?: string;
  table: string;
  /**
   * The timestamp a reader may window and bucket by. A metric without one
   * answers every row, whatever the board's picker is set to.
   */
  time?: { json?: string; column?: string; type?: string };
  /** The widest window this metric will answer, as an ISO duration. */
  max_range?: string;
  fields: {
    json?: string;
    column?: string;
    type: string;
    agg?: string;
    as_name: string;
    person?: "email" | "id";
  }[];
  group_by?: string[];
  filters?: {
    json?: string;
    column?: string;
    type: string;
    op: string;
    value: unknown;
  }[];
  order_by?: { field: string; direction?: "asc" | "desc" };
  limit?: number;
}

export interface MetricResult {
  columns: string[];
  rows: unknown[][];
  /** The columns whose numbers are percentages, named by the metric. */
  percents?: string[];
}

/**
 * One thing a dashboard draws: a stored widget, a section title over the
 * widgets that follow, or a line of prose between them.
 */
export type DashboardItem =
  | { widget: string }
  | { heading: string }
  | { text: string };

export interface Dashboard {
  title: string;
  /** What the dashboard draws, top to bottom. */
  items?: DashboardItem[];
  /** The older shorthand: a list of nothing but widgets. */
  widgets?: string[];
  /** The windows this board offers, as server tokens. None means no picker. */
  time_ranges?: string[];
  default_range?: string;
}


export interface ChatCreated {
  metric?: string;
  widgets: string[];
  dashboard?: string;
}

/** One turn already in the thread, sent back so the model can read it. */
export interface ChatTurn {
  role: "user" | "assistant";
  content: string;
}

export interface ChatReply {
  reply: string;
  result?: MetricResult;
  created?: ChatCreated;
  /** Names that already existed and now hold something else. */
  updated?: ChatCreated;
}

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

async function readJson<T>(res: Response): Promise<T> {
  if (!res.ok) {
    throw new CustomApiError(res.status, await res.json().catch(() => null));
  }
  return (await res.json()) as T;
}

/** What a definition is, in the API's path segments. */
export type DefinitionKind = "metrics" | "widgets" | "dashboards";

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
  const res = await fetchWithAuth(`${BASE}/${kind}/${encodeURIComponent(name)}`, {
    method: "DELETE",
  });
  if (!res.ok) {
    throw new CustomApiError(res.status, await res.json().catch(() => null));
  }
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
  if (!res.ok) {
    throw new CustomApiError(res.status, await res.json().catch(() => null));
  }

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
  return readJson<Dashboard>(res);
}

export async function fetchMetricNames(
  page: PageRequest = {}
): Promise<NamePage> {
  const res = await fetchWithAuth(`${BASE}/metrics${pageQuery(page)}`);
  return readJson<NamePage>(res);
}

export async function fetchMetric(name: string): Promise<MetricDefinition> {
  const res = await fetchWithAuth(
    `${BASE}/metrics/${encodeURIComponent(name)}`
  );
  return readJson<MetricDefinition>(res);
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
  return readJson<Widget>(res);
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

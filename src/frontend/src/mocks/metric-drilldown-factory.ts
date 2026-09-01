import type {
  MetricDrilldownRequest,
  MetricDrilldownResponse,
  MetricEvidenceColumn,
  MetricEvidenceRow,
  MetricEvidenceSort,
} from "@/api/metric-drilldown-client";

import { metaFor } from "./metric-results-factory";
import { PEOPLE } from "./registry";

/**
 * `/v1/metric-drilldown`, answered the way the service answers it.
 *
 * The point of this mock is the ORDER and the NARROWING: both moved to the
 * server, so a client-side reimplementation here would hide exactly the
 * behaviour it exists to show. Everything below mirrors
 * `domain/metric_drilldown` — the emptiness flag ahead of the sort key, the
 * digits-sort-as-digits rule, the tiebreaker travelling with the direction,
 * `person` resolved to a name and not sortable, and a cursor that restarts
 * rather than resuming into a selection it was not issued for.
 */

const PAGE_ROWS_PER_PERSON = 15;

function hash(input: string): number {
  let h = 2166136261;
  for (let index = 0; index < input.length; index += 1) {
    h ^= input.charCodeAt(index);
    h = Math.imul(h, 16777619);
  }
  return Math.abs(h);
}

const REPOSITORIES = [
  "example/platform",
  "example/web",
  "example/ingestion",
  "example/docs",
];

const BRANCHES = ["main", "release/1.4", "develop"];

const SUBJECTS = [
  "Add the parser",
  "Fix logging on retry",
  "Cache the resolved roster",
  "Drop the unused column",
  "Rename the evidence relation",
  "Handle an empty period",
  "", // a record whose subject never arrived — the empty-cell case
  "Widen the cursor to the sort key",
];

interface DetailColumn {
  key: string;
  label: string;
  type: MetricEvidenceColumn["type"];
}

const COMMIT_COLUMNS: DetailColumn[] = [
  { key: "ref", label: "Ref", type: "string" },
  { key: "title", label: "Title", type: "string" },
  { key: "repository", label: "Repository", type: "string" },
  { key: "author", label: "Author", type: "string" },
  { key: "lines_added", label: "Lines added", type: "number" },
  { key: "lines_removed", label: "Lines removed", type: "number" },
];

const REQUEST_COLUMNS: DetailColumn[] = [
  { key: "ref", label: "Ref", type: "string" },
  { key: "title", label: "Title", type: "string" },
  { key: "repository", label: "Repository", type: "string" },
  { key: "author", label: "Author", type: "string" },
  { key: "destination_branch", label: "Destination branch", type: "string" },
];

const TASK_COLUMNS: DetailColumn[] = [
  { key: "ref", label: "Ref", type: "string" },
  { key: "title", label: "Title", type: "string" },
  { key: "project", label: "Project", type: "string" },
  { key: "type", label: "Type", type: "string" },
];

/** What a measure declares it carries; nothing means the date and a number. */
function detailColumns(metricKey: string): DetailColumn[] {
  if (metricKey.startsWith("git.pr")) return REQUEST_COLUMNS;
  if (metricKey.startsWith("git.")) return COMMIT_COLUMNS;
  if (metricKey.startsWith("tasks.")) return TASK_COLUMNS;
  return [];
}

function isRatio(metricKey: string): boolean {
  return metaFor(metricKey).computation === "ratio";
}

function dayIn(period: { from: string; to: string }, offset: number): string {
  const [year, month, day] = period.from.split("-").map(Number);
  const start = Date.UTC(year ?? 2026, (month ?? 1) - 1, day ?? 1);
  const [toYear, toMonth, toDay] = period.to.split("-").map(Number);
  const end = Date.UTC(toYear ?? 2026, (toMonth ?? 1) - 1, toDay ?? 1);
  const span = Math.max(1, Math.round((end - start) / 86_400_000) + 1);
  return new Date(start + (offset % span) * 86_400_000)
    .toISOString()
    .slice(0, 10);
}

const NAME_BY_PERSON = new Map(
  PEOPLE.map((person) => [person.person_id, person.name]),
);

/** Every person the selection reads, in the order the request bound them. */
function rosterOf(request: MetricDrilldownRequest): string[] {
  if (request.entity.type === "persons") return request.entity.ids;
  if (request.entity.type === "person") return [request.entity.id];
  return ["tenant"];
}

/**
 * The `person` column, and only where the service serves one: a roster, and
 * never a ratio, whose rows are two aggregates over a day rather than records
 * anyone owns.
 */
function servesPerson(request: MetricDrilldownRequest): boolean {
  return request.entity.type === "persons" && !isRatio(request.metric_key);
}

export function drilldownColumns(
  request: MetricDrilldownRequest,
): MetricEvidenceColumn[] {
  const columns: MetricEvidenceColumn[] = [];
  if (servesPerson(request)) {
    // Resolved to a name on the server, so the query holds an id it cannot
    // order by. Shown, not sortable — the same reason the service gives.
    columns.push({
      key: "person",
      label: "Who",
      type: "string",
      sortable: false,
    });
  }
  if (isRatio(request.metric_key)) {
    return [
      ...columns,
      { key: "date", label: "Date", type: "date", sortable: true },
      { key: "numerator", label: "Numerator", type: "number", sortable: true },
      {
        key: "denominator",
        label: "Denominator",
        type: "number",
        sortable: true,
      },
    ];
  }

  const details = detailColumns(request.metric_key);
  columns.push(
    ...details.map((column) => ({ ...column, sortable: true })),
    { key: "date", label: "Date", type: "date" as const, sortable: true },
  );
  if (details.length === 0) {
    columns.push({
      key: "value",
      label: "Value",
      type: "number",
      sortable: true,
    });
  }
  return columns;
}

/** Every record behind the selection, before any order or narrowing. */
function evidenceRows(request: MetricDrilldownRequest): MetricEvidenceRow[] {
  const roster = rosterOf(request);
  const ratio = isRatio(request.metric_key);
  const details = detailColumns(request.metric_key);
  const withPerson = servesPerson(request);

  return roster.flatMap((personId, personIndex) =>
    Array.from({ length: PAGE_ROWS_PER_PERSON }, (_, index) => {
      const seed = hash(`${request.metric_key}|${personId}|${index}`);
      const date = dayIn(request.period, seed % 90);
      const values: Record<string, unknown> = {};
      if (withPerson) {
        values.person = NAME_BY_PERSON.get(personId) ?? null;
      }
      if (ratio) {
        const denominator = (seed % 7) + 1;
        return {
          values: {
            ...values,
            date,
            numerator: seed % 40,
            denominator,
          },
        };
      }

      // A spread of magnitudes on purpose: ascending must read 41, 300, 2958
      // and not 2958, 300, 41 — lexicographic order is only visibly wrong
      // where the digit counts differ.
      const ref = String((seed % 30_000) + 1);
      const repository = REPOSITORIES[seed % REPOSITORIES.length]!;
      for (const column of details) {
        switch (column.key) {
          case "ref":
            values.ref = ref;
            break;
          case "title":
            values.title = SUBJECTS[seed % SUBJECTS.length] ?? "";
            break;
          case "repository":
            values.repository = repository;
            break;
          case "author":
            values.author = NAME_BY_PERSON.get(personId) ?? personId;
            break;
          case "project":
            values.project = repository.split("/")[1] ?? repository;
            break;
          case "type":
            values.type = seed % 3 === 0 ? "Bug" : "Task";
            break;
          case "destination_branch":
            values.destination_branch = BRANCHES[seed % BRANCHES.length]!;
            break;
          case "lines_added":
            // Some records carry no line counts, so the empties-last rule has
            // something to put last.
            values.lines_added = seed % 4 === 0 ? null : seed % 320;
            break;
          case "lines_removed":
            values.lines_removed = seed % 4 === 0 ? null : seed % 90;
            break;
          default:
            break;
        }
      }
      values.date = date;
      if (details.length === 0) values.value = (seed % 12) + 1;

      return {
        values,
        links: values.ref
          ? {
              ref: `https://example.com/${repository}/pull/${ref}`,
              title: `https://example.com/${repository}/pull/${ref}`,
              repository: `https://example.com/${repository}`,
            }
          : {},
        // Not on the wire — the tiebreaker the service closes its ordering key
        // with, kept here so a page boundary lands in one place.
        __tiebreak: `${personIndex}:${index}`,
      } as MetricEvidenceRow & { __tiebreak: string };
    }),
  );
}

function cellText(value: unknown): string {
  if (value == null) return "";
  return String(value);
}

/**
 * INVARIANT: mirrors `sort.rs` — digits sort as digits so `300` precedes
 * `2958`, and anything else keeps the lexicographic order it already had.
 */
function naturalKey(text: string): string {
  const trimmed = text.trim();
  return /^\d+$/.test(trimmed) && trimmed.length < 20
    ? trimmed.padStart(20, "0")
    : trimmed;
}

function orderKey(
  row: MetricEvidenceRow,
  column: MetricEvidenceColumn,
): number | string {
  const text = cellText(row.values[column.key]);
  if (column.type === "number") return Number.parseFloat(text) || 0;
  if (column.type === "date") return text.trim();
  return naturalKey(text);
}

function compare(left: number | string, right: number | string): number {
  if (typeof left === "number" && typeof right === "number") {
    return left - right;
  }
  return String(left) < String(right) ? -1 : String(left) > String(right) ? 1 : 0;
}

/**
 * INVARIANT: mirrors `compiler.rs` — an emptiness flag ahead of the sort key
 * so blank cells land last whichever way the column runs, then the whole key
 * travelling in one direction, tiebreaker included.
 */
function ordered(
  rows: MetricEvidenceRow[],
  columns: MetricEvidenceColumn[],
  sort: MetricEvidenceSort,
): MetricEvidenceRow[] {
  const column = columns.find((candidate) => candidate.key === sort.key);
  if (!column) return rows;
  const factor = sort.direction === "asc" ? 1 : -1;

  return [...rows].sort((left, right) => {
    const leftEmpty = cellText(left.values[column.key]).trim() === "";
    const rightEmpty = cellText(right.values[column.key]).trim() === "";
    if (leftEmpty !== rightEmpty) return leftEmpty ? 1 : -1;

    const byKey = compare(orderKey(left, column), orderKey(right, column));
    if (byKey !== 0) return byKey * factor;

    const tiebreak = compare(
      (left as { __tiebreak?: string }).__tiebreak ?? "",
      (right as { __tiebreak?: string }).__tiebreak ?? "",
    );
    return tiebreak * factor;
  });
}

/**
 * INVARIANT: mirrors the service's search — every column the reader can see
 * EXCEPT `person`, which the query holds as an id and cannot match a name
 * against. Narrowing it here would hide that.
 */
function narrowed(
  rows: MetricEvidenceRow[],
  columns: MetricEvidenceColumn[],
  search: string | undefined,
): MetricEvidenceRow[] {
  const needle = search?.trim().toLowerCase();
  if (!needle) return rows;
  const searchable = columns.filter((column) => column.key !== "person");
  return rows.filter((row) =>
    searchable.some((column) =>
      cellText(row.values[column.key]).toLowerCase().includes(needle),
    ),
  );
}

/** The order a request that names none is served in. */
const NEWEST_FIRST: MetricEvidenceSort = { key: "date", direction: "desc" };

/**
 * A cursor is bound to the selection that issued it. This one carries a
 * fingerprint of the whole read, and a mismatch restarts at the first page —
 * where the service refuses outright, which a mock cannot usefully imitate
 * without turning every dev reload into an error dialog.
 */
function fingerprint(request: MetricDrilldownRequest): string {
  return String(
    hash(
      JSON.stringify([
        request.metric_key,
        request.entity,
        request.period,
        request.filters,
        request.display_dimensions,
        request.sort ?? NEWEST_FIRST,
        request.search ?? null,
      ]),
    ),
  );
}

function offsetOf(request: MetricDrilldownRequest): number {
  if (!request.cursor) return 0;
  try {
    const decoded = JSON.parse(atob(request.cursor)) as {
      fp?: string;
      at?: number;
    };
    return decoded.fp === fingerprint(request) ? Number(decoded.at ?? 0) : 0;
  } catch {
    return 0;
  }
}

export function buildMetricDrilldownResponse(
  request: MetricDrilldownRequest,
): MetricDrilldownResponse {
  const columns = drilldownColumns(request);
  const sort =
    request.sort && columns.some((c) => c.key === request.sort?.key && c.sortable)
      ? request.sort
      : NEWEST_FIRST;

  const matching = ordered(
    narrowed(evidenceRows(request), columns, request.search),
    columns,
    sort,
  );

  const limit = Math.max(1, Math.min(request.limit ?? 100, 250));
  const offset = offsetOf(request);
  const page = matching.slice(offset, offset + limit);
  const next = offset + page.length;

  return {
    selection: {
      metric_key: request.metric_key,
      entity: request.entity,
      period: request.period,
      filters: request.filters,
      display_dimensions: request.display_dimensions,
      // Always the effective order — its absence is what tells a client it is
      // talking to a server that cannot sort.
      sort,
      search: request.search?.trim() || null,
    },
    columns,
    rows: page.map(({ values, links }) => ({ values, links })),
    next_cursor:
      next < matching.length
        ? btoa(JSON.stringify({ fp: fingerprint(request), at: next }))
        : null,
  };
}

/** The same rows the page serves, as the CSV the export endpoint returns. */
export function buildMetricDrilldownCsv(
  request: MetricDrilldownRequest,
): string {
  const response = buildMetricDrilldownResponse({ ...request, limit: 250 });
  const cell = (value: unknown) => `"${cellText(value).replaceAll('"', '""')}"`;
  return [
    response.columns.map((column) => cell(column.label)).join(","),
    ...response.rows.map((row) =>
      response.columns.map((column) => cell(row.values[column.key])).join(","),
    ),
  ].join("\r\n");
}

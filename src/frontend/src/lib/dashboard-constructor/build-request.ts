/**
 * The constructor's form state and its projection onto the query contract.
 *
 * INVARIANT: this module knows nothing about React or the network. Every
 * value a user typed stays a string here; typing (number for a measurable,
 * text for a dimension) happens once, when the request is built.
 */

import type {
  Query,
  QueryAggregate,
  QueryDataset,
  QueryDirection,
  QueryFilter,
  QueryFilterOp,
  QueryFoldFn,
  QueryGrain,
  QueryScalar,
} from "@/api/query-client";

export type AggregateFn = "count" | QueryFoldFn;

export const FILTER_OPS: readonly QueryFilterOp[] = [
  "eq",
  "in",
  "gt",
  "gte",
  "lt",
  "lte",
  "between",
  "not_null",
  "like",
  "match",
];

export const AGGREGATE_FNS: readonly AggregateFn[] = ["count", "sum", "avg", "min", "max"];

export const GRAINS: readonly QueryGrain[] = ["day", "week", "month"];

export interface FilterDraft {
  id: string;
  field: string;
  op: QueryFilterOp;
  /** `eq`, the ordered tests and the patterns; comma-separated for `in`. */
  value: string;
  low: string;
  high: string;
}

export interface AggregateDraft {
  id: string;
  name: string;
  fn: AggregateFn;
  field: string;
  filter: FilterDraft | null;
}

export interface QueryDraft {
  dataset: string;
  from: string;
  to: string;
  timeAxis: boolean;
  grain: QueryGrain;
  dimensions: string[];
  aggregates: AggregateDraft[];
  filters: FilterDraft[];
  orderBy: string;
  orderDir: QueryDirection;
  limit: string;
}

let nextId = 0;
export function draftId(): string {
  nextId += 1;
  return `d${nextId}`;
}

export function emptyFilter(field: string): FilterDraft {
  return { id: draftId(), field, op: "eq", value: "", low: "", high: "" };
}

export function emptyAggregate(): AggregateDraft {
  return { id: draftId(), name: "rows", fn: "count", field: "", filter: null };
}

export function initialDraft(dataset: QueryDataset, today: Date): QueryDraft {
  const to = isoDay(today);
  const from = isoDay(new Date(today.getTime() - 89 * 24 * 60 * 60 * 1000));
  return {
    dataset: dataset.key,
    from,
    to,
    timeAxis: false,
    grain: "week",
    dimensions: [],
    aggregates: [emptyAggregate()],
    filters: [],
    orderBy: "",
    orderDir: "desc",
    limit: String(dataset.limits.default_limit),
  };
}

export function isoDay(date: Date): string {
  return date.toISOString().slice(0, 10);
}

/** Aggregate names plus grouped columns: what an order term may name. */
export function orderableColumns(draft: QueryDraft): string[] {
  const columns = [...draft.dimensions];
  if (draft.timeAxis) columns.push("time");
  for (const aggregate of draft.aggregates) {
    if (aggregate.name.trim() !== "") columns.push(aggregate.name.trim());
  }
  return columns;
}

/** Whether a filter operator takes each operand box. */
export function operandsOf(op: QueryFilterOp): { value: boolean; range: boolean } {
  if (op === "not_null") return { value: false, range: false };
  if (op === "between") return { value: false, range: true };
  return { value: true, range: false };
}

export function buildQuery(draft: QueryDraft, dataset: QueryDataset): Query {
  const measurables = new Set(dataset.measurables.map((m) => m.field));
  const scalar = (field: string, raw: string): QueryScalar =>
    measurables.has(field) ? numberOrText(raw) : raw;

  const filters = draft.filters.map((filter) => buildFilter(filter, scalar));
  const aggregates = draft.aggregates.map((aggregate) => buildAggregate(aggregate, scalar));

  const group_by: Query["group_by"] = draft.dimensions.map((field) => ({
    axis: "dimension" as const,
    field,
  }));
  if (draft.timeAxis) group_by.push({ axis: "time" });

  const query: Query = {
    dataset: draft.dataset,
    aggregates,
    time: {
      from: draft.from,
      to: draft.to,
      ...(draft.timeAxis ? { grain: draft.grain } : {}),
    },
  };
  if (filters.length > 0) query.filters = filters;
  if (group_by.length > 0) query.group_by = group_by;
  if (draft.orderBy !== "") query.order = [{ by: draft.orderBy, dir: draft.orderDir }];

  const limit = Number.parseInt(draft.limit, 10);
  if (Number.isFinite(limit)) query.limit = limit;

  return query;
}

function buildFilter(
  filter: FilterDraft,
  scalar: (field: string, raw: string) => QueryScalar,
): QueryFilter {
  const { field, op } = filter;
  switch (op) {
    case "not_null":
      return { op, field };
    case "like":
    case "match":
      return { op, field, value: filter.value };
    case "between":
      return {
        op,
        field,
        low: scalar(field, filter.low.trim()),
        high: scalar(field, filter.high.trim()),
      };
    case "in":
      return {
        op,
        field,
        values: filter.value
          .split(",")
          .map((part) => part.trim())
          .filter((part) => part !== "")
          .map((part) => scalar(field, part)),
      };
    case "eq":
    case "gt":
    case "gte":
    case "lt":
    case "lte":
      return { op, field, value: scalar(field, filter.value.trim()) };
  }
}

function buildAggregate(
  aggregate: AggregateDraft,
  scalar: (field: string, raw: string) => QueryScalar,
): QueryAggregate {
  const name = aggregate.name.trim();
  const filter = aggregate.filter ? buildFilter(aggregate.filter, scalar) : undefined;
  if (aggregate.fn === "count") {
    return filter ? { fn: "count", name, filter } : { fn: "count", name };
  }
  const fold = { fn: aggregate.fn, name, field: aggregate.field };
  return filter ? { ...fold, filter } : fold;
}

/** A measurable compares against numbers; anything that is not one is sent as typed so the server names the field. */
function numberOrText(raw: string): QueryScalar {
  if (raw === "") return raw;
  const parsed = Number(raw);
  return Number.isFinite(parsed) ? parsed : raw;
}

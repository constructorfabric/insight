/** What the Custom zone's service hands back and takes, as shapes. */

import type {
  MetricEvidenceColumn,
  MetricEvidenceRow,
  MetricEvidenceSort,
} from "@/api/metric-drilldown-client";

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
 * Where one field or one condition of a metric reads its value from.
 *
 * Over a dataset it names a declared field, and where the value sits in a
 * record is the declaration's to say. Over a warehouse table it names a
 * column, or a JSON key path inside one; a key with no column is read inside
 * the row's `raw_data`.
 */
export interface MetricRead {
  field?: string;
  column?: string;
  json?: string;
}

/**
 * A metric's stored query, as the service interprets it.
 *
 * It reads one dataset or one warehouse table: `dataset` names the first,
 * `table` (as `database.table`, or beside a `database` of its own) the second.
 */
export interface MetricDefinition {
  dataset?: string;
  table?: string;
  database?: string;
  /**
   * The date a reader may window and bucket by. Over a dataset, left out, the
   * dataset's own main date is used - which only the declaration knows, so
   * the service reports the one in force rather than the body implying it.
   */
  time?: MetricRead;
  /** The widest window this metric will answer, as an ISO duration. */
  max_range?: string;
  fields: (MetricRead & {
    type: string;
    agg?: string;
    as_name: string;
  })[];
  group_by?: string[];
  filters?: (MetricRead & {
    type: string;
    op: string;
    value: unknown;
  })[];
  order_by?: { field: string; direction?: "asc" | "desc" };
  limit?: number;
}

/**
 * The date a window over a metric selects by, and who decided it.
 *
 * A metric over a dataset may name no date and still be windowed, because the
 * dataset marks one, so its stored body cannot say whether a card drawn from
 * it follows the board's window. This can.
 */
export interface EffectiveClock {
  field: string;
  from: "metric" | "dataset";
}

/** A stored definition as the service hands it back. */
export interface DefinitionResponse<T> {
  body: T;
  /** Only a metric has one, and only when a window has a date to select by. */
  clock?: EffectiveClock;
}

/** A metric as it is stored, with the clock a window over it would use. */
export interface StoredMetric {
  definition: MetricDefinition;
  clock?: EffectiveClock;
}

/** What a value is read as, which decides what a metric may do with it. */
export type FieldType = "string" | "int" | "float" | "bool" | "datetime";

/** What a field is for, as a reader is told it. Advisory only. */
export type FieldRole = "dimension" | "measurable" | "time";

/** One field of a dataset, as declared. */
export interface DeclaredField {
  name: string;
  /** Where the value sits in a record, as dot-separated segments. */
  path: string;
  type: FieldType;
  role?: FieldRole;
  description?: string;
  /** What a reader is shown where the value is empty. Presentation only. */
  absent_value?: string;
  person?: "email" | "id";
  /** Whether a window with no date of its own selects by this one. */
  default_clock?: boolean;
}

/** What a dataset says about the records it holds. */
export interface DatasetDeclaration {
  title: string;
  description?: string;
  fields: DeclaredField[];
  /** The fields that make two records the same record. */
  row_identity?: string[];
}

/** Which part of the warehouse a table belongs to, read off its database. */
export type TableLayer = "bronze" | "silver" | "gold" | "identity" | "other";

/** One warehouse table as the catalogue lists it: enough to name it. */
export interface WarehouseTable {
  database: string;
  table: string;
  layer: TableLayer;
}

export interface TableList {
  tables: WarehouseTable[];
  total: number;
}

export interface TableColumn {
  name: string;
  /** As the warehouse spells it, `Nullable(Int64)` and the like. */
  type: string;
}

/** One warehouse table with what a metric over it needs. */
export interface TableSchema extends WarehouseTable {
  /** A replacing engine is read through FINAL without the metric saying so. */
  engine: string;
  columns: TableColumn[];
}

/** A dataset as the catalogue and its page read it. */
export interface Dataset {
  name: string;
  declaration: DatasetDeclaration;
}

/** One record a dataset holds, as it arrived. */
export interface DatasetRecord {
  id: string;
  received_at: string;
  raw_data: unknown;
}

/** One definition that names another: what a removal would break. */
export interface Holder {
  /** The kind it is, as the API path spells it. */
  kind: "metrics" | "widgets" | "dashboards";
  name: string;
}

/** What one page of a dataset's records is asked for. */
export interface RecordPage {
  /** Absent asks for the page size the installation allows. */
  limit?: number;
  offset?: number;
  /** A declared field, or `received_at`. Absent means arrival order. */
  orderBy?: string;
  descending?: boolean;
}

/** A look at a dataset: one page of its records, and how many it holds in all. */
export interface DatasetRecords {
  records: DatasetRecord[];
  /** Every record that arrived, a re-sent one counted again. */
  total: number;
  /**
   * The page size the service applied. Its cap is an installation's setting,
   * so a reader stepping by offset reads this rather than assuming the limit
   * it asked for was the one used.
   */
  limit: number;
}

export interface MetricResult {
  columns: string[];
  rows: unknown[][];
  /** The columns whose numbers are percentages, named by the metric. */
  percents?: string[];
}

export type DrilldownSort = MetricEvidenceSort;

/**
 * One ordered page of a metric's rows, asked for by the reader.
 *
 * The window is the card's own; the order, the page and the cursor are the
 * reader's. A cursor is what the previous page handed back, and nothing else.
 */
export interface DrilldownRequest {
  range?: string;
  bucket?: boolean;
  sort?: DrilldownSort;
  limit?: number;
  cursor?: string;
}

/**
 * One column of a page, as the analytics table draws one, plus what the
 * metric says its numbers are.
 */
export interface DrilldownColumn extends MetricEvidenceColumn {
  sortable: boolean;
  percent: boolean;
}

export type DrilldownRow = Pick<MetricEvidenceRow, "values">;

export interface DrilldownPage {
  /** The read as the service performed it. `sort` is always the effective order. */
  selection: {
    metric: string;
    range?: string;
    bucket?: boolean;
    sort: DrilldownSort;
  };
  columns: DrilldownColumn[];
  rows: DrilldownRow[];
  next_cursor: string | null;
}

/**
 * One thing a dashboard draws: a stored widget, a section title over the
 * widgets that follow, or a line of prose between them.
 */
export type DashboardItem =
  { widget: string } | { heading: string } | { text: string };

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

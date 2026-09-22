/** What the Custom zone's service hands back and takes, as shapes. */

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
 * Every field names a field the dataset declares. Where a value sits in a
 * record is the declaration's to say, so a metric never says it.
 */
export interface MetricDefinition {
  /** The dataset this metric reads. Every metric reads one. */
  dataset: string;
  /**
   * The declared field a reader may window and bucket by. Left out, the
   * dataset's own main date is used - which only the declaration knows, so
   * the service reports the one in force rather than the body implying it.
   */
  time?: { field?: string };
  /** The widest window this metric will answer, as an ISO duration. */
  max_range?: string;
  fields: {
    field?: string;
    type: string;
    agg?: string;
    as_name: string;
  }[];
  group_by?: string[];
  filters?: {
    field?: string;
    type: string;
    op: string;
    value: unknown;
  }[];
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

/**
 * Where a field's value sits, which depends on what the dataset is over.
 *
 * INVARIANT: one or the other, never both and never neither. A field of a
 * dataset records are sent into reads a key path out of the payload; one of a
 * dataset over a relation reads a column.
 */
export type FieldAt = { path: string } | { column: string };

/** One field of a dataset, as declared. */
export type DeclaredField = FieldAt & {
  name: string;
  type: FieldType;
  role?: FieldRole;
  description?: string;
  /** What a reader is shown where the value is empty. Presentation only. */
  absent_value?: string;
  person?: "email" | "id";
  /** Whether a window with no date of its own selects by this one. */
  default_clock?: boolean;
};

/**
 * What a dataset is over, and with it who owns its rows.
 *
 * INVARIANT: written, never inferred from what else the declaration carries.
 * Too much follows from it — who provisions the relation, whether records may
 * be sent, how every field is read.
 */
export type DatasetSource =
  | { kind: "stream" }
  | { kind: "relation"; database: string; table: string };

/** What a dataset says about the records it holds. */
export interface DatasetDeclaration {
  title: string;
  description?: string;
  source: DatasetSource;
  fields: DeclaredField[];
  /** The fields that make two records the same record. */
  row_identity?: string[];
}

/** A dataset as the catalogue and its page read it. */
export interface Dataset {
  name: string;
  declaration: DatasetDeclaration;
}

/**
 * One row of a dataset as a reader is shown it.
 *
 * A record sent into a dataset carries its own identity and the instant it
 * arrived. A row of a relation the warehouse builds carries neither: it was
 * not sent, and nothing stamped it.
 */
export interface DatasetRecord {
  id?: string;
  received_at?: string;
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
  /**
   * A declared field, or `received_at` for a dataset records are sent into.
   * Absent means arrival order, or — over a relation, which has none — the
   * dataset's main date.
   */
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

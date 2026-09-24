import type { Description, Field, Shape } from "./describe";

// INVARIANT: these mirror the shapes the service refuses against. Where they
// drift the service is right, and its refusal lands on the field rather than
// being hidden here.

const FIELD_TYPES = ["string", "int", "float", "bool", "datetime"] as const;
const FIELD_ROLES = ["dimension", "measurable", "time"] as const;
const PERSON_HANDLES = ["email", "id"] as const;
const AGGREGATES = ["count", "sum", "avg", "min", "max"] as const;
const OPERATORS = ["eq", "ne", "gt", "gte", "lt", "lte"] as const;
const DIRECTIONS = ["asc", "desc"] as const;

const DECLARED_FIELD: Shape = {
  of: "record",
  fields: [
    {
      name: "name",
      label: "Name",
      shape: { of: "text" },
      required: true,
      hint: "The identifier metrics, filters, the window and the row identity refer to this field by.",
    },
    {
      name: "path",
      label: "Path",
      shape: { of: "text" },
      required: true,
      hint: "Where the value sits in a record, as dot-separated keys (who.email).",
    },
    {
      name: "type",
      label: "Type",
      shape: { of: "choice", options: FIELD_TYPES },
      required: true,
      hint: "How the value is read, which decides what a metric may do with it: sum an int or a float, window by a datetime, compare a filter as this type.",
    },
    {
      name: "role",
      label: "Role",
      shape: { of: "choice", options: FIELD_ROLES },
      hint: "What the field is for, as a reader and the assistant are told: grouped by, counted, or a date. Advisory only.",
    },
    {
      name: "description",
      label: "Description",
      shape: { of: "text" },
      hint: "One line for people and the assistant.",
    },
    {
      name: "absent_value",
      label: "Shown when empty",
      shape: { of: "text" },
      hint: "What a reader sees where the record has no value. Presentation only: a filter and the row identity read the record's own value. String fields only.",
    },
    {
      name: "person",
      label: "Holds a person by",
      shape: { of: "choice", options: PERSON_HANDLES },
      hint: "The value names a person, by email or by id, for the service to resolve. String fields only.",
    },
    {
      name: "default_clock",
      label: "Main date",
      shape: { of: "flag", alone: true },
      hint: "A window with no date of its own selects by this one. One per dataset, on a datetime field.",
    },
  ],
};

const DATASET: Description = {
  kind: "datasets",
  noun: "dataset",
  fields: [
    {
      name: "title",
      label: "Title",
      shape: { of: "text" },
      required: true,
      hint: "The heading a reader sees in the catalogue and on the dataset's page.",
    },
    {
      name: "description",
      label: "Description",
      shape: { of: "longText" },
      hint: "What the records are and where they come from, for people and the assistant.",
    },
    {
      name: "fields",
      label: "Fields",
      hint: "How a record is read: one entry per value the dataset exposes, each with the key it sits under.",
      shape: { of: "list", entry: DECLARED_FIELD, entryLabel: "field" },
      required: true,
    },
    {
      name: "row_identity",
      label: "One record per",
      shape: {
        of: "list",
        entry: { of: "pick", from: { list: "fields", property: "name" } },
        entryLabel: "field name",
      },
      hint: "Declared fields that make two records the same row. A record sent again replaces the earlier one instead of counting twice. Leave empty to keep every record on its own.",
    },
  ],
};

const DECLARED = { dataset: "dataset" } as const;
const TABLE = { table: "table", database: "database" } as const;
const OWN_COLUMN = { list: "fields", property: "as_name" } as const;

/** What a metric reads: a dataset by its declared fields, or a table by its columns. */
type Source = "dataset" | "table";

const TYPE_OPTIONS = ["string", "int", "float"] as const;

/**
 * Where one field or one condition takes its value from, by what the metric
 * reads. Over a dataset that is a declared field; over a table a column, a
 * JSON key path inside one, or a key inside the row's `raw_data`.
 */
function readsFrom(
  source: Source,
  hint: { dataset: string; table: string },
  required?: true
): readonly Field[] {
  if (source === "dataset") {
    return [
      {
        name: "field",
        label: required ? "Field" : "Reads",
        hint: hint.dataset,
        shape: { of: "pick", from: DECLARED },
        required,
      },
    ];
  }
  return [
    {
      name: "column",
      label: "Column",
      hint: required ? `${hint.table} This or a JSON key below.` : hint.table,
      shape: { of: "pick", from: TABLE },
    },
    {
      name: "json",
      label: "JSON key",
      hint: "A dot-separated key path read inside the column above, or inside the row's raw_data when no column is named. Leave empty to read the column as is.",
      shape: { of: "text" },
    },
  ];
}

function condition(source: Source): Shape {
  return {
    of: "record",
    fields: [
      ...readsFrom(
        source,
        {
          dataset: "The declared field this compares.",
          table: "The column this compares.",
        },
        true
      ),
      {
        name: "type",
        label: "Type",
        hint:
          source === "dataset"
            ? "The type the value is written as here. The service compares as the field's declared type."
            : "The type the value is written as here, and compared as.",
        shape: { of: "choice", options: TYPE_OPTIONS },
        required: true,
      },
      {
        name: "op",
        label: "Compares",
        hint: "eq and ne on any field; gt, gte, lt and lte on numbers and dates.",
        shape: { of: "choice", options: OPERATORS },
        required: true,
      },
      {
        name: "value",
        label: "Against",
        hint:
          source === "dataset"
            ? "What the field is compared with, read as the field's declared type."
            : "What the column is compared with, read as the type above.",
        shape:
          source === "dataset"
            ? {
                of: "typed",
                by: "type",
                declared: { dataset: "dataset", named: "field" },
              }
            : { of: "typed", by: "type" },
        required: true,
      },
    ],
  };
}

function produced(source: Source): Field {
  return {
    name: "fields",
    label: "Fields",
    hint: "The columns this metric produces, one per entry: a value read as is, or an aggregate over one.",
    shape: {
      of: "list",
      entryLabel: "field",
      entry: {
        of: "record",
        fields: [
          ...readsFrom(source, {
            dataset: "A declared field. Leave empty for a count of the records.",
            table: "A column of the table. Leave empty for a count of the rows.",
          }),
          {
            name: "type",
            label: "Type",
            hint: "The type of the column produced: int for a count, the value's type otherwise.",
            shape: { of: "choice", options: TYPE_OPTIONS },
            required: true,
          },
          {
            name: "agg",
            label: "Aggregate",
            hint: "Leave empty to read the value as is. A count needs no field.",
            shape: { of: "choice", options: AGGREGATES },
          },
          {
            name: "as_name",
            label: "Called",
            hint: "The column's name in the result: what a widget, a grouping and an ordering refer to.",
            shape: { of: "text" },
            required: true,
          },
          {
            name: "when",
            label: "Counted only when",
            shape: {
              of: "list",
              entry: condition(source),
              entryLabel: "condition",
            },
            hint: "Rows the aggregate takes in; the rest are left out of it alone.",
          },
          {
            name: "divide",
            label: "Divided",
            shape: {
              of: "list",
              entry: { of: "pick", from: OWN_COLUMN },
              entryLabel: "column",
            },
            hint: "Two of this metric's own columns: the numerator, then the denominator.",
          },
          {
            name: "percent",
            label: "As a percentage",
            hint: "Show the division as a percentage rather than a ratio.",
            shape: { of: "flag" },
          },
        ],
      },
    },
    required: true,
  };
}

function windowedBy(source: Source): Field {
  if (source === "dataset") {
    return {
      name: "time",
      label: "Window by",
      shape: {
        of: "record",
        fields: [
          {
            name: "field",
            label: "Declared date",
            hint: "A datetime field of the dataset.",
            shape: { of: "pick", from: DECLARED },
          },
        ],
      },
      hint: "Leave empty to use the dataset's own main date.",
    };
  }
  return {
    name: "time",
    label: "Window by",
    shape: {
      of: "record",
      fields: readsFrom(source, {
        dataset: "",
        table: "A date or datetime column: when the thing happened, never when the row was loaded.",
      }),
    },
    hint: "The timestamp a reader may window and bucket by. Without one the metric answers every row, whatever the board is set to.",
  };
}

function filtered(source: Source): Field {
  return {
    name: "filters",
    label: "Filtered",
    hint: "Rows the whole metric reads. A condition on one aggregate alone goes under that aggregate.",
    shape: { of: "list", entry: condition(source), entryLabel: "filter" },
  };
}

const METRIC: Description = {
  kind: "metrics",
  noun: "metric",
  // A metric with neither address is no variant at all and shows nothing to
  // fill in, so a new one begins over a dataset until told otherwise.
  starting: { dataset: "" },
  fields: [
    {
      name: "source",
      label: "Source",
      hint: "A dataset, by the fields it declares, or any warehouse table, by its columns.",
      required: true,
      shape: {
        of: "variants",
        // A metric over another source is another query: its fields, window
        // and filters all name the source it was written for.
        resets: true,
        variants: {
          dataset: [
            {
              name: "dataset",
              label: "Dataset",
              shape: { of: "reference", to: "datasets" },
              required: true,
              hint: "Where a value sits, its type and the main date all come from the declaration.",
            },
            produced("dataset"),
            windowedBy("dataset"),
            filtered("dataset"),
          ],
          table: [
            {
              name: "table",
              label: "Table",
              shape: { of: "pick", from: { catalogue: "tables" } },
              required: true,
              hint: "As database.table, any table the warehouse holds. A replacing table is read through FINAL without saying so.",
            },
            {
              name: "database",
              label: "Database",
              shape: { of: "text" },
              hint: "Only when the table above is written bare.",
            },
            produced("table"),
            windowedBy("table"),
            filtered("table"),
          ],
        },
      },
    },
    {
      name: "max_range",
      label: "Widest window",
      shape: { of: "text", placeholder: "P1Y" },
      hint: "An ISO duration of whole days, months or years.",
    },
    {
      name: "group_by",
      label: "Grouped by",
      shape: {
        of: "list",
        entry: { of: "pick", from: OWN_COLUMN, also: ["bucket"] },
        entryLabel: "column",
      },
      hint: "Names this metric produces: an `as_name`, or `bucket` for a windowed run.",
    },
    {
      name: "order_by",
      label: "Ordered by",
      hint: "Which rows come first, so a limit keeps the largest or the latest.",
      shape: {
        of: "record",
        fields: [
          {
            name: "field",
            label: "Column",
            hint: "One of this metric's own columns, or bucket.",
            shape: { of: "pick", from: OWN_COLUMN, also: ["bucket"] },
          },
          {
            name: "direction",
            label: "Direction",
            hint: "Ascending unless said otherwise.",
            shape: { of: "choice", options: DIRECTIONS },
          },
        ],
      },
    },
    {
      name: "limit",
      label: "Limit",
      hint: "How many rows at most, after ordering.",
      shape: { of: "number" },
    },
  ],
};

const METRIC_REFERENCE: Field = {
  name: "metric",
  label: "Metric",
  hint: "The stored metric this draws.",
  shape: { of: "reference", to: "metrics" },
  required: true,
};

/** A line, a bar and an area all read one column against another. */
const SERIES: readonly Field[] = [
  METRIC_REFERENCE,
  {
    name: "x",
    label: "x",
    hint: "The metric column along the horizontal axis; bucket for a run over time.",
    shape: { of: "text" },
    required: true,
  },
  {
    name: "y",
    label: "y",
    hint: "The metric column drawn as the value.",
    shape: { of: "text" },
    required: true,
  },
];

const WIDGET: Description = {
  kind: "widgets",
  noun: "widget",
  fields: [
    {
      name: "type",
      label: "Type",
      hint: "What the metric is drawn as. Each type asks for the columns it needs.",
      shape: {
        of: "variants",
        recorded: "type",
        variants: {
          table: [
            METRIC_REFERENCE,
            {
              name: "columns",
              label: "Columns",
              hint: "The metric columns shown, in this order.",
              shape: {
                of: "list",
                entry: { of: "text" },
                entryLabel: "column",
              },
              required: true,
            },
          ],
          line: SERIES,
          bar: SERIES,
          area: SERIES,
          stat: [
            METRIC_REFERENCE,
            {
              name: "value",
              label: "Value",
              hint: "The metric column shown as the one number.",
              shape: { of: "text" },
              required: true,
            },
            {
              name: "label",
              label: "Label",
              hint: "A caption under the number.",
              shape: { of: "text" },
            },
          ],
          pie: [
            METRIC_REFERENCE,
            {
              name: "label",
              label: "Slices",
              hint: "The metric column that names the slices.",
              shape: { of: "text" },
              required: true,
            },
            {
              name: "value",
              label: "Value",
              hint: "The metric column that sizes the slices.",
              shape: { of: "text" },
              required: true,
            },
          ],
        },
      },
      required: true,
    },
    {
      name: "title",
      label: "Title",
      hint: "The heading a reader sees. Without it the card shows the identifier.",
      shape: { of: "text" },
    },
    {
      name: "detail",
      label: "Rows behind it",
      shape: { of: "reference", to: "metrics" },
      hint: "A metric run when a reader opens the card's rows: the records a chart's numbers were counted from. Empty, the card shows the rows of its own metric.",
    },
  ],
};

const DASHBOARD: Description = {
  kind: "dashboards",
  noun: "dashboard",
  fields: [
    {
      name: "title",
      label: "Title",
      hint: "The heading of the board.",
      shape: { of: "text" },
      required: true,
    },
    {
      name: "items",
      label: "Drawn, top to bottom",
      hint: "A stored widget by name, a section heading, or a paragraph of text.",
      shape: {
        of: "list",
        entryLabel: "item",
        entry: {
          of: "variants",
          variants: {
            widget: [
              {
                name: "widget",
                label: "Widget",
                hint: "A stored widget, by name.",
                shape: { of: "reference", to: "widgets" },
                required: true,
              },
            ],
            heading: [
              {
                name: "heading",
                label: "Heading",
                hint: "A section title between widgets.",
                shape: { of: "text" },
                required: true,
              },
            ],
            text: [
              {
                name: "text",
                label: "Text",
                hint: "A paragraph, shown as written.",
                shape: { of: "longText" },
                required: true,
              },
            ],
          },
        },
      },
    },
    {
      name: "time_ranges",
      label: "Windows offered",
      shape: { of: "list", entry: { of: "text" }, entryLabel: "window" },
      hint: "The windows a reader may pick: PDC yesterday, P7D and P30D the last so many days, PMC the last whole month, PQC the last whole quarter, P1Y the last 365 days, inf all time, or two dates as YYYY-MM-DD/YYYY-MM-DD. Empty, the board offers no window and reads all time.",
    },
    {
      name: "default_range",
      label: "Opens on",
      hint: "One of the windows offered, shown first.",
      shape: { of: "text" },
    },
  ],
};

export const DESCRIPTIONS = {
  datasets: DATASET,
  metrics: METRIC,
  widgets: WIDGET,
  dashboards: DASHBOARD,
} as const;

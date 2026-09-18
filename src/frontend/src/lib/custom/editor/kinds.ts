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

const CONDITION: Shape = {
  of: "record",
  fields: [
    {
      name: "field",
      label: "Field",
      hint: "The declared field this compares.",
      shape: { of: "pick", from: DECLARED },
      required: true,
    },
    {
      name: "type",
      label: "Type",
      hint: "The type the value is written as here. The service compares as the field's declared type.",
      shape: { of: "choice", options: ["string", "int", "float"] },
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
      hint: "What the field is compared with, read as the field's declared type.",
      shape: {
        of: "typed",
        by: "type",
        declared: { dataset: "dataset", named: "field" },
      },
      required: true,
    },
  ],
};

const OWN_COLUMN = { list: "fields", property: "as_name" } as const;

const METRIC: Description = {
  kind: "metrics",
  noun: "metric",
  fields: [
    {
      name: "dataset",
      label: "Dataset",
      shape: { of: "reference", to: "datasets" },
      required: true,
      hint: "Every metric reads one, by its declared fields.",
    },
    {
      name: "fields",
      label: "Fields",
      hint: "The columns this metric produces, one per entry: a field read as is, or an aggregate over one.",
      shape: {
        of: "list",
        entryLabel: "field",
        entry: {
          of: "record",
          fields: [
            {
              name: "field",
              label: "Reads",
              shape: { of: "pick", from: DECLARED },
              hint: "A declared field. Leave empty for a count of the records.",
            },
            {
              name: "type",
              label: "Type",
              hint: "The type of the column produced: int for a count, the field's type otherwise.",
              shape: { of: "choice", options: ["string", "int", "float"] },
              required: true,
            },
            {
              name: "agg",
              label: "Aggregate",
              hint: "Leave empty to read the field as is. A count needs no field.",
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
              shape: { of: "list", entry: CONDITION, entryLabel: "condition" },
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
    },
    {
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
      name: "filters",
      label: "Filtered",
      hint: "Rows the whole metric reads. A condition on one aggregate alone goes under that aggregate.",
      shape: { of: "list", entry: CONDITION, entryLabel: "filter" },
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
      hint: "The windows a reader may pick, as the service names them: PDC, P7D, P30D, PMC, PQC, P1Y, inf, or two dates as YYYY-MM-DD/YYYY-MM-DD.",
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

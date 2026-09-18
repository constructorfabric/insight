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
    { name: "name", label: "Name", shape: { of: "text" }, required: true },
    {
      name: "path",
      label: "Path",
      shape: { of: "text", placeholder: "who.email" },
      required: true,
      hint: "Where the value sits in a record, as dot-separated keys.",
    },
    {
      name: "type",
      label: "Type",
      shape: { of: "choice", options: FIELD_TYPES },
      required: true,
    },
    {
      name: "role",
      label: "Role",
      shape: { of: "choice", options: FIELD_ROLES },
    },
    { name: "description", label: "Description", shape: { of: "text" } },
    {
      name: "absent_value",
      label: "Shown when empty",
      shape: { of: "text" },
      hint: "Presentation only: a filter and the row identity read the record's own value.",
    },
    {
      name: "person",
      label: "Holds a person by",
      shape: { of: "choice", options: PERSON_HANDLES },
    },
    {
      name: "default_clock",
      label: "Main date",
      shape: { of: "flag", alone: true },
      hint: "A window with no date of its own selects by this one.",
    },
  ],
};

const DATASET: Description = {
  kind: "datasets",
  noun: "dataset",
  fields: [
    { name: "title", label: "Title", shape: { of: "text" }, required: true },
    { name: "description", label: "Description", shape: { of: "longText" } },
    {
      name: "fields",
      label: "Fields",
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
      hint: "Leave empty to keep every record on its own.",
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
      shape: { of: "pick", from: DECLARED },
      required: true,
    },
    {
      name: "type",
      label: "Type",
      shape: { of: "choice", options: ["string", "int", "float"] },
      required: true,
    },
    {
      name: "op",
      label: "Compares",
      shape: { of: "choice", options: OPERATORS },
      required: true,
    },
    {
      name: "value",
      label: "Against",
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
              shape: { of: "choice", options: ["string", "int", "float"] },
              required: true,
            },
            {
              name: "agg",
              label: "Aggregate",
              shape: { of: "choice", options: AGGREGATES },
            },
            {
              name: "as_name",
              label: "Called",
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
      shape: { of: "list", entry: CONDITION, entryLabel: "filter" },
    },
    {
      name: "order_by",
      label: "Ordered by",
      shape: {
        of: "record",
        fields: [
          {
            name: "field",
            label: "Column",
            shape: { of: "pick", from: OWN_COLUMN, also: ["bucket"] },
          },
          {
            name: "direction",
            label: "Direction",
            shape: { of: "choice", options: DIRECTIONS },
          },
        ],
      },
    },
    { name: "limit", label: "Limit", shape: { of: "number" } },
  ],
};

const METRIC_REFERENCE: Field = {
  name: "metric",
  label: "Metric",
  shape: { of: "reference", to: "metrics" },
  required: true,
};

/** A line, a bar and an area all read one column against another. */
const SERIES: readonly Field[] = [
  METRIC_REFERENCE,
  { name: "x", label: "x", shape: { of: "text" }, required: true },
  { name: "y", label: "y", shape: { of: "text" }, required: true },
];

const WIDGET: Description = {
  kind: "widgets",
  noun: "widget",
  fields: [
    {
      name: "type",
      label: "Type",
      shape: {
        of: "variants",
        recorded: "type",
        variants: {
          table: [
            METRIC_REFERENCE,
            {
              name: "columns",
              label: "Columns",
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
              shape: { of: "text" },
              required: true,
            },
            { name: "label", label: "Label", shape: { of: "text" } },
          ],
          pie: [
            METRIC_REFERENCE,
            {
              name: "label",
              label: "Slices",
              shape: { of: "text" },
              required: true,
            },
            {
              name: "value",
              label: "Value",
              shape: { of: "text" },
              required: true,
            },
          ],
        },
      },
      required: true,
    },
    { name: "title", label: "Title", shape: { of: "text" } },
    {
      name: "detail",
      label: "Rows behind it",
      shape: { of: "reference", to: "metrics" },
      hint: "The rows behind the chart, when they are a different query.",
    },
  ],
};

const DASHBOARD: Description = {
  kind: "dashboards",
  noun: "dashboard",
  fields: [
    { name: "title", label: "Title", shape: { of: "text" }, required: true },
    {
      name: "items",
      label: "Drawn, top to bottom",
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
                shape: { of: "reference", to: "widgets" },
                required: true,
              },
            ],
            heading: [
              {
                name: "heading",
                label: "Heading",
                shape: { of: "text" },
                required: true,
              },
            ],
            text: [
              {
                name: "text",
                label: "Text",
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
      hint: "Server tokens: PDC, P7D, P30D, PMC, PQC, P1Y, inf.",
    },
    { name: "default_range", label: "Opens on", shape: { of: "text" } },
  ],
};

export const DESCRIPTIONS = {
  datasets: DATASET,
  metrics: METRIC,
  widgets: WIDGET,
  dashboards: DASHBOARD,
} as const;

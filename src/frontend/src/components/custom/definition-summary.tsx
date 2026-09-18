import type { ReactNode } from "react";
import { Link } from "@tanstack/react-router";

import type { MetricDefinition, Widget } from "@/api/custom-client";
import { Badge } from "@/components/ui/badge";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/**
 * What a stored definition says, read back.
 *
 * The catalogue pages and a widget's drilldown show the same thing — the query
 * as it is stored — so they render it from here rather than each having its own
 * idea of what a metric looks like.
 */

/** Where a field or filter reads its value: a column, or a payload key. */
function source(of: { column?: string; json?: string }): string {
  return of.column ?? of.json ?? "";
}

/** One field, as the query reads it. */
function reads(field: MetricDefinition["fields"][number]): string {
  const read = field.agg ? `${field.agg}(${source(field)})` : source(field);
  const named = field.person ? `${read} by name` : read;

  return `${named} as ${field.as_name}`;
}

export function MetricSummary({
  definition,
}: {
  definition: MetricDefinition;
}) {
  const grouped = definition.group_by ?? [];
  const filters = definition.filters ?? [];

  return (
    <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5">
      <Row label="Table">
        <code className="font-mono">
          {definition.database
            ? `${definition.database}.${definition.table}`
            : definition.table}
        </code>
      </Row>
      <Row label="Fields">
        <span className="font-mono">
          {definition.fields.map(reads).join(", ")}
        </span>
      </Row>
      {grouped.length ? (
        <Row label="Grouped by">
          <span className="font-mono">{grouped.join(", ")}</span>
        </Row>
      ) : null}
      {filters.length ? (
        <Row label="Filtered">
          <span className="font-mono">
            {filters
              .map((f) => `${source(f)} ${f.op} ${String(f.value)}`)
              .join(", ")}
          </span>
        </Row>
      ) : null}
      {definition.order_by ? (
        <Row label="Ordered by">
          <span className="font-mono">
            {definition.order_by.field} {definition.order_by.direction ?? "asc"}
          </span>
        </Row>
      ) : null}
      {definition.limit ? (
        <Row label="Limit">
          <span className="font-mono">{definition.limit}</span>
        </Row>
      ) : null}
    </dl>
  );
}

/** What the row beside the metric is called, per kind. */
function drawnLabel(widget: Widget): string {
  switch (widget.type) {
    case "table":
      return "Columns";
    case "stat":
      return "Value";
    case "pie":
      return "Slices";
    default:
      return "Axes";
  }
}

/** The columns this widget reads, as it reads them. */
function drawn(widget: Widget): string {
  switch (widget.type) {
    case "table":
      return widget.columns.join(", ");
    case "stat":
      return widget.label ? `${widget.value} as ${widget.label}` : widget.value;
    case "pie":
      return `${widget.label} by ${widget.value}`;
    default:
      return `x ${widget.x} · y ${widget.y}`;
  }
}

export function WidgetSummary({
  widget,
  linkMetric = true,
}: {
  widget: Widget;
  linkMetric?: boolean;
}) {
  return (
    <div className="flex flex-col gap-2">
      <Badge variant="secondary" className="w-fit font-mono">
        {widget.type}
      </Badge>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5">
        <Row label="Metric">
          {linkMetric ? (
            <Link
              to="/portal/custom/metrics"
              className="font-mono underline decoration-dotted underline-offset-4"
            >
              {widget.metric}
            </Link>
          ) : (
            <span className="font-mono">{widget.metric}</span>
          )}
        </Row>
        {widget.detail ? (
          <Row label="Rows behind it">
            <span className="font-mono">{widget.detail}</span>
          </Row>
        ) : null}
        <Row label={drawnLabel(widget)}>
          <span className="font-mono">{drawn(widget)}</span>
        </Row>
      </dl>
    </div>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className={TEXT_LABEL}>{label}</dt>
      <dd className={cn(TEXT_BODY, "min-w-0 break-words")}>{children}</dd>
    </>
  );
}

import type { ReactElement } from "react";
import { Tooltip, Treemap, type TreemapNode } from "recharts";

import type { CategoryWidget, MetricResult } from "@/api/custom-client";
import { groupedNumber, unitFor } from "@/components/custom/chart-format";

import { categoryRows } from "../adapters/category";
import { tooltipContent } from "../chart-style";
import { KindChart, KindFigure } from "../chrome";
import { seriesColor } from "../palette";

const CHAR_WIDTH = 6.8;

export function TreemapKind({
  widget,
  result,
}: {
  widget: CategoryWidget;
  result: MetricResult;
}) {
  const unit = unitFor(result.percents, widget.value);
  const tiles = categoryRows(result, widget.label, widget.value, {
    positiveOnly: true,
    order: "desc",
  }).map(({ label, value }) => ({ name: label, value }));

  return (
    <KindFigure kind="treemap">
      <KindChart>
        <Treemap
          data={tiles}
          dataKey="value"
          nameKey="name"
          aspectRatio={1.5}
          content={(node: TreemapNode) => tile(node, unit)}
          isAnimationActive={false}
        >
          <Tooltip content={tooltipContent({ unit, hideLabel: true })} />
        </Treemap>
      </KindChart>
    </KindFigure>
  );
}

function tile(node: TreemapNode, unit: string): ReactElement {
  if (node.depth === 0 || node.children?.length) return <g />;

  const color = seriesColor(node.index);
  const roomy = node.width >= 62 && node.height >= 34;
  const fits = Math.max(3, Math.floor((node.width - 24) / CHAR_WIDTH));
  const name = String(node.name ?? "");
  const label = name.length > fits ? `${name.slice(0, fits - 1)}…` : name;

  return (
    <g>
      <title>{`${name}: ${groupedNumber(node.value, unit)}`}</title>
      <rect
        x={node.x}
        y={node.y}
        width={Math.max(0, node.width)}
        height={Math.max(0, node.height)}
        rx={6}
        fill={`color-mix(in srgb, ${color} 22%, var(--card))`}
        stroke={`color-mix(in srgb, ${color} 35%, var(--card))`}
        strokeWidth={1}
      />
      {roomy ? (
        <text
          x={node.x + 12}
          y={node.y + 21}
          fill="var(--foreground)"
          fontSize={12}
          fontWeight={600}
        >
          {label}
        </text>
      ) : null}
      {roomy && node.height >= 54 ? (
        <text
          x={node.x + 12}
          y={node.y + 40}
          fill="var(--muted-foreground)"
          fontSize={11}
        >
          {groupedNumber(node.value, unit)}
        </text>
      ) : null}
    </g>
  );
}

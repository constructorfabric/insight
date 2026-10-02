import { Label, PolarAngleAxis, RadialBar, RadialBarChart } from "recharts";

import type { MetricResult, RadialWidget } from "@/api/custom-client";

import { progress } from "../adapters/category";
import { GRID_STROKE } from "../chart-style";
import { KindChart, KindFigure } from "../chrome";
import { seriesColor } from "../palette";

export function RadialKind({
  widget,
  result,
}: {
  widget: RadialWidget;
  result: MetricResult;
}) {
  const percent = progress(result, widget.value, widget.max);
  const color = seriesColor(0);

  return (
    <KindFigure
      kind="radial"
      legend={[
        { label: "Done", color },
        { label: "Remaining", color: GRID_STROKE },
      ]}
    >
      <KindChart>
        <RadialBarChart
          data={[{ name: widget.value, value: percent ?? 0 }]}
          innerRadius="72%"
          outerRadius="94%"
          startAngle={90}
          endAngle={-270}
        >
          <PolarAngleAxis type="number" domain={[0, 100]} tick={false} />
          <RadialBar
            dataKey="value"
            cornerRadius={20}
            background={{ fill: GRID_STROKE }}
            fill={color}
            maxBarSize={24}
            isAnimationActive={false}
          >
            <Label
              value={percent === null ? "—" : `${Math.round(percent)}%`}
              position="center"
              fill="var(--foreground)"
              fontSize={28}
              fontWeight={650}
            />
          </RadialBar>
        </RadialBarChart>
      </KindChart>
    </KindFigure>
  );
}

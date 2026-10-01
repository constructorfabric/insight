import type {
  MetricResult,
  StatWidget,
  TableWidget,
  Widget,
} from "@/api/custom-client";

import { AreaKind } from "./area";
import { BarKind } from "./bar";
import { ComposedKind } from "./composed";
import { DonutKind } from "./donut";
import { FunnelKind } from "./funnel";
import { HeatmapKind } from "./heatmap";
import { LineKind } from "./line";
import { PulseKind } from "./pulse";
import { RadarKind } from "./radar";
import { RadialKind } from "./radial";
import { RankedKind } from "./ranked";
import { ScatterKind } from "./scatter";
import { StackedKind } from "./stacked";
import { TreemapKind } from "./treemap";
import { WaterfallKind } from "./waterfall";

export type ChartWidget = Exclude<Widget, TableWidget | StatWidget>;

export function ChartKind({
  widget,
  result,
}: {
  widget: ChartWidget;
  result: MetricResult;
}) {
  switch (widget.type) {
    case "line":
      return <LineKind widget={widget} result={result} />;
    case "bar":
      return <BarKind widget={widget} result={result} />;
    case "area":
      return <AreaKind widget={widget} result={result} />;
    case "pie":
    case "donut":
      return <DonutKind widget={widget} result={result} />;
    case "ranked":
      return <RankedKind widget={widget} result={result} />;
    case "treemap":
      return <TreemapKind widget={widget} result={result} />;
    case "funnel":
      return <FunnelKind widget={widget} result={result} />;
    case "waterfall":
      return <WaterfallKind widget={widget} result={result} />;
    case "stacked":
      return <StackedKind widget={widget} result={result} />;
    case "composed":
      return <ComposedKind widget={widget} result={result} />;
    case "scatter":
    case "bubble":
      return <ScatterKind widget={widget} result={result} />;
    case "radar":
      return <RadarKind widget={widget} result={result} />;
    case "radial":
      return <RadialKind widget={widget} result={result} />;
    case "heatmap":
      return <HeatmapKind widget={widget} result={result} />;
    case "pulse":
      return <PulseKind widget={widget} result={result} />;
  }
}

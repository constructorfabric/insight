import { CartesianGrid, Tooltip, XAxis, YAxis } from "recharts";

import { compactNumber } from "@/components/custom/chart-format";

import { AXIS_TICK, CATEGORY_AXIS_WIDTH, tooltipContent } from "../chart-style";
import { CategoryTick } from "../chrome";

const SHARE_TICKS = [0, 25, 50, 75, 100];

export function CategoryAxes({
  dataKey,
  unit = "",
  share = false,
}: {
  dataKey: string;
  unit?: string;
  share?: boolean;
}) {
  return (
    <>
      <CartesianGrid horizontal={false} />
      <XAxis
        type="number"
        domain={share ? [0, 100] : undefined}
        ticks={share ? SHARE_TICKS : undefined}
        tick={AXIS_TICK}
        tickLine={false}
        axisLine={false}
        tickFormatter={(value) => compactNumber(value, share ? "%" : unit)}
      />
      <YAxis
        type="category"
        dataKey={dataKey}
        interval={0}
        tick={<CategoryTick />}
        tickLine={false}
        axisLine={false}
        width={CATEGORY_AXIS_WIDTH}
      />
      <Tooltip content={tooltipContent({ unit: share ? "%" : unit })} />
    </>
  );
}

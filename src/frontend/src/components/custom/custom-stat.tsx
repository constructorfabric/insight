import type { MetricResult } from "@/api/custom-client";
import { groupedNumber, unitFor } from "@/components/custom/chart-format";
import { TEXT_FIGURE, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export interface CustomStatProps {
  result: MetricResult;
  value: string;
  label?: string;
}

/**
 * One number.
 *
 * "How many commits are there" answers with a single row and a single column,
 * which a table renders as a header above one cell.
 */
export function CustomStat({ result, value, label }: CustomStatProps) {
  const at = result.columns.indexOf(value);
  const [first] = result.rows;

  return (
    <div data-testid="custom-stat" className="flex flex-col gap-1">
      <span className={TEXT_FIGURE}>
        {groupedNumber(first?.[at], unitFor(result.percents, value))}
      </span>
      <span className={cn(TEXT_LABEL, "font-mono")}>{label ?? value}</span>
    </div>
  );
}

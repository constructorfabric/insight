import { Empty, EmptyDescription, EmptyHeader } from "@/components/ui/empty";
import { cn } from "@/lib/utils";

/**
 * Placeholder for a chart with no data — centers the message in the plot area
 * so an empty chart keeps its neighbours' footprint instead of collapsing to
 * a line of text. No frame of its own: the enclosing card is the border.
 * `className` carries the plot height so it matches the chart it stands in for.
 */
export function ChartEmpty({
  message,
  className,
}: {
  message: string;
  className?: string;
}) {
  return (
    <Empty className={cn("min-h-48 p-6", className)}>
      <EmptyHeader>
        <EmptyDescription>{message}</EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}

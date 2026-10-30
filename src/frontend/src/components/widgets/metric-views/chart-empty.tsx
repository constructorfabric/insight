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
    <div
      className={cn(
        "flex min-h-48 w-full min-w-0 flex-1 flex-col items-center justify-center p-6 text-center text-balance",
        className
      )}
    >
      <p className="max-w-sm text-sm/relaxed text-muted-foreground">
        {message}
      </p>
    </div>
  );
}

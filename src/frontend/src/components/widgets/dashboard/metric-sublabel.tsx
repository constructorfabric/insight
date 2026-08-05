import { useSettings } from "@/hooks/use-settings";
import { cn } from "@/lib/utils";

export interface MetricSublabelProps {
  description?: string;
  className?: string;
}

/**
 * Inline metric description — visible only when the Explanations toggle is ON.
 */
export function MetricSublabel({
  description,
  className,
}: MetricSublabelProps) {
  const { showExplanations } = useSettings();
  if (!showExplanations || !description) return null;
  return (
    <p
      className={cn(
        "line-clamp-2 text-xs leading-snug text-muted-foreground",
        className,
      )}
    >
      {description}
    </p>
  );
}

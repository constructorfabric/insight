import { Spinner } from "@/components/ui/spinner";
import { cn } from "@/lib/utils";

/**
 * The one loading state for dashboards and drilldowns: a single centered
 * spinner over the whole surface while its queries load. Widgets never show
 * their own loaders — the screen swaps between this and fully-loaded content.
 */
export function CenteredSpinner({ className }: { className?: string }) {
  return (
    // The Spinner itself announces the status role; the wrapper only centers.
    <div className={cn("flex items-center justify-center", className)}>
      <Spinner className="size-12 text-muted-foreground" />
    </div>
  );
}

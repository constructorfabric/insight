import { Spinner } from "@/components/ui/spinner";
import { useAutoLoadOnScroll } from "@/hooks/use-auto-load-on-scroll";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/** What a catalogue reads a page at a time, and how it asks for the rest. */
export interface Paging {
  /** Every match, not the page — undefined until the first page lands. */
  total: number | undefined;
  hasMore: boolean;
  isFetchingMore: boolean;
  onMore: () => void;
  /** Whether a needle is narrowing the count. */
  searching: boolean;
}

/** How many of a kind there are, or how many a search matched. */
export function DefinitionCount({
  total,
  noun,
  searching,
}: {
  total: number | undefined;
  noun: string;
  searching: boolean;
}) {
  if (total === undefined) {
    return null;
  }

  return (
    <p className={cn(TEXT_LABEL, "shrink-0 text-muted-foreground")}>
      {searching ? `${total} matching` : `${total} ${noun}`}
    </p>
  );
}

/** The end of a catalogue, which asks for the next page as it comes into view. */
export function MoreDefinitions({
  hasMore,
  isFetchingMore,
  onMore,
}: Omit<Paging, "total" | "searching">) {
  const marker = useAutoLoadOnScroll({
    hasNextPage: hasMore,
    isFetchingNextPage: isFetchingMore,
    fetchNextPage: onMore,
  });

  return (
    <div ref={marker} className="mt-4 flex min-h-4 justify-center">
      {isFetchingMore ? <Spinner className="size-4" /> : null}
    </div>
  );
}

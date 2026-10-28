import { useState } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";

import type { EditableKind } from "@/api/custom-client";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { definitionPagesQuery, type Narrowing } from "@/queries/custom";

/** Long enough that a word is typed before the service hears about it. */
const SEARCH_DEBOUNCE_MS = 400;

/**
 * One catalogue: the definitions read so far, how many are behind them, and
 * the box that narrows both.
 *
 * The needle is debounced and trimmed before it reaches the query key, so a
 * word typed at speed is one request rather than one per keystroke.
 */
export function useDefinitionCatalogue<K extends EditableKind>(
  kind: K,
  { enabled = true, ...narrowing }: Narrowing & { enabled?: boolean } = {}
) {
  const [needle, setNeedle] = useState("");
  const searching = useDebouncedValue(needle, SEARCH_DEBOUNCE_MS).trim();
  const catalogue = useInfiniteQuery({
    ...definitionPagesQuery(kind, searching, narrowing),
    enabled,
  });
  const pages = catalogue.data?.pages;

  return {
    items: pages?.flatMap((page) => page.items),
    isLoading: catalogue.isLoading || !enabled,
    isError: catalogue.isError,
    refetch: () => void catalogue.refetch(),
    search: { value: needle, onChange: setNeedle },
    paging: {
      // The last page's count, not the first's: a definition stored while the
      // reader is paging changes it.
      total: pages?.at(-1)?.total,
      hasMore: catalogue.hasNextPage,
      isFetchingMore: catalogue.isFetchingNextPage,
      onMore: () => void catalogue.fetchNextPage(),
      searching: searching !== "",
    },
  };
}

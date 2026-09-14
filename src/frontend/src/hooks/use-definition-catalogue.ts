import { useState } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";

import type { DefinitionKind } from "@/api/custom-client";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { definitionPagesQuery } from "@/queries/custom";

/** Long enough that a word is typed before the service hears about it. */
const SEARCH_DEBOUNCE_MS = 400;

/**
 * One catalogue: the names read so far, how many are behind them, and the box
 * that narrows both.
 *
 * The needle is debounced and trimmed before it reaches the query key, so a
 * word typed at speed is one request rather than one per keystroke.
 */
export function useDefinitionCatalogue(kind: DefinitionKind) {
  const [needle, setNeedle] = useState("");
  const searching = useDebouncedValue(needle, SEARCH_DEBOUNCE_MS).trim();
  const catalogue = useInfiniteQuery(definitionPagesQuery(kind, searching));
  const pages = catalogue.data?.pages;

  return {
    names: pages?.flatMap((page) => page.names),
    isLoading: catalogue.isLoading,
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

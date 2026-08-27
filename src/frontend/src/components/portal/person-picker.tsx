/**
 * The person picker: find someone by any current identity value (name, any
 * email ever current, git handle) and hand the choice back to the caller.
 *
 * A finding tool, not a deciding one (#2424): it never fires a verb itself.
 * Composed from the house search idiom (Search icon inside an Input) over a
 * debounced live query, paged with the cursor the service returns, so a common
 * term reaches every match instead of stopping at the first page. Terminated
 * people stay visible but marked — finding a leaver is often the point.
 *
 * `browseWhenEmpty` decides what an empty field means. The console's person
 * mode wants the roster there; the assign picker inside an account dialog
 * wants matches, and listing the tenant into a dropdown would bury the one
 * name the operator came to type.
 */
import { Search } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { PersonSummary } from "@/api/identity-client";
import { PersonCell } from "@/components/portal/person-cell";
import { personDisplayName } from "@/lib/identities/person-display";
import { activatesRow, activatesRowByKey } from "@/lib/identities/row-activation";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { ScrollToEnds } from "@/components/widgets/scroll-to-ends";
import { useAutoLoadOnScroll } from "@/hooks/use-auto-load-on-scroll";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import {
  belowPersonFloor,
  listsAnyone,
  MIN_SEARCH_CHARS,
  SEARCH_DEBOUNCE_MS,
  usePersonList,
} from "@/queries/identity-resolution";
import { cn } from "@/lib/utils";

export function PersonPicker({
  onPick,
  /** Persons already shown elsewhere in the panel — filtered out, not repeated. */
  excludeIds = [],
  browseWhenEmpty = false,
  /**
   * The picker IS the surface rather than a field inside a panel: the list gets
   * the card the console's other listings have, fills the height it is given,
   * and carries the jump-to-either-end control a long roster needs. In a panel
   * it stays a short list under a field, where a full-height scroller would
   * take the dialog over.
   */
  asSurface = false,
  initialQuery = "",
  onSettled,
}: {
  onPick: (person: PersonSummary) => void;
  excludeIds?: string[];
  browseWhenEmpty?: boolean;
  asSurface?: boolean;
  /** What the field starts with — a caller that remembers the terms elsewhere
   *  hands them back here on the way in. Read once, on mount: while the field
   *  is on screen it is the only source of truth for what it holds. */
  initialQuery?: string;
  /** The terms once the reader has stopped typing, for a caller that outlives
   *  this field and wants them back when it returns. Fired debounced, not per
   *  keystroke: a caller that stores them in the URL would otherwise navigate
   *  on every letter. */
  onSettled?: (query: string) => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState(initialQuery);
  const debounced = useDebouncedValue(query, SEARCH_DEBOUNCE_MS);
  // Reported from an effect, not from the change handler: `debounced` is what
  // the search itself runs on, so the caller is told exactly what was asked.
  const settled = useRef(onSettled);
  useEffect(() => {
    settled.current = onSettled;
  }, [onSettled]);
  useEffect(() => {
    settled.current?.(debounced);
  }, [debounced]);

  const intent = browseWhenEmpty ? "browse" : "match";
  const asked = listsAnyone(debounced, intent);
  const list = usePersonList(debounced, intent);

  const excluded = new Set(excludeIds);
  const results = asked
    ? (list.data?.pages ?? [])
        .flatMap((page) => page.items)
        .filter((p) => !excluded.has(p.person_id))
    : [];
  const loading = list.isFetching && !list.isFetchingNextPage;
  // Reads the live field rather than the debounced one: the answer to "why is
  // nothing happening" should not wait for the debounce to expire. Only while
  // nothing is listed, though — deleting a character from a searched term keeps
  // its rows for one debounce, and a notice above them contradicts them.
  const tooShort = results.length === 0 && belowPersonFloor(query);
  // These rows answer the term the query key carries, which is not the one in
  // the field until the debounce fires AND the fetch lands. Kept on screen so
  // the list does not blank between keystrokes — see `placeholderData` — and
  // marked, or the operator reads a list that is one term behind as the answer.
  // Marked, NOT disabled: a click lands on a row they can read, and picking is
  // a step towards a verb rather than the verb itself.
  const behind = query !== debounced || list.isPlaceholderData;
  // A page whose every row was excluded is not "nobody matches" while more
  // pages are unread — the next page is the answer, not the message.
  const exhausted = !list.hasNextPage;
  const scroller = useRef<HTMLDivElement>(null);
  const loadMore = useAutoLoadOnScroll({
    hasNextPage: Boolean(asked && list.hasNextPage),
    isFetchingNextPage: list.isFetchingNextPage,
    fetchNextPage: () => void list.fetchNextPage(),
    root: scroller,
  });

  return (
    <div
      className={cn(
        "flex flex-col gap-2",
        asSurface && "min-h-0 flex-1",
      )}
    >
      <div className="relative w-full shrink-0">
        <Search className="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="search"
          className="ps-8"
          placeholder={t("identities.picker.placeholder")}
          aria-label={t("identities.picker.placeholder")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
      {loading ? (
        <div className="flex justify-center py-2">
          <Spinner className="size-4" />
        </div>
      ) : null}
      {tooShort ? (
        <p className="text-sm text-muted-foreground">
          {t("identities.picker.min_chars", { min: MIN_SEARCH_CHARS })}
        </p>
      ) : null}
      {asked && list.isError ? (
        <p className="text-sm text-destructive">
          {t("identities.picker.failed")}
        </p>
      ) : null}
      {asked &&
      list.data &&
      results.length === 0 &&
      !loading &&
      !behind &&
      exhausted ? (
        <p className="text-sm text-muted-foreground">
          {t("identities.picker.no_matches")}
        </p>
      ) : null}
      {/* Rendered for an unread next page even with nothing visible: a page
          whose every row was excluded has no rows to scroll, and the marker is
          the only thing that can ask for the page that does. It has to live
          INSIDE the scroller — an observer never reports a target that is not a
          descendant of its root. */}
      {results.length > 0 || (asked && list.hasNextPage) ? (
        <ListShell asSurface={asSurface}>
          <div
            ref={scroller}
            className={cn(
              "overflow-y-auto",
              asSurface ? "min-h-0 flex-1 p-2" : "max-h-64",
            )}
          >
            <ul
              aria-busy={behind}
              className={cn("flex flex-col gap-1", behind && "opacity-60")}
            >
              {results.map((person) => (
                <li key={person.person_id}>
                  {/* Not a <button>: the card inside carries its own copy control,
                      and a button may not nest a button — the same rule the queue
                      rows follow. */}
                  <div
                    role="button"
                    tabIndex={0}
                    onClick={(event) => {
                      if (activatesRow(event)) onPick(person);
                    }}
                    onKeyDown={(event) => {
                      if (!activatesRowByKey(event)) return;
                      event.preventDefault();
                      onPick(person);
                    }}
                    aria-label={personDisplayName(person)}
                    className="w-full cursor-pointer rounded-md border border-transparent p-2 text-start select-text hover:bg-muted/60"
                  >
                    <PersonCell person={person} />
                  </div>
                </li>
              ))}
            </ul>
            {/* The page after this one is asked for when this marker nears the
                viewport, so the roster continues instead of ending in a button.
                The marker is always there while a page is unread — the observer
                needs an element — but it only SAYS anything while that page is
                on its way. */}
            {asked && list.hasNextPage ? (
              <p
                ref={loadMore}
                aria-live="polite"
                className="p-2 text-sm text-muted-foreground"
              >
                {list.isFetchingNextPage
                  ? t("identities.picker.loading_more")
                  : null}
              </p>
            ) : null}
          </div>
          {asSurface ? (
            <ScrollToEnds scroller={scroller} rows={results.length} />
          ) : null}
        </ListShell>
      ) : null}
    </div>
  );
}

/** The card every listing in the console sits in — absent inside a panel. */
function ListShell({
  asSurface,
  children,
}: {
  asSurface: boolean;
  children: React.ReactNode;
}) {
  if (!asSurface) return children;
  return (
    <Card className="relative flex min-h-0 flex-1 flex-col overflow-hidden py-0">
      {children}
    </Card>
  );
}

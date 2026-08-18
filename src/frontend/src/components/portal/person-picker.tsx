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
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { PersonSummary } from "@/api/identity-client";
import { PersonCell } from "@/components/portal/person-cell";
import { personDisplayName } from "@/lib/identities/person-display";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { listsAnyone, usePersonList } from "@/queries/identity-resolution";
import { cn } from "@/lib/utils";

const DEBOUNCE_MS = 250;

export function PersonPicker({
  onPick,
  /** Persons already shown elsewhere in the panel — filtered out, not repeated. */
  excludeIds = [],
  browseWhenEmpty = false,
  /** Taller where the picker IS the view rather than a field inside a dialog. */
  className,
}: {
  onPick: (person: PersonSummary) => void;
  excludeIds?: string[];
  browseWhenEmpty?: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query, DEBOUNCE_MS);
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
  // A page whose every row was excluded is not "nobody matches" while more
  // pages are unread — the button below is the answer, not the message.
  const exhausted = !list.hasNextPage;

  return (
    <div className="flex flex-col gap-2">
      <div className="relative w-full">
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
      {asked && list.isError ? (
        <p className="text-sm text-destructive">
          {t("identities.picker.failed")}
        </p>
      ) : null}
      {asked && list.data && results.length === 0 && !loading && exhausted ? (
        <p className="text-sm text-muted-foreground">
          {t("identities.picker.no_matches")}
        </p>
      ) : null}
      {results.length > 0 ? (
        <ul
          className={cn(
            "flex max-h-64 flex-col gap-1 overflow-y-auto",
            className,
          )}
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
                  if (
                    event.target instanceof Element &&
                    event.target.closest("button, a")
                  ) {
                    return;
                  }
                  onPick(person);
                }}
                onKeyDown={(event) => {
                  if (event.key !== "Enter" && event.key !== " ") return;
                  if (event.target !== event.currentTarget) return;
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
      ) : null}
      {asked && list.hasNextPage ? (
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="self-start"
          aria-busy={list.isFetchingNextPage}
          onClick={() => void list.fetchNextPage()}
        >
          {list.isFetchingNextPage
            ? t("identities.picker.loading_more")
            : t("identities.picker.load_more")}
        </Button>
      ) : null}
    </div>
  );
}

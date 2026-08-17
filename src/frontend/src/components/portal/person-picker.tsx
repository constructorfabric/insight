/**
 * The person picker: find someone by any current identity value (name, any
 * email ever current, git handle) and hand the choice back to the caller.
 *
 * A finding tool, not a deciding one (#2424): it never fires a verb itself.
 * Composed from the house search idiom (Search icon inside an Input) over a
 * debounced live query; a truncated answer says "narrow the terms" instead of
 * posing as complete, and terminated people stay visible but marked — finding
 * a leaver is often the point.
 */
import { Search } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { PersonSummary } from "@/api/identity-client";
import { PersonCell } from "@/components/portal/person-cell";
import { personDisplayName } from "@/lib/identities/person-display";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { usePersonSearch } from "@/queries/identity-resolution";

const DEBOUNCE_MS = 250;
const MIN_QUERY_CHARS = 2;

export function PersonPicker({
  onPick,
  /** Persons already shown elsewhere in the panel — filtered out, not repeated. */
  excludeIds = [],
}: {
  onPick: (person: PersonSummary) => void;
  excludeIds?: string[];
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query, DEBOUNCE_MS);
  const search = usePersonSearch(debounced);

  const excluded = new Set(excludeIds);
  const results = (search.data?.items ?? []).filter(
    (p) => !excluded.has(p.person_id),
  );
  const active = debounced.trim().length >= MIN_QUERY_CHARS;

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
      {search.isFetching ? (
        <div className="flex justify-center py-2">
          <Spinner className="size-4" />
        </div>
      ) : null}
      {active && search.isError ? (
        <p className="text-sm text-destructive">
          {t("identities.picker.failed")}
        </p>
      ) : null}
      {active && search.data && results.length === 0 && !search.isFetching ? (
        <p className="text-sm text-muted-foreground">
          {t("identities.picker.no_matches")}
        </p>
      ) : null}
      {results.length > 0 ? (
        <ul className="flex max-h-64 flex-col gap-1 overflow-y-auto">
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
      {search.data?.truncated ? (
        <p className="text-xs text-muted-foreground">
          {t("identities.picker.truncated")}
        </p>
      ) : null}
    </div>
  );
}

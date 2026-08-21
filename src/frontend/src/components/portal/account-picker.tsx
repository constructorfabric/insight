/**
 * The account picker: find an observed account by any value a source reports
 * and hand the choice back to the caller.
 *
 * The mirror of {@link PersonPicker}, and a finding tool for the same reason:
 * it never fires a verb itself. Inside the person window a click on a row
 * means "bind this one to the person I have open" — the same gesture the person
 * picker carries inside the account window, pointing the other way.
 *
 * A blank field lists nothing on purpose. The tenant's whole fold would bury
 * the handful of accounts the person actually holds, which is what the reader
 * is looking at right above this.
 */
import { Search } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { AccountMatch } from "@/api/identity-client";
import { PersonCell } from "@/components/portal/person-cell";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { useAutoLoadOnScroll } from "@/hooks/use-auto-load-on-scroll";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { accountKey } from "@/lib/identities/account-key";
import { personDisplayName } from "@/lib/identities/person-display";
import { activatesRow, activatesRowByKey } from "@/lib/identities/row-activation";
import {
  belowAccountFloor,
  listsAnyAccount,
  MIN_SEARCH_CHARS,
  SEARCH_DEBOUNCE_MS,
  useAccountList,
} from "@/queries/identity-resolution";
import { cn } from "@/lib/utils";

export function AccountPicker({
  onPick,
  /** Accounts already listed by the caller — filtered out, not repeated. */
  excludeKeys = [],
  placeholder,
}: {
  onPick: (account: AccountMatch) => void;
  excludeKeys?: string[];
  placeholder: string;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query, SEARCH_DEBOUNCE_MS);
  const search = useAccountList(debounced, "match");

  const excluded = new Set(excludeKeys);
  // The shared rule, never a second spelling of it: a list rendered for terms
  // the query never asked for is another caller's cache.
  const asked = listsAnyAccount(debounced, "match");
  const items = asked
    ? (search.data?.pages ?? [])
        .flatMap((page) => page.items)
        .filter((item) => !excluded.has(accountKey(item)))
    : [];
  const loading = search.isFetching && !search.isFetchingNextPage;
  // Reads the live field rather than the debounced one: the answer to "why is
  // nothing happening" should not wait for the debounce to expire. Only while
  // nothing is listed, though — for one debounce the rows still answer the
  // shorter term, and a notice above them contradicts them.
  const tooShort = items.length === 0 && belowAccountFloor(query);
  // These rows answer the term the query key carries, not the one in the field,
  // until the debounce fires AND the fetch lands. Marked rather than hidden, so
  // the list does not blank between keystrokes — and never read as the answer.
  const behind = query !== debounced || search.isPlaceholderData;
  // A page whose every row was filtered out is not "nothing matches" while more
  // pages are unread — the next page is the answer, not the message.
  const exhausted = !search.hasNextPage;
  const scroller = useRef<HTMLDivElement>(null);
  const loadMore = useAutoLoadOnScroll({
    hasNextPage: Boolean(asked && search.hasNextPage),
    isFetchingNextPage: search.isFetchingNextPage,
    fetchNextPage: () => void search.fetchNextPage(),
    root: scroller,
  });

  return (
    <div className="flex min-h-0 flex-col gap-2">
      <div className="relative w-full shrink-0">
        <Search className="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="search"
          className="ps-8"
          placeholder={placeholder}
          aria-label={placeholder}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        {loading ? (
          <Spinner className="absolute end-2.5 top-1/2 size-4 -translate-y-1/2" />
        ) : null}
      </div>
      {tooShort ? (
        <p className="text-sm text-muted-foreground">
          {t("identities.accounts.min_chars", { min: MIN_SEARCH_CHARS })}
        </p>
      ) : null}
      {asked && search.isError ? (
        <p className="text-sm text-destructive">
          {t("identities.accounts.failed")}
        </p>
      ) : null}
      {asked &&
      search.data &&
      items.length === 0 &&
      !loading &&
      !behind &&
      exhausted ? (
        <p className="text-sm text-muted-foreground">
          {t("identities.accounts.no_matches")}
        </p>
      ) : null}
      {/* Rendered for an unread next page even with nothing visible: a page
          whose every row was filtered out has no rows to scroll, and the marker
          is the only thing that can ask for the page that does. It has to live
          INSIDE the scroller — an observer never reports a target that is not a
          descendant of its root. */}
      {items.length > 0 || (asked && search.hasNextPage) ? (
        <div ref={scroller} className="min-h-0 max-h-64 overflow-y-auto">
          <ul
            aria-busy={behind}
            className={cn("flex flex-col gap-1", behind && "opacity-60")}
          >
            {items.map((item) => (
              <li key={accountKey(item)}>
                <AccountOption item={item} onPick={() => onPick(item)} />
              </li>
            ))}
          </ul>
          {asked && search.hasNextPage ? (
            <p
              ref={loadMore}
              aria-live="polite"
              className="p-2 text-sm text-muted-foreground"
            >
              {search.isFetchingNextPage
                ? t("identities.accounts.loading_more")
                : null}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/**
 * One account offered for binding — with whoever holds it now.
 *
 * The holder is the whole reason this row is not a plain label: binding an
 * account somebody else holds takes it off them, and that has to be visible
 * before the click, not in the confirmation after it.
 */
function AccountOption({
  item,
  onPick,
}: {
  item: AccountMatch;
  onPick: () => void;
}) {
  const { t } = useTranslation();
  const label =
    item.email?.trim() ||
    item.username?.trim() ||
    item.display_name?.trim() ||
    item.account_id;
  return (
    // Not a <button>: the holder's card carries its own copy control, and a
    // button may neither nest one nor let its text be selected — the same rule
    // the queue rows and the person picker follow.
    <div
      role="button"
      tabIndex={0}
      onClick={(event) => {
        if (activatesRow(event)) onPick();
      }}
      onKeyDown={(event) => {
        if (!activatesRowByKey(event)) return;
        event.preventDefault();
        onPick();
      }}
      // The holder is the whole reason this row is not a plain label, so the
      // name has to carry them: without it a reader hears the address, presses
      // Enter, and only learns in the confirmation that it is somebody else's.
      aria-label={[
        label,
        item.source,
        item.person
          ? t("identities.queue.bound_to", {
              name: personDisplayName(item.person),
            })
          : item.excluded
            ? t("identities.accounts.excluded")
            : t("identities.accounts.unbound"),
      ].join(", ")}
      className="grid w-full cursor-pointer grid-cols-1 items-center gap-2 rounded-md border border-transparent p-2 text-start select-text hover:bg-muted/60 md:grid-cols-[minmax(0,1fr)_minmax(0,18rem)]"
    >
      <div className="min-w-0">
        <div className="truncate text-sm font-medium">{label}</div>
        <div className="truncate font-mono text-xs text-muted-foreground">
          {item.source} · {item.account_id}
        </div>
      </div>
      {item.person ? (
        <PersonCell person={item.person} />
      ) : item.excluded ? (
        <Badge variant="outline" className="justify-self-start font-normal">
          {t("identities.accounts.excluded")}
        </Badge>
      ) : (
        <span className="text-xs text-muted-foreground">
          {t("identities.accounts.unbound")}
        </span>
      )}
    </div>
  );
}

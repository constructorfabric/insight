/**
 * Finding an account, and the window it is decided in.
 *
 * Two surfaces use one field. The accounts mode arrives with a value in hand —
 * a git login from a review, an address from a ticket — the question neither
 * other mode can answer, because both are entered through a person; there a
 * blank field lists what the connectors reported, since the accounts nobody
 * asks about are exactly the ones nobody finds by searching. Inside one person
 * the same field is how an account gets re-bound to them, and a blank field
 * lists nothing: the whole fold would bury the accounts they actually hold.
 */
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ScanSearch, Search } from "lucide-react";

import type { AccountMatch, PersonSummary } from "@/api/identity-client";
import { CaseDialog, type CaseRow } from "@/components/portal/case-dialog";
import { PersonCell } from "@/components/portal/person-cell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { ScrollToEnds } from "@/components/widgets/scroll-to-ends";
import { useAutoLoadOnScroll } from "@/hooks/use-auto-load-on-scroll";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { accountKey } from "@/lib/identities/account-key";
import { KIND_SEARCH_MATCH } from "@/lib/identities/cases";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { usePortalSearch } from "@/lib/portal/portal-search";
import {
  belowAccountFloor,
  listsAnyAccount,
  MIN_SEARCH_CHARS,
  SEARCH_DEBOUNCE_MS,
  useAccountList,
  type AccountListIntent,
} from "@/queries/identity-resolution";
import { cn } from "@/lib/utils";

/** The accounts mode: the field over the whole fold, nothing else on screen. */
export function AccountSearchView() {
  const { t } = useTranslation();
  return (
    <AccountFinder
      intent="browse"
      placeholder={t("identities.accounts.placeholder")}
    />
  );
}

/**
 * The field, its results, and the one case window they open into.
 *
 * INVARIANT: the window lives here and nowhere else on the surface. It is
 * opened by `?acct=` alone, so a second one on the same screen would open
 * beside this one on the same link. A surface that lists accounts of its own
 * hands them over as `alsoOpenable` instead of rendering a window for them.
 */
export function AccountFinder({
  intent,
  placeholder,
  /** Rows the surface already lists, so its own accounts open in this window. */
  alsoOpenable = [],
  /** Offer binding straight to this person — the one the surface has open. */
  bindTo,
  className,
}: {
  intent: AccountListIntent;
  placeholder: string;
  alsoOpenable?: CaseRow[];
  bindTo?: PersonSummary | null;
  className?: string;
}) {
  const { t } = useTranslation();
  const { acct } = usePortalSearch();
  const { setAcct } = usePortalNavActions();
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query, SEARCH_DEBOUNCE_MS);
  const search = useAccountList(debounced, intent);

  // Never rows the query did not ask for: under the floor the listing is not
  // this field's answer, and kept pages would be the previous term's.
  const lists = listsAnyAccount(debounced, intent);
  const items = lists
    ? (search.data?.pages ?? []).flatMap((page) => page.items)
    : [];
  const loading = search.isFetching && !search.isFetchingNextPage;
  // A needle actually reached the service, so an empty answer means "nothing
  // matched" rather than "nothing is observed". The floor comes from the shared
  // rule, or the two disagree about what counts as one character.
  const asked = debounced.trim() !== "" && lists;
  // Reads the live field rather than the debounced one: the answer to "why is
  // nothing happening" should not wait for the debounce to expire. Only while
  // nothing is listed, though — for one debounce the rows are still the ones
  // the shorter field asked for, and a notice above them contradicts them.
  const tooShort = items.length === 0 && belowAccountFloor(query);
  // What is listed answers the term the query key carries, not the one in the
  // field, until the debounce fires AND the fetch lands. Neither emptiness below
  // is a claim about the field's own needle while that is true.
  const stale = query !== debounced || search.isPlaceholderData;
  const scroller = useRef<HTMLDivElement>(null);
  const loadMore = useAutoLoadOnScroll({
    hasNextPage: search.hasNextPage,
    isFetchingNextPage: search.isFetchingNextPage,
    fetchNextPage: () => void search.fetchNextPage(),
    root: scroller,
  });

  // The window takes queue-shaped rows; a listed account adapts. This is also
  // the voucher that the account exists — without it an unbound, never-decided
  // account the list just showed would open as a stale link with no verbs.
  //
  // No candidates: this list answers "whose is it", not "whose could it be".
  // The holder travels as the holder, so the window names them without the
  // window mistaking them for somebody to bind the account to.
  const asCases: CaseRow[] = items.map((m) => ({
    kind: KIND_SEARCH_MATCH,
    source: m.source,
    source_id: m.source_id,
    account_id: m.account_id,
    email: m.email,
    username: m.username,
    display_name: m.display_name,
    candidates: [],
    holder: m.person ?? null,
  }));
  // Matches first: they are what the reader just went looking for, and the
  // prev/next footer walks this order.
  //
  // INVARIANT: one entry per account. A match can also be a row the surface
  // already lists — searching inside a person finds the accounts they hold — and
  // the footer walks `ordered` by index, so a key listed twice sends `next` back
  // to the first copy instead of onward.
  const openable = uniqueByAccount([...asCases, ...alsoOpenable]);
  const ordered = openable.map((item) => accountKey(item));

  return (
    <div className={cn("flex min-h-0 min-w-0 flex-1 flex-col gap-4", className)}>
      <div className="relative shrink-0">
        <Search className="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={placeholder}
          aria-label={placeholder}
          className="ps-9"
        />
        {loading ? (
          <Spinner className="absolute end-3 top-1/2 size-4 -translate-y-1/2" />
        ) : null}
      </div>

      {tooShort ? (
        <p className="text-sm text-muted-foreground">
          {t("identities.accounts.min_chars", { min: MIN_SEARCH_CHARS })}
        </p>
      ) : null}

      {search.isError ? (
        <ComingSoon
          variant="card"
          state="error"
          label={t("identities.accounts.failed")}
        />
      ) : null}

      {/* Two different emptinesses: terms that matched nothing, and nothing to
          list. The second one does not claim the tenant is empty — the service
          answers an empty list for a fold it cannot read yet, and an operator
          cannot tell that from a tenant nobody has connected. A field that
          lists nothing until asked is a third thing: it is simply waiting, and
          says so by showing nothing at all. */}
      {!loading &&
      !tooShort &&
      !stale &&
      items.length === 0 &&
      !search.isError &&
      (asked || intent === "browse") ? (
        <Empty className="rounded-lg border">
          <EmptyHeader>
            {asked ? null : (
              <EmptyMedia variant="icon">
                <ScanSearch />
              </EmptyMedia>
            )}
            <EmptyTitle>
              {t(
                asked
                  ? "identities.accounts.no_matches"
                  : "identities.accounts.none_observed",
              )}
            </EmptyTitle>
            <EmptyDescription>
              {t(
                asked
                  ? "identities.accounts.no_matches_description"
                  : "identities.accounts.none_observed_description",
              )}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : null}

      {/* Rendered for an unread next page even with nothing listed: the marker
          asks for that page, and an observer only reports a target inside its
          own root. */}
      {items.length > 0 || (lists && search.hasNextPage) ? (
        <Card className="relative min-h-0 flex-1 overflow-hidden py-0">
          <CardContent
            ref={scroller}
            className="flex flex-col gap-1 overflow-y-auto p-2"
          >
            {items.map((item) => (
              <AccountRow
                key={accountKey(item)}
                item={item}
                selected={accountKey(item) === acct}
                onOpen={() => setAcct(accountKey(item))}
              />
            ))}
            {/* The page after this one is asked for when this marker nears the
                viewport, so the list continues instead of ending in a button.
                The marker is always there while a page is unread — the observer
                needs an element — but it only SAYS anything while that page is
                on its way: labelling an idle list "loading" is a lie the reader
                cannot dismiss. */}
            {search.hasNextPage ? (
              <div
                ref={loadMore}
                aria-live="polite"
                className="p-2 text-sm text-muted-foreground"
              >
                {search.isFetchingNextPage
                  ? t("identities.accounts.loading_more")
                  : null}
              </div>
            ) : null}
          </CardContent>
          <ScrollToEnds scroller={scroller} rows={items.length} />
        </Card>
      ) : null}

      <CaseDialog
        acct={acct}
        items={openable}
        ordered={ordered}
        bindTo={bindTo}
        onSelect={setAcct}
        onClose={() => setAcct(null)}
      />
    </div>
  );
}

/** The first row for each account wins — the earlier one carries the match. */
function uniqueByAccount(rows: CaseRow[]): CaseRow[] {
  const seen = new Set<string>();
  return rows.filter((row) => {
    const key = accountKey(row);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function AccountRow({
  item,
  selected,
  onOpen,
}: {
  item: AccountMatch;
  selected: boolean;
  onOpen: () => void;
}) {
  const { t } = useTranslation();
  const label =
    item.email?.trim() ||
    item.username?.trim() ||
    item.display_name?.trim() ||
    item.account_id;
  return (
    // Fixed columns, not content-sized ones: a list is read down a column, and
    // holders whose names differ in length would otherwise step the cards and
    // the verbs sideways on every row. Only where there is room for all four —
    // the trailing tracks are ~29rem, and taking them from a narrower window
    // would leave the address, the one value this mode answers with, an
    // ellipsis. Below that the row stacks instead.
    <div
      className={cn(
        "grid grid-cols-1 items-center gap-2 rounded-md border p-3",
        "lg:grid-cols-[minmax(0,1fr)_minmax(0,18rem)_minmax(0,11rem)_auto]",
        selected ? "border-ring bg-muted" : "border-transparent",
      )}
    >
      <div className="min-w-0">
        <div className="truncate text-sm font-medium select-text">{label}</div>
        <div className="truncate font-mono text-xs text-muted-foreground select-text">
          {item.source} · {item.account_id}
        </div>
      </div>
      {/* Whose it is — the answer the mode exists for. Unbound is an answer
          too, and exclusion is a third one: an operator's recorded decision,
          which "bound to nobody" would invite undoing. */}
      {item.person ? (
        <PersonCell person={item.person} />
      ) : item.excluded ? (
        <Badge variant="secondary" className="justify-self-start font-normal">
          {t("identities.accounts.excluded")}
        </Badge>
      ) : (
        <span className="text-xs text-muted-foreground">
          {t("identities.accounts.unbound")}
        </span>
      )}
      <Badge
        variant={item.bound_by_operator ? "secondary" : "outline"}
        className="justify-self-start font-normal"
      >
        {item.bound_by_operator
          ? t("identities.person_accounts.by_operator")
          : t("identities.person_accounts.by_automation")}
      </Badge>
      <Button type="button" size="xs" variant="outline" onClick={onOpen}>
        {t("identities.person_accounts.open")}
      </Button>
    </div>
  );
}

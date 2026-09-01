/**
 * Finding an account, and the window it is decided in.
 *
 * The mode arrives with a value in hand — a git login from a review, an address
 * from a ticket — the question neither other mode can answer, because both are
 * entered through a person. A blank field lists what the connectors reported,
 * since the accounts nobody asks about are exactly the ones nobody finds by
 * searching.
 *
 * A row opens the account, on the click itself: the queue's rows behave that
 * way, and a listing one tab across that demanded a button instead made the
 * same gesture mean two things.
 */
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ScanSearch, Search } from "lucide-react";

import type { AccountMatch } from "@/api/identity-client";
import { CaseDialog, type CaseRow } from "@/components/portal/case-dialog";
import { PersonCell } from "@/components/portal/person-cell";
import { Badge } from "@/components/ui/badge";
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
import { personDisplayName } from "@/lib/identities/person-display";
import { activatesRow, activatesRowByKey } from "@/lib/identities/row-activation";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { usePortalSearch } from "@/lib/portal/portal-search";
import {
  belowAccountFloor,
  listsAnyAccount,
  MAX_SEARCH_CHARS,
  MIN_SEARCH_CHARS,
  SEARCH_DEBOUNCE_MS,
  useAccountList,
} from "@/queries/identity-resolution";
import { cn } from "@/lib/utils";

/**
 * The field, its results, and the one case window they open into.
 *
 * INVARIANT: the window lives here and nowhere else on the surface. It is
 * opened by `?acct=` alone, so a second one on the same screen would open
 * beside this one on the same link.
 */
export function AccountSearchView() {
  const { t } = useTranslation();
  const { acct } = usePortalSearch();
  const { setAcct } = usePortalNavActions();
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query, SEARCH_DEBOUNCE_MS);
  const search = useAccountList(debounced, "browse");

  // Never rows the query did not ask for: under the floor the listing is not
  // this field's answer, and kept pages would be the previous term's.
  const lists = listsAnyAccount(debounced, "browse");
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
  const openable: CaseRow[] = items.map((m) => ({
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

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-4">
      <div className="relative shrink-0">
        <Search className="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("identities.accounts.placeholder")}
          aria-label={t("identities.accounts.placeholder")}
          // The service refuses a longer needle. Without the stop the field
          // accepts a paste it cannot search — an id, a url, a log line — and
          // the operator gets a refusal instead of a result.
          maxLength={MAX_SEARCH_CHARS}
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
          cannot tell that from a tenant nobody has connected. */}
      {!loading &&
      !tooShort &&
      !stale &&
      items.length === 0 &&
      !search.isError ? (
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
        onClose={() => setAcct(null)}
      />
    </div>
  );
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
    // holders whose names differ in length would otherwise step the cards
    // sideways on every row. Only where there is room — taking the trailing
    // tracks from a narrower window would leave the address, the one value this
    // mode answers with, an ellipsis. Below that the row stacks instead.
    <div
      role="button"
      tabIndex={0}
      onClick={(event) => {
        if (activatesRow(event)) onOpen();
      }}
      onKeyDown={(event) => {
        if (!activatesRowByKey(event)) return;
        event.preventDefault();
        onOpen();
      }}
      aria-pressed={selected}
      // Without a label the accessible name is computed from everything inside,
      // which reads out the holder's whole card before the account is named.
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
      ]
        .filter(Boolean)
        .join(", ")}
      className={cn(
        "grid cursor-pointer grid-cols-1 items-center gap-2 rounded-md border p-3 text-start select-text",
        "lg:grid-cols-[minmax(0,1fr)_minmax(0,18rem)_minmax(0,11rem)]",
        selected ? "border-ring bg-muted" : "border-transparent hover:bg-muted/60",
      )}
    >
      <div className="min-w-0">
        <div className="truncate text-sm font-medium">{label}</div>
        <div className="truncate font-mono text-xs text-muted-foreground">
          {item.source} · {item.account_id}
        </div>
      </div>
      {/* Whose it is — the answer the mode exists for. Unbound is an answer
          too, and exclusion is a third one: an operator's recorded decision,
          which "bound to nobody" would invite undoing. */}
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
      {/* Who decided the binding, as a line: the two answers are two states
          of one fact, and only one of them used to carry a shape. */}
      <Badge variant="outline" className="justify-self-start font-normal">
        {item.bound_by_operator
          ? t("identities.person_accounts.by_operator")
          : t("identities.person_accounts.by_automation")}
      </Badge>
    </div>
  );
}

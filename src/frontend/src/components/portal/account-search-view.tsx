/**
 * The console's third mode: an account in hand, and whose it is.
 *
 * The queue arrives from a problem and the person mode from a name. This one
 * answers the question an operator gets handed instead — a git login from a
 * review, an address from a ticket — which neither of the others can, because
 * both are entered through a person.
 *
 * With nothing typed it lists what the connectors reported, a page at a time:
 * the accounts nobody has asked about are exactly the ones an operator never
 * finds by searching for them.
 */
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ScanSearch, Search } from "lucide-react";

import type { AccountMatch, AttentionItem } from "@/api/identity-client";
import { CaseDialog } from "@/components/portal/case-dialog";
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
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { usePortalSearch } from "@/lib/portal/portal-search";
import {
  belowAccountFloor,
  listsAnyAccount,
  MIN_SEARCH_CHARS,
  SEARCH_DEBOUNCE_MS,
  useAccountList,
} from "@/queries/identity-resolution";
import { cn } from "@/lib/utils";

export function AccountSearchView() {
  const { t } = useTranslation();
  const { acct } = usePortalSearch();
  const { setAcct } = usePortalNavActions();
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query, SEARCH_DEBOUNCE_MS);
  const search = useAccountList(debounced);

  // Never rows the query did not ask for: under the floor the listing is not
  // this field's answer, and kept pages would be the previous term's.
  const items = listsAnyAccount(debounced)
    ? (search.data?.pages ?? []).flatMap((page) => page.items)
    : [];
  const ordered = items.map((item) => accountKey(item));
  const loading = search.isFetching && !search.isFetchingNextPage;
  // A needle actually reached the service, so an empty answer means "nothing
  // matched" rather than "nothing is observed". The floor comes from the shared
  // rule, or the two disagree about what counts as one character.
  const asked = debounced.trim() !== "" && listsAnyAccount(debounced);
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
  const asCases: AttentionItem[] = items.map((m) => ({
    kind: "match",
    source: m.source,
    source_id: m.source_id,
    account_id: m.account_id,
    email: m.email,
    username: m.username,
    display_name: m.display_name,
    candidates: m.person ? [m.person] : [],
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
      {!loading && !tooShort && !stale && items.length === 0 && !search.isError ? (
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
      {items.length > 0 || (listsAnyAccount(debounced) && search.hasNextPage) ? (
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
                viewport, so the list continues instead of ending in a button. */}
            {search.hasNextPage ? (
              <div ref={loadMore} className="p-2 text-sm text-muted-foreground">
                {t("identities.accounts.loading_more")}
              </div>
            ) : null}
          </CardContent>
          <ScrollToEnds scroller={scroller} rows={items.length} />
        </Card>
      ) : null}

      <CaseDialog
        acct={acct}
        items={asCases}
        ordered={ordered}
        onSelect={setAcct}
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

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
import { useState } from "react";
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
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { accountKey } from "@/lib/identities/account-key";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { usePortalSearch } from "@/lib/portal/portal-search";
import { useAccountList } from "@/queries/identity-resolution";
import { cn } from "@/lib/utils";

const DEBOUNCE_MS = 250;

export function AccountSearchView() {
  const { t } = useTranslation();
  const { acct } = usePortalSearch();
  const { setAcct } = usePortalNavActions();
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query, DEBOUNCE_MS);
  const search = useAccountList(debounced);

  const items = (search.data?.pages ?? []).flatMap((page) => page.items);
  const ordered = items.map((item) => accountKey(item));
  const loading = search.isFetching && !search.isFetchingNextPage;
  const asked = debounced.trim().length > 0;
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
    <div className="flex min-w-0 flex-col gap-4">
      <div className="relative">
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
      {!loading && items.length === 0 && !search.isError ? (
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

      {items.length > 0 ? (
        <Card>
          <CardContent className="flex flex-col gap-1 p-2">
            {items.map((item) => (
              <AccountRow
                key={accountKey(item)}
                item={item}
                selected={accountKey(item) === acct}
                onOpen={() => setAcct(accountKey(item))}
              />
            ))}
            {search.hasNextPage ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="m-2 self-start"
                // Not `disabled`: disabling the element that has focus blurs it,
                // and the operator's next Tab restarts from the page chrome —
                // a whole page of rows away from the button they just pressed.
                aria-busy={search.isFetchingNextPage}
                onClick={() => void search.fetchNextPage()}
              >
                {search.isFetchingNextPage
                  ? t("identities.accounts.loading_more")
                  : t("identities.accounts.load_more")}
              </Button>
            ) : null}
          </CardContent>
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

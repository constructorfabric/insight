/**
 * The console's third mode: an account in hand, and whose it is.
 *
 * The queue arrives from a problem and the person mode from a name. This one
 * answers the question an operator gets handed instead — a git login from a
 * review, an address from a ticket — which neither of the others can, because
 * both are entered through a person.
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Search } from "lucide-react";

import type { AccountMatch } from "@/api/identity-client";
import { CaseDialog } from "@/components/portal/case-dialog";
import { PersonCell } from "@/components/portal/person-cell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { useDebouncedValue } from "@/hooks/use-debounced-value";
import { accountKey } from "@/lib/identities/account-key";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { usePortalSearch } from "@/lib/portal/portal-search";
import { useAccountSearch } from "@/queries/identity-resolution";
import { cn } from "@/lib/utils";

const DEBOUNCE_MS = 250;
/** The service's own floor: a shorter needle scans the fold to answer with everything. */
const MIN_QUERY_CHARS = 3;

export function AccountSearchView() {
  const { t } = useTranslation();
  const { acct } = usePortalSearch();
  const { setAcct } = usePortalNavActions();
  const [query, setQuery] = useState("");
  const debounced = useDebouncedValue(query, DEBOUNCE_MS);
  const search = useAccountSearch(debounced);

  const items = search.data?.items ?? [];
  const ordered = items.map((item) => accountKey(item));
  const asked = debounced.trim().length >= MIN_QUERY_CHARS;

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
        {search.isFetching ? (
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

      {asked && !search.isFetching && items.length === 0 && !search.isError ? (
        <p className="p-3 text-sm text-muted-foreground">
          {t("identities.accounts.no_matches")}
        </p>
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
            {search.data?.truncated ? (
              <p className="p-2 text-xs text-muted-foreground">
                {t("identities.accounts.truncated")}
              </p>
            ) : null}
          </CardContent>
        </Card>
      ) : null}

      <CaseDialog
        acct={acct}
        items={[]}
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
    <div
      className={cn(
        "flex flex-wrap items-center gap-2 rounded-md border p-3",
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
          too, and a different one from "nobody has decided yet". */}
      {item.person ? (
        <PersonCell person={item.person} className="ms-auto max-w-xs" />
      ) : (
        <span className="ms-auto text-xs text-muted-foreground">
          {t("identities.accounts.unbound")}
        </span>
      )}
      <Badge
        variant={item.bound_by_operator ? "secondary" : "outline"}
        className="shrink-0 font-normal"
      >
        {item.bound_by_operator
          ? t("identities.people.by_operator")
          : t("identities.people.by_automation")}
      </Badge>
      <Button type="button" size="xs" variant="outline" onClick={onOpen}>
        {t("identities.people.open")}
      </Button>
    </div>
  );
}

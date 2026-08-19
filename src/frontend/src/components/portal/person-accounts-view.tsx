/**
 * The console's second mode: one person and every account bound to them.
 *
 * The review queue only ever shows what automation could not decide, so a
 * settled binding is invisible — and the questions that actually arrive are
 * about settled bindings ("this person's work looks split", "that account is
 * not theirs"). The verbs already accept those accounts; this is the door to
 * them.
 *
 * Entered through a person rather than through a list of every binding: the
 * roster is browsable here, but a decision still starts from whose accounts
 * these are, so it stays attached to the question that prompted it.
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";

import type {
  PersonAccountEntry,
  PersonSummary,
} from "@/api/identity-client";
import { PersonCell } from "@/components/portal/person-cell";
import { AccountFinder } from "@/components/portal/account-search-view";
import type { CaseRow } from "@/components/portal/case-dialog";
import { PersonPicker } from "@/components/portal/person-picker";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { accountKey } from "@/lib/identities/account-key";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { usePortalSearch, useSetPortalSearch } from "@/lib/portal/portal-search";
import { usePersonAccounts } from "@/queries/identity-resolution";
import { cn } from "@/lib/utils";

export function PersonAccountsView() {
  const { t } = useTranslation();
  const { person } = usePortalSearch();
  const setSearch = useSetPortalSearch();
  // The URL owns which person is open; this remembers the card the operator
  // picked, so the heading names them. Arriving by link there is no card —
  // search resolves values, not ids — and the id stands alone, honestly.
  const [picked, setPicked] = useState<PersonSummary | null>(null);

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-4">
      {/* Choosing a person replaces the roster with their accounts AND clears
          the terms that found them, so the way back is not on screen anywhere:
          without this, leaving a person means editing the URL. */}
      {person ? (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="-ms-2 self-start"
          onClick={() => setSearch({ person: undefined, acct: undefined })}
        >
          {t("identities.person_accounts.back")}
        </Button>
      ) : null}
      {/* Two different questions, so two different fields — never both. With
          nobody chosen the roster IS this mode, and it takes the surface: the
          card and the full height the other two modes' listings have. Inside a
          person, searching for people again is asking the reader to find
          somebody they already have open; what they need there is an ACCOUNT,
          to bind it to them. */}
      {person ? (
        <PersonAccounts
          personId={person}
          card={picked?.person_id === person ? picked : null}
        />
      ) : (
        <PersonPicker
          browseWhenEmpty
          asSurface
          onPick={(next: PersonSummary) => {
            setPicked(next);
            setSearch({ person: next.person_id, acct: undefined });
          }}
        />
      )}
    </div>
  );
}

function PersonAccounts({
  personId,
  card,
}: {
  personId: string;
  card: PersonSummary | null;
}) {
  const { t } = useTranslation();
  const { acct } = usePortalSearch();
  const { setAcct } = usePortalNavActions();
  const accounts = usePersonAccounts(personId);

  if (accounts.isLoading) return <CenteredSpinner className="min-h-40" />;
  if (accounts.isError || !accounts.data) {
    return (
      <ComingSoon
        variant="card"
        state="error"
        label={t("identities.person_accounts.load_failed")}
        onRetry={() => void accounts.refetch()}
      />
    );
  }

  const entries = accounts.data.accounts;
  const person = card ?? { person_id: personId };
  // Queue-shaped rows for the window: the voucher that each account exists
  // (they are read from the person's own bindings), plus the person as the
  // HOLDER so the current binding renders as a card rather than a bare id.
  // Never as a candidate: every account here is already theirs, and a candidate
  // list saying so invites re-asserting the binding — the confirm act, which the
  // queue lists the accounts for.
  const asCases: CaseRow[] = entries.map((entry) => ({
    kind: "member",
    source: entry.source,
    source_id: entry.source_id,
    account_id: entry.account_id,
    email: entry.email,
    username: entry.username,
    candidates: [],
    holder: person,
  }));

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-4">
      {/* The field here searches ACCOUNTS, and it owns the one case window on
          this surface — so the rows below open in it too, rather than in a
          second window `?acct=` would open beside the first. */}
      <AccountFinder
        intent="match"
        placeholder={t("identities.person_accounts.find_account")}
        alsoOpenable={asCases}
        bindTo={person}
        className="flex-none"
      />
      {/* Shrinkable, not fixed: with more accounts than fit, a card that refuses
          to shrink runs off the bottom of the mode and takes the field and the
          tabs above it out of reach. Sized to its rows while they fit. */}
      <Card className="min-h-0 overflow-hidden">
        <CardHeader>
          <CardTitle className="flex flex-wrap items-center gap-2 text-sm">
            {t("identities.person_accounts.accounts")}
            <Badge variant="secondary">{entries.length}</Badge>
            <PersonCell person={person} className="ms-auto" />
          </CardTitle>
        </CardHeader>
        <CardContent className="flex min-h-0 flex-col gap-1 overflow-y-auto p-2 pt-0">
          {entries.length === 0 ? (
            <p className="p-3 text-sm text-muted-foreground">
              {t("identities.person_accounts.no_accounts")}
            </p>
          ) : (
            entries.map((entry) => (
              <AccountRow
                key={accountKey(entry)}
                entry={entry}
                selected={accountKey(entry) === acct}
                onOpen={() => setAcct(accountKey(entry))}
              />
            ))
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function AccountRow({
  entry,
  selected,
  onOpen,
}: {
  entry: PersonAccountEntry;
  selected: boolean;
  onOpen: () => void;
}) {
  const { t } = useTranslation();
  const label = entry.email?.trim() || entry.username?.trim() || entry.account_id;
  return (
    // Same fixed columns, and the same width to earn them, as the account
    // listing: the two lists sit one tab apart and reading either one down a
    // column should feel the same.
    <div
      className={cn(
        "grid grid-cols-1 items-center gap-2 rounded-md border p-3",
        "lg:grid-cols-[minmax(0,1fr)_minmax(0,11rem)_auto]",
        selected ? "border-ring bg-muted" : "border-transparent",
      )}
    >
      <div className="min-w-0">
        <div className="truncate text-sm font-medium select-text">{label}</div>
        <div className="truncate font-mono text-xs text-muted-foreground select-text">
          {entry.source} · {entry.account_id}
        </div>
      </div>
      {/* Who decided this binding is the first thing to know before changing
          it: undoing automation is routine, overruling a colleague is not. */}
      <Badge
        variant={entry.bound_by_operator ? "secondary" : "outline"}
        className="justify-self-start font-normal"
      >
        {entry.bound_by_operator
          ? t("identities.person_accounts.by_operator")
          : t("identities.person_accounts.by_automation")}
      </Badge>
      <Button type="button" size="xs" variant="outline" onClick={onOpen}>
        {t("identities.person_accounts.open")}
      </Button>
    </div>
  );
}

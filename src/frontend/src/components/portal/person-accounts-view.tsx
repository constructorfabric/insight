/**
 * The console's second mode: one person and every account bound to them.
 *
 * The review queue only ever shows what automation could not decide, so a
 * settled binding is invisible — and the questions that actually arrive are
 * about settled bindings ("this person's work looks split", "that account is
 * not theirs"). The verbs already accept those accounts; this is the door to
 * them.
 *
 * Deliberately entered through a person rather than a list of every binding:
 * an operator arrives holding a name, and a decision reached that way stays
 * attached to the question that prompted it.
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { UserSearch } from "lucide-react";

import type {
  AttentionItem,
  PersonAccountEntry,
  PersonSummary,
} from "@/api/identity-client";
import { CaseDialog } from "@/components/portal/case-dialog";
import { PersonCell } from "@/components/portal/person-cell";
import { PersonPicker } from "@/components/portal/person-picker";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
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
    <div className="flex min-w-0 flex-col gap-4">
      <PersonPicker
        // Remounted per person so choosing one clears the terms that found
        // them: the results are a way in, not a view worth keeping open.
        key={person ?? "none"}
        onPick={(next: PersonSummary) => {
          setPicked(next);
          setSearch({ person: next.person_id, acct: undefined });
        }}
      />
      {person ? (
        <PersonAccounts
          personId={person}
          card={picked?.person_id === person ? picked : null}
        />
      ) : (
        <Empty className="rounded-lg border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <UserSearch />
            </EmptyMedia>
            <EmptyTitle>{t("identities.people.no_person")}</EmptyTitle>
            <EmptyDescription>
              {t("identities.people.no_person_description")}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
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
        label={t("identities.people.load_failed")}
        onRetry={() => void accounts.refetch()}
      />
    );
  }

  const entries = accounts.data.accounts;
  const ordered = entries.map((entry) => accountKey(entry));
  // Queue-shaped rows for the window: the voucher that each account exists
  // (they are read from the person's own bindings), plus the person as the
  // one candidate so the current binding renders as a card, not a bare id.
  const asCases: AttentionItem[] = entries.map((entry) => ({
    kind: "member",
    source: entry.source,
    source_id: entry.source_id,
    account_id: entry.account_id,
    email: entry.email,
    username: entry.username,
    candidates: [card ?? { person_id: personId }],
  }));

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2 text-sm">
          {t("identities.people.accounts")}
          <Badge variant="secondary">{entries.length}</Badge>
          <PersonCell person={card ?? { person_id: personId }} className="ms-auto" />
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-1 p-2 pt-0">
        {entries.length === 0 ? (
          <p className="p-3 text-sm text-muted-foreground">
            {t("identities.people.no_accounts")}
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
      <CaseDialog
        acct={acct}
        items={asCases}
        ordered={ordered}
        onSelect={setAcct}
        onClose={() => setAcct(null)}
      />
    </Card>
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
    <div
      className={cn(
        "flex items-center gap-2 rounded-md border p-3",
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
        className="ms-auto shrink-0 font-normal"
      >
        {entry.bound_by_operator
          ? t("identities.people.by_operator")
          : t("identities.people.by_automation")}
      </Badge>
      <Button type="button" size="xs" variant="outline" onClick={onOpen}>
        {t("identities.people.open")}
      </Button>
    </div>
  );
}

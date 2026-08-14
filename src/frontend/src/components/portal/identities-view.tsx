/**
 * The identity-resolution operator console (Manage → Identities), phase 1:
 * the review queue, read-only.
 *
 * A triage surface, not a roster: the operator lands in what NEEDS a
 * decision, grouped by why it does, and works the backlog to zero — the
 * empty queue is the goal state and renders as one. The tenant-wide rates
 * strip on top is honest about scale: it counts every observed account,
 * never just the visible page.
 *
 * Selection lives in `?acct=` so an operator can hand a colleague a link to
 * the exact account under discussion — and that link answers whatever the
 * queue looks like by then, an emptied backlog included.
 */
import { useTranslation } from "react-i18next";

import type { AttentionItem, ResolutionRates } from "@/api/identity-client";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { PersonCell } from "@/components/portal/person-cell";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { usePortalSearch } from "@/lib/portal/portal-search";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { itemKey, parseAccountKey } from "@/lib/identities/account-key";
import { AccountDetail } from "@/components/portal/account-detail";
import { useAttention } from "@/queries/identity-resolution";
import { TEXT_FIGURE, TEXT_LABEL } from "@/lib/type-scale";
import { STATUS_SURFACE_CLASS, type Status } from "@/lib/status";
import { cn } from "@/lib/utils";
import { PartyPopper, TriangleAlert, UserSearch } from "lucide-react";

/** Queue groups in working order: conflicts first, then the unknowns. */
const KIND_ORDER = ["contested", "binding_conflict", "no_evidence"] as const;

const RATE_TILES: ReadonlyArray<{ key: keyof ResolutionRates; status: Status }> = [
  { key: "observed", status: "neutral" },
  { key: "bound", status: "good" },
  { key: "pending", status: "warn" },
  { key: "no_evidence", status: "bad" },
  { key: "excluded", status: "neutral" },
];

export function IdentitiesView() {
  const { t } = useTranslation();
  const attention = useAttention();

  if (attention.isLoading) return <CenteredSpinner className="min-h-[60vh]" />;
  if (attention.isError || !attention.data) {
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          label={t("identities.queue.load_failed")}
          onRetry={() => void attention.refetch()}
        />
      </div>
    );
  }

  const { items, rates, truncated, items_truncated: itemsTruncated } =
    attention.data;
  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 p-6">
      <header>
        <h1 className="text-lg font-semibold tracking-tight">
          {t("identities.title")}
        </h1>
        <p className="text-sm text-muted-foreground">
          {t("identities.subtitle")}
        </p>
      </header>
      {truncated ? (
        <Alert variant="destructive" role="status">
          <TriangleAlert />
          <AlertDescription>{t("identities.queue.truncated")}</AlertDescription>
        </Alert>
      ) : null}
      {/* The list was cut by the server's item cap while the rates stay
          whole-tenant — a different fact from `truncated`, and one an
          operator working to zero has to know. */}
      {itemsTruncated && !truncated ? (
        <Alert role="status">
          <TriangleAlert />
          <AlertDescription>
            {t("identities.queue.items_truncated")}
          </AlertDescription>
        </Alert>
      ) : null}
      <RatesStrip rates={rates} />
      <Queue items={items} />
    </div>
  );
}

function RatesStrip({ rates }: { rates: ResolutionRates }) {
  const { t } = useTranslation();
  return (
    <div className="grid grid-cols-[repeat(auto-fit,minmax(9rem,1fr))] gap-3">
      {RATE_TILES.map(({ key, status }) => (
        <div key={key} className="rounded-lg border bg-card p-4">
          <div className={TEXT_FIGURE}>{rates[key]}</div>
          <span
            className={cn(
              TEXT_LABEL,
              "mt-1 inline-block rounded px-1.5 py-0.5",
              STATUS_SURFACE_CLASS[status],
            )}
          >
            {t(`identities.rates.${key}`)}
          </span>
        </div>
      ))}
    </div>
  );
}

/** The goal state, celebrated rather than rendered as a blank table. */
function AllResolved() {
  const { t } = useTranslation();
  return (
    <Empty>
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <PartyPopper />
        </EmptyMedia>
        <EmptyTitle>{t("identities.queue.empty_title")}</EmptyTitle>
        <EmptyDescription>
          {t("identities.queue.empty_description")}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}

function Queue({ items }: { items: AttentionItem[] }) {
  const { acct } = usePortalSearch();
  const { setAcct } = usePortalNavActions();
  const groups: Array<{ kind: string; items: AttentionItem[] }> = KIND_ORDER.map(
    (kind) => ({ kind, items: items.filter((i) => i.kind === kind) }),
  ).filter((g) => g.items.length > 0);
  // An unknown kind from a newer backend still shows up rather than
  // vanishing — the vocabulary is open by contract.
  const known = new Set<string>(KIND_ORDER);
  const other = items.filter((i) => !known.has(i.kind));
  if (other.length > 0) groups.push({ kind: "other", items: other });

  // The worked-to-zero queue is the goal state — but a shared `?acct=` link
  // has to answer even then, and the backlog reaching zero is exactly when a
  // colleague opens the link they were sent. So the celebration replaces the
  // GROUP LIST, never the grid the detail panel lives in.
  if (items.length === 0 && !acct) return <AllResolved />;

  return (
    <div className="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1fr)_20rem]">
      <div className="flex min-w-0 flex-col gap-4">
        {items.length === 0 ? (
          <AllResolved />
        ) : (
          groups.map((group) => (
            <QueueGroup
              key={group.kind}
              kind={group.kind}
              items={group.items}
              selectedKey={acct}
              onSelect={(key) => setAcct(key === acct ? null : key)}
            />
          ))
        )}
      </div>
      <DetailPane acct={acct} items={items} />
    </div>
  );
}

function DetailPane({
  acct,
  items,
}: {
  acct: string | undefined;
  items: AttentionItem[];
}) {
  const ref = parseAccountKey(acct);
  if (!ref) return <DetailPlaceholder />;
  const queueItem = items.find((i) => itemKey(i) === acct);
  // Keyed by the account: the panel holds per-account state (a verb's outcome
  // alert, an open dialog), and a cached binding renders the next selection
  // synchronously — an unkeyed panel would carry that state across accounts.
  return <AccountDetail key={acct} accountRef={ref} queueItem={queueItem} />;
}

function QueueGroup({
  kind,
  items,
  selectedKey,
  onSelect,
}: {
  kind: string;
  items: AttentionItem[];
  selectedKey: string | undefined;
  onSelect: (key: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-sm">
          {t(`identities.kind.${kind}`, { defaultValue: kind })}
          <Badge variant="secondary">{items.length}</Badge>
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-1 p-2 pt-0">
        {items.map((item) => {
          const key = itemKey(item);
          const selected = key === selectedKey;
          return (
            <button
              key={key}
              type="button"
              onClick={() => onSelect(key)}
              aria-pressed={selected}
              className={cn(
                "rounded-md border p-3 text-start",
                selected
                  ? "border-ring bg-muted"
                  : "border-transparent hover:bg-muted/60",
              )}
            >
              <div className="flex items-baseline gap-2">
                <span className="truncate text-sm font-medium">
                  {item.email?.trim() || item.username?.trim() || item.account_id}
                </span>
                <span className="ms-auto shrink-0 font-mono text-xs text-muted-foreground">
                  {item.source}
                </span>
              </div>
              {item.candidates.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-x-6 gap-y-2">
                  {item.candidates.map((candidate) => (
                    <PersonCell key={candidate.person_id} person={candidate} />
                  ))}
                </div>
              ) : null}
            </button>
          );
        })}
      </CardContent>
    </Card>
  );
}

function DetailPlaceholder() {
  const { t } = useTranslation();
  return (
    <Empty className="h-fit rounded-lg border lg:sticky lg:top-4">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <UserSearch />
        </EmptyMedia>
        <EmptyTitle>{t("identities.detail.no_selection")}</EmptyTitle>
        <EmptyDescription>
          {t("identities.detail.no_selection_description")}
        </EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
}

/**
 * One account under review — the body of the case window: the decision surface
 * (who holds it, who else could, and every verb), then every decision ever
 * recorded. The journal is append-only, so that trail is complete by
 * construction.
 *
 * The holder and the candidates are rendered by `AccountActions`, which owns the
 * verbs that act on them — naming the holder here as well made the window list
 * one person twice.
 *
 * It answers for an account no longer in the queue, which is what a shared
 * link lands on. The binding read never 404s: an account nobody ever observed
 * or decided answers 200 with an empty journal, so "not in the queue, no
 * binding, no history" is the stale-link state — it says so instead of
 * offering verbs whose bind would pre-register a typo as a real account.
 * The trail keeps two records in one order: a binding row says what changed,
 * an operator call says who ran it and why. They stay separate rows — one call
 * can move a dozen accounts, and folding it into this account's row would
 * invent a link the journal does not record.
 */
import { useTranslation } from "react-i18next";

import type {
  AccountOperation,
  AttentionItem,
  BindingHistoryEntry,
  PersonSummary,
} from "@/api/identity-client";
import { AccountActions } from "@/components/portal/account-actions";
import { PersonId } from "@/components/portal/person-id";
import { Badge } from "@/components/ui/badge";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import type { AccountRef } from "@/lib/identities/account-key";
import { isQueueItem } from "@/lib/identities/cases";
import { personDisplayName } from "@/lib/identities/person-display";
import { formatUtcAge, formatUtcInstant } from "@/lib/format";
import { useAccountBinding } from "@/queries/identity-resolution";

/** Known reason codes → i18n keys; anything else renders as-is (open vocabulary). */
const VERB_KEYS: Record<string, string> = {
  "operator-bind": "identities.history.bind",
  "operator-merge": "identities.history.merge",
  "operator-detach": "identities.history.detach",
  "operator-exclude": "identities.history.exclude",
  "login-bootstrap": "identities.history.login_bootstrap",
  "roster-mint": "identities.history.roster_mint",
};

export function AccountDetail({
  accountRef,
  queueItem,
  observed = false,
  holder,
  bindTo,
  onDecided,
}: {
  accountRef: AccountRef;
  /** The queue row for this account, when it is still in the queue — the
   *  source of hydrated candidate cards and observed evidence. */
  queueItem: AttentionItem | undefined;
  /** The caller vouches the account exists (a queue row, a search hit, a
   *  person's account list). Without a voucher, an account with no binding
   *  and no history reads as a stale link — offering verbs there would let a
   *  mistyped `?acct=` pre-register a typo as a real account. */
  observed?: boolean;
  /**
   * Whoever holds the account, for the surfaces that know it without having any
   * candidates — the binding read answers with an id and no card, so without
   * this the section below could only name the holder by finding them among the
   * queue's candidates.
   */
  holder?: PersonSummary | null;
  /** Bind straight to the person the surface has open. See `AccountActions`. */
  bindTo?: PersonSummary | null;
  /** A verb decided every account it named. See `AccountActions`. */
  onDecided?: () => void;
}) {
  const { t } = useTranslation();
  const binding = useAccountBinding(accountRef);

  if (binding.isLoading) return <CenteredSpinner className="min-h-40" />;
  if (binding.isError) {
    return (
      <ComingSoon
        variant="card"
        state="error"
        label={t("identities.detail.load_failed")}
        onRetry={() => void binding.refetch()}
      />
    );
  }
  if (!binding.data) return null;

  const neverSeen =
    !observed &&
    queueItem == null &&
    binding.data.person_id == null &&
    binding.data.history.length === 0;
  if (neverSeen) {
    return (
      <ComingSoon
        variant="card"
        state="empty"
        label={t("identities.detail.not_found")}
      />
    );
  }

  const candidates = queueItem?.candidates ?? [];
  // The holder the surface passed is a card for the id it read; the binding read
  // is who holds the account NOW. Naming that card for a different id would
  // caption a fresh binding — a detach mints a person the surface never saw —
  // with the person it was just moved away from.
  const boundCard =
    holder?.person_id === binding.data.person_id
      ? holder
      : candidates.find((c) => c.person_id === binding.data.person_id);

  return (
    // One column, not two: the people are what an operator reads across, and
    // splitting the window halved the width of the addresses and ids that
    // tell two namesakes apart. The decision sits above the trail behind it,
    // and only the trail scrolls — the verbs stay where they were.
    <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
      <div className="shrink-0">
        <AccountActions
          accountRef={accountRef}
          binding={binding.data}
          candidates={candidates}
          holder={boundCard ?? null}
          // The accounts and persons modes reuse this window for settled
          // accounts, and their rows carry a kind of the console's own making.
          queued={queueItem != null && isQueueItem(queueItem.kind)}
          bindTo={bindTo}
          onDecided={onDecided}
        />
      </div>
      <section className="flex min-h-0 flex-1 flex-col">
        <SectionLabel>{t("identities.detail.history")}</SectionLabel>
        {binding.data.history.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("identities.detail.no_history")}
          </p>
        ) : (
          // A floor under the trail: with a tall decision block above it the
          // remaining space collapses to a sliver, and a one-line history is
          // unreadable. Below the floor the window itself scrolls.
          <ol className="flex min-h-48 flex-1 flex-col gap-2 overflow-y-auto pe-1">
            {trail(binding.data.history, binding.data.operations).map((row) =>
              row.kind === "decision" ? (
                <HistoryRow
                  key={row.key}
                  entry={row.entry}
                  known={boundCard ? [...candidates, boundCard] : candidates}
                />
              ) : (
                <OperationRow key={row.key} operation={row.operation} />
              ),
            )}
          </ol>
        )}
      </section>
    </div>
  );
}

type TrailRow =
  | { kind: "decision"; key: string; entry: BindingHistoryEntry }
  | { kind: "call"; key: string; operation: AccountOperation };

/**
 * The two records an account's past is kept in, in one order.
 *
 * A binding row says what changed; an operator call says who ran it and why.
 * They are deliberately not merged into one another — the call may have moved
 * a dozen other accounts, and claiming otherwise would invent a link the
 * journal does not record. Shown side by side in time, the pair reads itself.
 */
function trail(
  history: BindingHistoryEntry[],
  operations: AccountOperation[] | undefined,
): TrailRow[] {
  const rows: TrailRow[] = [
    ...history.map((entry, index) => ({
      kind: "decision" as const,
      key: `d-${entry.recorded_at}-${index}`,
      entry,
    })),
    ...(operations ?? []).map((operation) => ({
      kind: "call" as const,
      key: `c-${operation.operation_id}`,
      operation,
    })),
  ];
  const at = (row: TrailRow) =>
    row.kind === "decision" ? row.entry.recorded_at : row.operation.recorded_at;
  return rows.sort((a, b) => at(b).localeCompare(at(a)));
}

function OperationRow({ operation }: { operation: AccountOperation }) {
  const { t } = useTranslation();
  const verbKey = VERB_KEYS[operation.verb];
  return (
    <li className="rounded-md border border-dashed p-2">
      <div className="flex items-center gap-2">
        <span className="text-xs font-medium">
          {verbKey ? t(verbKey) : operation.verb}
        </span>
        <Badge variant="outline" className="font-normal">
          {t("identities.history.call")}
        </Badge>
        {operation.accounts_touched > 1 ? (
          <Badge variant="outline" className="font-normal">
            {t("identities.history.accounts_touched", {
              count: operation.accounts_touched,
            })}
          </Badge>
        ) : null}
        <span className="ms-auto text-xs text-muted-foreground">
          {formatUtcInstant(operation.recorded_at, "d MMM yyyy, HH:mm")}
          <span className="opacity-70">
            {` (${formatUtcAge(operation.recorded_at)})`}
          </span>
        </span>
      </div>
      <div className="mt-1.5 flex flex-wrap items-center gap-x-1.5 text-xs text-muted-foreground">
        <span>{t("identities.history.by")}</span>
        {operation.author ? (
          <span className="font-medium text-foreground">
            {personDisplayName(operation.author)}
          </span>
        ) : null}
        <PersonId id={operation.author_person_id} />
        {operation.outcome ? (
          <span className="ms-auto font-mono">{operation.outcome}</span>
        ) : null}
      </div>
      {/* The one thing no other record holds: why a human did this. */}
      {operation.comment ? (
        <p className="mt-1.5 border-s-2 ps-2 text-xs text-foreground italic select-text">
          {operation.comment}
        </p>
      ) : null}
    </li>
  );
}

function HistoryRow({
  entry,
  known,
}: {
  entry: BindingHistoryEntry;
  /** Cards the surface already holds, to name a row the service left as an id. */
  known: PersonSummary[];
}) {
  const { t } = useTranslation();
  // The resolver stores no reason for its own rows — as an empty string, not
  // as null, so a nullish fallback leaves the badge blank on every automatic
  // entry, which is most of them.
  const reason = entry.reason?.trim() || undefined;
  const verbKey = reason ? VERB_KEYS[reason] : undefined;
  // The card the service resolved wins; whatever cards the surface already has
  // are the fallback for a backend that does not send one yet.
  const target =
    entry.person ?? known.find((c) => c.person_id === entry.person_id);
  return (
    <li className="rounded-md border p-2">
      <div className="flex items-center gap-2">
        <Badge variant={entry.by_operator ? "secondary" : "outline"}>
          {verbKey ? t(verbKey) : (reason ?? t("identities.history.automatic"))}
        </Badge>
        {/* The instant is what an operator compares between entries and pastes
            into a ticket; the age answers the question the trail is usually
            opened for — how long this has stood. Neither replaces the other. */}
        <span className="ms-auto text-xs text-muted-foreground">
          {formatUtcInstant(entry.recorded_at, "d MMM yyyy, HH:mm")}
          <span className="opacity-70">
            {` (${formatUtcAge(entry.recorded_at)})`}
          </span>
        </span>
      </div>
      <div className="mt-1.5 flex flex-wrap items-center gap-x-1.5 text-xs text-muted-foreground">
        <span>{t("identities.history.to")}</span>
        {target ? (
          <span className="font-medium text-foreground">
            {personDisplayName(target)}
          </span>
        ) : null}
        <PersonId id={entry.person_id} />
        {entry.by_operator ? (
          <span className="ms-auto">
            {entry.author
              ? t("identities.history.by_name", {
                  name: personDisplayName(entry.author),
                })
              : t("identities.history.by_operator")}
          </span>
        ) : null}
      </div>
    </li>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">
      {children}
    </div>
  );
}

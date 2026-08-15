/**
 * One account under review — the body of the case window: what the resolver
 * currently thinks (the binding), who it could belong to (the queue's
 * hydrated candidates), and every decision ever recorded (the history — the
 * journal is append-only, so this trail is complete by construction).
 *
 * It answers for an account no longer in the queue, which is what a shared
 * link lands on. The binding read never 404s: an account nobody ever observed
 * or decided answers 200 with an empty journal, so "not in the queue, no
 * binding, no history" is the stale-link state — it says so instead of
 * offering verbs whose bind would pre-register a typo as a real account.
 * Person ids in the history arrive bare (there is no id→name read for
 * arbitrary persons yet); when an id matches a hydrated candidate we show the
 * card, otherwise the id itself — honest over pretty.
 */
import { useTranslation } from "react-i18next";

import type {
  AttentionItem,
  BindingHistoryEntry,
  PersonSummary,
} from "@/api/identity-client";
import { AccountActions } from "@/components/portal/account-actions";
import { PersonCell } from "@/components/portal/person-cell";
import { Badge } from "@/components/ui/badge";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import type { AccountRef } from "@/lib/identities/account-key";
import { personDisplayName } from "@/lib/identities/person-display";
import { formatUtcInstant } from "@/lib/format";
import { useAccountBinding } from "@/queries/identity-resolution";

/** Known reason codes → i18n keys; anything else renders as-is (open vocabulary). */
const VERB_KEYS: Record<string, string> = {
  "operator-bind": "identities.history.bind",
  "operator-merge": "identities.history.merge",
  "operator-detach": "identities.history.detach",
  "operator-exclude": "identities.history.exclude",
  "login-bootstrap": "identities.history.login_bootstrap",
};

export function AccountDetail({
  accountRef,
  queueItem,
}: {
  accountRef: AccountRef;
  /** The queue row for this account, when it is still in the queue — the
   *  source of hydrated candidate cards and observed evidence. */
  queueItem: AttentionItem | undefined;
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
  const boundCard = candidates.find(
    (c) => c.person_id === binding.data.person_id,
  );

  return (
    // Decisions on the start side, the trail behind them on the end side —
    // and the trail scrolls on its own, so a long-lived account cannot push
    // the verbs out of reach.
    <div className="grid gap-6 md:grid-cols-2">
      <div className="flex min-w-0 flex-col gap-4">
        <section>
          <SectionLabel>{t("identities.detail.current_binding")}</SectionLabel>
          {binding.data.person_id ? (
            boundCard ? (
              <PersonCell person={boundCard} />
            ) : (
              <PersonId id={binding.data.person_id} />
            )
          ) : (
            <p className="text-sm text-muted-foreground">
              {t("identities.detail.unbound")}
            </p>
          )}
        </section>
        <AccountActions
          accountRef={accountRef}
          binding={binding.data}
          candidates={candidates}
        />
      </div>
      <section className="flex min-w-0 flex-col">
        <SectionLabel>{t("identities.detail.history")}</SectionLabel>
        {binding.data.history.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("identities.detail.no_history")}
          </p>
        ) : (
          <ol className="flex max-h-[50vh] flex-col gap-2 overflow-y-auto pe-1">
            {binding.data.history.map((entry, index) => (
              <HistoryRow
                key={`${entry.recorded_at}-${index}`}
                entry={entry}
                candidates={candidates}
              />
            ))}
          </ol>
        )}
      </section>
    </div>
  );
}

function HistoryRow({
  entry,
  candidates,
}: {
  entry: BindingHistoryEntry;
  candidates: PersonSummary[];
}) {
  const { t } = useTranslation();
  // The resolver stores no reason for its own rows — as an empty string, not
  // as null, so a nullish fallback leaves the badge blank on every automatic
  // entry, which is most of them.
  const reason = entry.reason?.trim() || undefined;
  const verbKey = reason ? VERB_KEYS[reason] : undefined;
  const target = candidates.find((c) => c.person_id === entry.person_id);
  return (
    <li className="rounded-md border p-2">
      <div className="flex items-center gap-2">
        <Badge variant={entry.by_operator ? "secondary" : "outline"}>
          {verbKey ? t(verbKey) : (reason ?? t("identities.history.automatic"))}
        </Badge>
        <span className="ms-auto text-xs text-muted-foreground">
          {formatUtcInstant(entry.recorded_at, "d MMM yyyy, HH:mm")}
        </span>
      </div>
      <div className="mt-1.5 flex items-center gap-1.5 text-xs text-muted-foreground">
        <span>{t("identities.history.to")}</span>
        {target ? (
          <span className="font-medium text-foreground">
            {personDisplayName(target)}
          </span>
        ) : (
          <PersonId id={entry.person_id} />
        )}
        {entry.by_operator ? (
          <span className="ms-auto">{t("identities.history.by_operator")}</span>
        ) : null}
      </div>
    </li>
  );
}

/** A bare person id, honest and copyable, when no card is known for it. */
function PersonId({ id }: { id: string }) {
  return <span className="font-mono text-xs">{id}</span>;
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">
      {children}
    </div>
  );
}


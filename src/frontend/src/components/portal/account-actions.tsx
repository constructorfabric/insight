/**
 * The decision surface for one account: every correction verb, each behind a
 * confirmation, with the server's per-item outcomes shown verbatim.
 *
 * Grammar (mirrors the API):
 * - Bind — attach THIS account to a person. On the current person it is the
 *   "confirm" act: re-asserting an automatic binding records the operator's
 *   decision and clears the queue item.
 * - Detach — mint a fresh person for this account. Offered only where it means
 *   something: taking a person's only account replaces them with an identical
 *   one and leaves a husk behind. An unheld account is the exception — there it
 *   is how an orphan gets a person.
 * - Exclude — not a person at all (bot / CI).
 *
 * The holder is rendered HERE rather than by the window around it, because the
 * verb that re-asserts their binding belongs beside them. Listing the holder a
 * second time under "candidates" just to hang Confirm off it read as two people
 * with the same name and id.
 *
 * Merge is deliberately NOT here. It is a claim about people, and this window
 * argues about one account: the button sat in the row of the person who would
 * survive while the absorbed one was named in a section above it, so nothing on
 * screen said which way round it went. It lives on the queue's case, where the
 * people are listed — see `MergeCaseDialog`.
 *
 * A verb that changed everything it named reports by toast and hands the window
 * back (`onDecided`): the surface it was taken from re-reads, and the operator
 * never acts twice on a candidate list the server has already moved past.
 * A `refused` item is different — the account kept its binding, so the window
 * stays and states the counters verbatim (#2424).
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "@/components/ui/sonner";

import type {
  AccountBinding,
  CorrectionResponse,
  PersonSummary,
  WireAccountRef,
} from "@/api/identity-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { PersonCell } from "@/components/portal/person-cell";
import { PersonId } from "@/components/portal/person-id";
import { PersonPicker } from "@/components/portal/person-picker";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { AccountRef } from "@/lib/identities/account-key";
import { fullyDecided, refusedCount } from "@/lib/identities/outcomes";
import { apiErrorReason } from "@/lib/query-console/api-error";
import {
  useBindAccount,
  useDetachAccount,
  useExcludeAccount,
  usePersonAccounts,
} from "@/queries/identity-resolution";

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">
      {children}
    </div>
  );
}

/** Long enough to read a UUID off and paste it somewhere. */
const MINTED_ID_TOAST_MS = 15_000;

/** Join the sentences a description is built from, skipping the absent ones. */
function sentences(...parts: (string | null)[]): string {
  return parts.filter(Boolean).join(" ");
}

type PendingAction =
  | { kind: "closed" }
  | { kind: "bind"; person: PersonSummary }
  | { kind: "detach" }
  | { kind: "exclude" };

export function AccountActions({
  accountRef,
  binding,
  candidates,
  holder,
  bindTo,
  queued = false,
  onDecided,
}: {
  accountRef: AccountRef;
  binding: AccountBinding;
  /** Persons the evidence says could own this account — a QUEUE question, so
   *  empty everywhere the account is already settled. */
  candidates: PersonSummary[];
  /** Whoever holds it now, when the surface knows their card. */
  holder?: PersonSummary | null;
  /**
   * The person the surface behind this window has open. Binding to them is then
   * one press, and the search for a person is gone: the reader is already
   * inside that person, so asking them to find them again is asking twice.
   */
  bindTo?: PersonSummary | null;
  /**
   * This account really is on the review queue, so a verb really does take it
   * off. The same window opens over settled accounts from the search and from a
   * person's own list, and promising them the queue would describe a queue they
   * were never in.
   */
  queued?: boolean;
  /**
   * Every account the verb named was decided. The surface hands the window back
   * — its candidate list and its binding are both a read the server has now
   * moved past, and offering them again invites the same decision twice.
   */
  onDecided?: () => void;
}) {
  const { t } = useTranslation();
  const [action, setAction] = useState<PendingAction>({ kind: "closed" });
  const [outcome, setOutcome] = useState<CorrectionResponse | null>(null);

  const bind = useBindAccount();
  const detach = useDetachAccount();
  const exclude = useExcludeAccount();
  // A verb is in flight. The confirmation over these buttons is modal, so this
  // is belt-and-braces rather than the only guard.
  const busy = bind.isPending || detach.isPending || exclude.isPending;

  const wireRef: WireAccountRef = {
    source: accountRef.source,
    source_id: accountRef.source_id,
    id: accountRef.account_id,
  };
  const boundId = binding.person_id ?? null;
  // Only when it is a card for the id the binding read answers with: the surface
  // learnt its holder from a listing, and a verb taken here can move the account
  // under it — a detach binds it to a person no listing has yet. Needed for the
  // picker, which must not offer to move the account to whoever already has it.
  const boundCard =
    holder?.person_id === boundId
      ? holder
      : candidates.find((c) => c.person_id === boundId);

  // Everyone the evidence offers EXCEPT whoever already holds it: the holder is
  // named once, above, with the confirm verb on their own card.
  const rivals = candidates.filter((c) => c.person_id !== boundId);
  // What else the holder has, which is what decides whether a detach means
  // anything. Cached per person, so on a person's own page this read is already
  // in hand and costs nothing.
  const owned = usePersonAccounts(boundId);
  // A detach mints a person and moves the account to them. Taking somebody's
  // ONLY account does that and nothing else: the account still has one holder,
  // and the person it left keeps their name with nothing to attach it to. Held
  // back until the count says otherwise — offering a verb before knowing it
  // means something is how the husks got made.
  //
  // An account nobody holds is the exception: there a detach is the way to give
  // an orphan a person of its own.
  const detachable =
    boundId == null || (owned.data != null && owned.data.accounts.length > 1);

  // Confirming re-asserts the resolver's guess as the operator's decision. It
  // has something to say only while the account is still queued for exactly
  // that — and only when the surface knows the holder's card to press.
  const confirmable =
    boundCard != null && candidates.some((c) => c.person_id === boundId);

  const close = () => {
    setAction({ kind: "closed" });
    // A dialog's error belongs to the attempt made in THAT dialog; without a
    // reset the next dialog opens already wearing the previous failure.
    bind.reset();
    detach.reset();
    exclude.reset();
  };
  const done = (result: CorrectionResponse) => {
    close();

    // Not "no refusals": the outcome vocabulary is open by contract, so a value
    // this build has never heard of must not pass for success.
    if (!fullyDecided(result)) {
      // The account kept its binding, so the window keeps its verbs: the
      // operator has something left to decide and needs the counters to see it.
      setOutcome(result);
      const refused = refusedCount(result);
      toast.error(
        refused > 0
          ? t("identities.outcomes.toast_refused", { count: refused })
          : t("identities.dialogs.failed"),
      );
      return;
    }

    // An earlier attempt's counters have nothing to say about this one. Today
    // `onDecided` unmounts this window, but the prop is optional.
    setOutcome(null);
    const message =
      result.applied > 0
        ? t("identities.outcomes.toast_applied", { count: result.applied })
        : t("identities.outcomes.toast_already");
    // The minted person's id is the one thing a detach reports that nothing else
    // on the page can name yet, and the window that used to hold it now closes —
    // so the toast carrying it stays up long enough to be copied out.
    toast.success(message, {
      description: result.new_person_id
        ? `${t("identities.outcomes.new_person")} ${result.new_person_id}`
        : undefined,
      duration: result.new_person_id ? MINTED_ID_TOAST_MS : undefined,
    });
    onDecided?.();
  };

  return (
    <div className="flex flex-col gap-4">
      {outcome ? <OutcomeAlert outcome={outcome} /> : null}

      <section>
        <SectionLabel>{t("identities.detail.current_binding")}</SectionLabel>
        {boundId ? (
          <div className="flex items-center gap-2">
            {boundCard ? (
              <PersonCell person={boundCard} className="min-w-0 flex-1" />
            ) : (
              <div className="min-w-0 flex-1">
                <PersonId id={boundId} />
              </div>
            )}
            {/* The confirm act, on the card it acts on. Offered only where the
                resolver's own guess is what stands: an operator-authored
                binding is already decided, and re-asserting it changes
                nothing. */}
            {confirmable ? (
              <Button
                type="button"
                size="xs"
                disabled={busy}
                onClick={() =>
                  boundCard && setAction({ kind: "bind", person: boundCard })
                }
              >
                {t("identities.actions.confirm")}
              </Button>
            ) : null}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            {t("identities.detail.unbound")}
          </p>
        )}
      </section>

      {rivals.length > 0 ? (
        <section>
          <SectionLabel>{t("identities.detail.candidates")}</SectionLabel>
          <div className="flex flex-col gap-2">
            {rivals.map((candidate) => (
              <div key={candidate.person_id} className="flex items-center gap-2">
                <PersonCell person={candidate} className="min-w-0 flex-1" />
                <Button
                  type="button"
                  size="xs"
                  variant="outline"
                  disabled={busy}
                  onClick={() => setAction({ kind: "bind", person: candidate })}
                >
                  {t("identities.actions.bind")}
                </Button>
              </div>
            ))}
          </div>
        </section>
      ) : null}

      {/* One section, two ways in. With a person open behind the window and the
          account not yet theirs, one press is the whole decision — the reader
          is already inside that person, and asking them to search for them
          again is asking twice. Once the account IS theirs that press would
          change nothing, so the search comes back: moving it to somebody else
          is the only decision left, and it is exactly the one an operator
          opened the account to make. */}
      <section>
        <div className="mb-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {boundId
            ? t("identities.actions.assign_other")
            : t("identities.actions.assign_person")}
        </div>
        {bindTo && boundId !== bindTo.person_id ? (
          <Button
            type="button"
            size="sm"
            disabled={busy}
            onClick={() => setAction({ kind: "bind", person: bindTo })}
          >
            {t("identities.actions.bind_to")}
          </Button>
        ) : (
          <PersonPicker
            // The holder too: this picker moves an account to somebody else,
            // and the one person it cannot move it to is the one who already
            // has it. Named by ID rather than by card — a surface that could
            // not hydrate the holder still knows who they are, and without
            // this the search would offer a bind that moves nothing.
            excludeIds={[
              ...candidates.map((c) => c.person_id),
              ...(boundCard ? [boundCard.person_id] : []),
              ...(boundId ? [boundId] : []),
              ...(bindTo ? [bindTo.person_id] : []),
            ]}
            onPick={(person) => setAction({ kind: "bind", person })}
          />
        )}
      </section>

      <section className="flex flex-wrap gap-2">
        {detachable ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => setAction({ kind: "detach" })}
          >
            {t("identities.actions.detach")}
          </Button>
        ) : null}
        <Button
          type="button"
          size="sm"
          variant="destructive"
          disabled={busy}
          onClick={() => setAction({ kind: "exclude" })}
        >
          {t("identities.actions.exclude")}
        </Button>
      </section>

      {action.kind === "bind" ? (
        <ConfirmDialog
          open
          onOpenChange={(open) => !open && close()}
          title={
            action.person.person_id === boundId
              ? t("identities.dialogs.confirm_title")
              : t("identities.dialogs.bind_title")
          }
          description={sentences(
            action.person.person_id === boundId
              ? t("identities.dialogs.confirm_description")
              : boundId
                ? t("identities.dialogs.rebind_description")
                : t("identities.dialogs.bind_description"),
            queued ? t("identities.dialogs.leaves_queue") : null,
          )}
          confirmLabel={
            action.person.person_id === boundId
              ? t("identities.actions.confirm")
              : t("identities.actions.bind")
          }
          isPending={bind.isPending}
          error={
            bind.isError
              ? apiErrorReason(bind.error, t("identities.dialogs.failed"))
              : null
          }
          onConfirm={() =>
            bind.mutate(
              { account: wireRef, person_id: action.person.person_id },
              { onSuccess: done },
            )
          }
        >
          <PersonCell person={action.person} />
        </ConfirmDialog>
      ) : null}

      {action.kind === "detach" ? (
        <ConfirmDialog
          open
          onOpenChange={(open) => !open && close()}
          title={t("identities.dialogs.detach_title")}
          description={sentences(
            t("identities.dialogs.detach_description"),
            queued ? t("identities.dialogs.leaves_queue") : null,
          )}
          confirmLabel={t("identities.actions.detach_confirm")}
          isPending={detach.isPending}
          error={
            detach.isError
              ? apiErrorReason(detach.error, t("identities.dialogs.failed"))
              : null
          }
          onConfirm={() => detach.mutate({ account: wireRef }, { onSuccess: done })}
        />
      ) : null}

      {action.kind === "exclude" ? (
        <ConfirmDialog
          open
          onOpenChange={(open) => !open && close()}
          title={t("identities.dialogs.exclude_title")}
          description={sentences(
            t("identities.dialogs.exclude_description"),
            queued ? t("identities.dialogs.leaves_queue") : null,
          )}
          confirmLabel={t("identities.actions.exclude_confirm")}
          destructive
          isPending={exclude.isPending}
          error={
            exclude.isError
              ? apiErrorReason(exclude.error, t("identities.dialogs.failed"))
              : null
          }
          onConfirm={() => exclude.mutate({ account: wireRef }, { onSuccess: done })}
        />
      ) : null}
    </div>
  );
}

/** The server's answer, verbatim — three counters, never a bare "done". */
function OutcomeAlert({ outcome }: { outcome: CorrectionResponse }) {
  const { t } = useTranslation();
  const refused = outcome.items.filter((i) => i.outcome === "refused").length;
  return (
    <Alert variant={refused > 0 ? "destructive" : "default"} role="status">
      <AlertTitle className="flex flex-wrap items-center gap-1.5">
        <Badge variant="secondary">
          {t("identities.outcomes.applied", { count: outcome.applied })}
        </Badge>
        {outcome.already_decided > 0 ? (
          <Badge variant="outline">
            {t("identities.outcomes.already_decided", {
              count: outcome.already_decided,
            })}
          </Badge>
        ) : null}
        {refused > 0 ? (
          <Badge variant="secondary" className="bg-destructive/15 text-destructive">
            {t("identities.outcomes.refused", { count: refused })}
          </Badge>
        ) : null}
      </AlertTitle>
      {outcome.new_person_id ? (
        <AlertDescription className="font-mono text-xs">
          {t("identities.outcomes.new_person")} {outcome.new_person_id}
        </AlertDescription>
      ) : null}
    </Alert>
  );
}

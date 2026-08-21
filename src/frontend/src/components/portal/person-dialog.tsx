/**
 * One person and every account bound to them — the window the person mode
 * decides in.
 *
 * The mirror of the account window. There the subject is an account, the list
 * is who could hold it, and the field searches PEOPLE; here the subject is a
 * person, the list is what they hold, and the field searches ACCOUNTS. A click
 * on a row means the same thing in both: it picks the other side of a binding,
 * never a door to walk through. One gesture, learnt once.
 *
 * So the rows here open nothing. An account a person holds is acted on in
 * place, by the verbs on its own row — a second window over this one would
 * stack two decisions an operator has to keep apart.
 *
 * The layout mirrors that window too: the field sits high and stays put, and
 * the long section under it takes the slack and scrolls.
 *
 * Opened by `?person=` alone, so a link lands a colleague on the same person.
 */
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type {
  AccountMatch,
  CorrectionResponse,
  PersonAccountEntry,
  PersonSummary,
  WireAccountRef,
} from "@/api/identity-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { AccountPicker } from "@/components/portal/account-picker";
import { OutcomeAlert } from "@/components/portal/outcome-alert";
import { PersonCell, PersonMarks } from "@/components/portal/person-cell";
import { PersonId } from "@/components/portal/person-id";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { useCorrectionReport } from "@/hooks/use-correction-report";
import { accountKey } from "@/lib/identities/account-key";
import { personName } from "@/lib/identities/person-display";
import { apiErrorReason } from "@/lib/query-console/api-error";
import {
  useBindAccount,
  useDetachAccount,
  useExcludeAccount,
  usePersonAccounts,
} from "@/queries/identity-resolution";
import { cn } from "@/lib/utils";

type PendingAction =
  | { kind: "closed" }
  | { kind: "bind"; account: AccountMatch }
  | { kind: "detach"; account: PersonAccountEntry }
  | { kind: "exclude"; account: PersonAccountEntry };

function wireRef(account: {
  source: string;
  source_id: string;
  account_id: string;
}): WireAccountRef {
  return {
    source: account.source,
    source_id: account.source_id,
    // The wire calls the account id `id` (unlike the read shapes).
    id: account.account_id,
  };
}

/** The open subject, kept across the close animation. See below. */
interface HeldPerson {
  id: string;
  card: PersonSummary | null;
}

export function PersonDialog({
  personId,
  card,
  onClose,
}: {
  personId: string | undefined;
  /** The roster row the operator picked, when there is one. Arriving by link
   *  there is none — search resolves values, not ids — and the id stands alone,
   *  honestly. */
  card: PersonSummary | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const popupRef = useRef<HTMLDivElement>(null);
  const liveId = personId ?? null;
  // Only a card for the person actually open: a stale one would caption this
  // window with the person the reader looked at before.
  const liveCard = card?.person_id === liveId ? card : null;

  // The popup plays a close transition after `?person=` is already gone, so the
  // subject has to outlive the URL by that much — otherwise the heading renames
  // itself to "Unnamed person" on the way out. Held via state adjusted during
  // render (the sanctioned previous-render pattern), keyed by id rather than by
  // object, or a freshly built card would loop.
  const [held, setHeld] = useState<HeldPerson | null>(null);
  if (liveId && (held?.id !== liveId || held.card !== liveCard)) {
    setHeld({ id: liveId, card: liveCard });
  }
  const shown = liveId ? { id: liveId, card: liveCard } : held;
  const person: PersonSummary | null = shown
    ? (shown.card ?? { person_id: shown.id })
    : null;
  const named = person ? personName(person) : null;

  return (
    <Dialog
      open={liveId != null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      {/* The same fixed height as the account window: an operator moves between
          the two, and a window that sizes itself to each subject moves the verbs
          under their cursor. */}
      <DialogContent
        ref={popupRef}
        closeLabel={t("common.actions.close")}
        className="flex h-[85vh] flex-col gap-4 sm:max-w-3xl"
        // Focus the window itself, not its first tabbable — that is a verb that
        // takes an account off somebody, and a decision should not be one
        // keypress from arriving. `initialFocus={false}` is not the alternative:
        // it does not move focus AT ALL, stranding it in the aria-hidden page
        // behind the dialog.
        tabIndex={-1}
        initialFocus={popupRef}
      >
        <DialogHeader>
          {/* An `h2`, which is what the primitive renders by default: a reader
              navigating an open window by heading should find the person it is
              about. */}
          <DialogTitle
            render={<h2 className="flex min-w-0 items-center gap-2" />}
          >
            <span
              className={cn(
                "truncate select-text",
                // A ternary, not `named || ...`: `named` is the NAME, and an
                // `||` fallback puts it in the class list — a display name
                // carrying a utility word would then restyle or hide itself.
                named ? undefined : "text-muted-foreground italic",
              )}
            >
              {named ?? t("identities.person.unnamed")}
            </span>
            {/* A leaver and a stub minted by automation are the two people an
                operator should not be moving accounts onto — marked here for the
                same reason the roster marks them. */}
            {person ? <PersonMarks person={person} /> : null}
          </DialogTitle>
          <DialogDescription render={<div className="flex items-center gap-1" />}>
            {person ? <PersonId id={person.person_id} /> : null}
          </DialogDescription>
        </DialogHeader>
        {/* Keyed by the person: the body holds per-person state (a pending verb,
            the counters of the last one), and a cached read renders the next
            person synchronously — unkeyed, that state would follow them. */}
        {person ? (
          <PersonBody
            key={person.person_id}
            person={person}
            onDecided={() => {
              // The control just pressed is about to be unmounted by the
              // refetch — the row goes, or the last detach takes every Detach
              // with it — and focus would fall to `body`, restarting Tab from
              // the top of the document.
              //
              // INVARIANT: after a frame, never in the handler itself. The list
              // has not settled yet inside it.
              requestAnimationFrame(() => popupRef.current?.focus());
            }}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

function PersonBody({
  person,
  onDecided,
}: {
  person: PersonSummary;
  /** A verb settled everything it named, so the control that fired it is going
   *  away with the row it sat on. */
  onDecided: () => void;
}) {
  const { t } = useTranslation();
  const accounts = usePersonAccounts(person.person_id);
  const [action, setAction] = useState<PendingAction>({ kind: "closed" });
  const [outcome, setOutcome] = useState<CorrectionResponse | null>(null);
  const report = useCorrectionReport();

  const bind = useBindAccount();
  const detach = useDetachAccount();
  const exclude = useExcludeAccount();

  const close = () => {
    setAction({ kind: "closed" });
    // A dialog's error belongs to the attempt made in THAT dialog; without a
    // reset the next one opens already wearing the previous failure.
    bind.reset();
    detach.reset();
    exclude.reset();
  };
  const done = (result: CorrectionResponse) => {
    close();
    // A refusal means the account kept its binding, so the counters stay on
    // screen: the operator has something left to decide. The window itself
    // stays either way — the list under it re-reads, and that IS the answer.
    const settled = report(result);
    setOutcome(settled ? null : result);
    // Only when the row really is going. A refused account keeps its row, and
    // its verbs are where the operator's hand already is.
    if (settled) onDecided();
  };

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
  // A detach mints a person and moves the account to them. Taking somebody's
  // ONLY account does that and nothing else: the account still has one holder,
  // and the person it left keeps their name with nothing to attach it to.
  const detachable = entries.length > 1;
  const busy = bind.isPending || detach.isPending || exclude.isPending;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      {outcome ? <OutcomeAlert outcome={outcome} /> : null}

      {/* The field sits high and fixed, the list takes the slack and scrolls —
          the account window's geometry, where the search is right under the
          subject and the history behind it absorbs the height. A field pinned
          to the bottom of a window this tall reads as a footer. */}
      <section className="flex min-h-0 flex-col">
        <SectionLabel>{t("identities.person_window.bind_section")}</SectionLabel>
        <AccountPicker
          placeholder={t("identities.person_window.find_account")}
          // The accounts already listed below: binding one of them to the
          // person who already holds it changes nothing.
          excludeKeys={entries.map(accountKey)}
          onPick={(account) => setAction({ kind: "bind", account })}
        />
      </section>

      <section className="flex min-h-0 flex-1 flex-col">
        <SectionLabel>
          {t("identities.person_accounts.accounts")}
          <Badge variant="secondary">{entries.length}</Badge>
        </SectionLabel>
        {entries.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t("identities.person_accounts.no_accounts")}
          </p>
        ) : (
          <ul className="flex min-h-24 flex-1 flex-col gap-1 overflow-y-auto pe-1">
            {entries.map((entry) => (
              <li key={accountKey(entry)}>
                <AccountRow
                  entry={entry}
                  detachable={detachable}
                  busy={busy}
                  onDetach={() => setAction({ kind: "detach", account: entry })}
                  onExclude={() => setAction({ kind: "exclude", account: entry })}
                />
              </li>
            ))}
          </ul>
        )}
      </section>

      {action.kind === "bind" ? (
        <ConfirmDialog
          open
          onOpenChange={(open) => !open && close()}
          title={t("identities.person_window.bind_title")}
          description={
            action.account.person &&
            action.account.person.person_id !== person.person_id
              ? t("identities.person_window.bind_from_description", {
                  name: personName(action.account.person) ??
                    action.account.person.person_id,
                })
              : t("identities.person_window.bind_description")
          }
          confirmLabel={t("identities.actions.bind")}
          isPending={bind.isPending}
          error={
            bind.isError
              ? apiErrorReason(bind.error, t("identities.dialogs.failed"))
              : null
          }
          onConfirm={() =>
            bind.mutate(
              {
                account: wireRef(action.account),
                person_id: person.person_id,
              },
              { onSuccess: done },
            )
          }
        >
          <AccountLine account={action.account} />
          <PersonCell person={person} />
        </ConfirmDialog>
      ) : null}

      {action.kind === "detach" ? (
        <ConfirmDialog
          open
          onOpenChange={(open) => !open && close()}
          title={t("identities.person_window.detach_title")}
          description={t("identities.person_window.detach_description")}
          confirmLabel={t("identities.actions.detach_confirm")}
          isPending={detach.isPending}
          error={
            detach.isError
              ? apiErrorReason(detach.error, t("identities.dialogs.failed"))
              : null
          }
          onConfirm={() =>
            detach.mutate({ account: wireRef(action.account) }, { onSuccess: done })
          }
        >
          <AccountLine account={action.account} />
        </ConfirmDialog>
      ) : null}

      {action.kind === "exclude" ? (
        <ConfirmDialog
          open
          onOpenChange={(open) => !open && close()}
          title={t("identities.dialogs.exclude_title")}
          description={t("identities.dialogs.exclude_description")}
          confirmLabel={t("identities.actions.exclude_confirm")}
          destructive
          isPending={exclude.isPending}
          error={
            exclude.isError
              ? apiErrorReason(exclude.error, t("identities.dialogs.failed"))
              : null
          }
          onConfirm={() =>
            exclude.mutate({ account: wireRef(action.account) }, { onSuccess: done })
          }
        >
          <AccountLine account={action.account} />
        </ConfirmDialog>
      ) : null}
    </div>
  );
}

/**
 * The account a confirmation is about.
 *
 * The window's subject is the PERSON, so a confirmation that names only them
 * restates what the operator already knows while the modal covers the row they
 * pressed. Every verb here acts on one account; this is which one.
 */
function AccountLine({
  account,
}: {
  account: { source: string; account_id: string; email?: string | null; username?: string | null };
}) {
  const label = account.email?.trim() || account.username?.trim() || account.account_id;
  return (
    <div className="min-w-0 rounded-md border p-2">
      <div className="truncate text-sm font-medium select-text">{label}</div>
      <div className="truncate font-mono text-xs text-muted-foreground select-text">
        {account.source} · {account.account_id}
      </div>
    </div>
  );
}

/**
 * One account this person holds, and the verbs that take it off them.
 *
 * Deliberately not clickable: nothing opens from here, so the row carries no
 * hover state either — a row that invites a press and answers nothing reads as
 * a fault.
 */
function AccountRow({
  entry,
  detachable,
  busy,
  onDetach,
  onExclude,
}: {
  entry: PersonAccountEntry;
  detachable: boolean;
  busy: boolean;
  onDetach: () => void;
  onExclude: () => void;
}) {
  const { t } = useTranslation();
  const label = entry.email?.trim() || entry.username?.trim() || entry.account_id;
  // Every row's verbs read alike, so their labels have to carry the account:
  // six accounts otherwise produce six identical "Detach" buttons, and the
  // row's own text is announced by nothing.
  const names = { account: label, source: entry.source };
  return (
    // Fixed columns, and the same first two as the account listing: the two
    // lists sit one tab apart, and reading either one down a column should feel
    // the same. `md`, not `lg`: this row lives in a window capped at 48rem, so
    // a viewport wider than that buys it nothing.
    <div className="grid grid-cols-1 items-center gap-2 rounded-md border border-transparent p-3 md:grid-cols-[minmax(0,1fr)_minmax(0,11rem)_auto]">
      <div className="min-w-0">
        <div className="truncate text-sm font-medium select-text">{label}</div>
        <div className="truncate font-mono text-xs text-muted-foreground select-text">
          {entry.source} · {entry.account_id}
        </div>
      </div>
      {/* Who decided this binding is the first thing to know before changing
          it: undoing automation is routine, overruling a colleague is not. */}
      <Badge variant="outline" className="justify-self-start font-normal">
        {entry.bound_by_operator
          ? t("identities.person_accounts.by_operator")
          : t("identities.person_accounts.by_automation")}
      </Badge>
      <div className="flex shrink-0 flex-wrap gap-2">
        {detachable ? (
          <Button
            type="button"
            size="xs"
            variant="outline"
            disabled={busy}
            aria-label={t("identities.person_window.detach_account", names)}
            onClick={onDetach}
          >
            {t("identities.person_window.detach")}
          </Button>
        ) : null}
        <Button
          type="button"
          size="xs"
          variant="destructive"
          disabled={busy}
          aria-label={t("identities.person_window.exclude_account", names)}
          onClick={onExclude}
        >
          {t("identities.person_window.exclude")}
        </Button>
      </div>
    </div>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="mb-1.5 flex items-center gap-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
      {children}
    </div>
  );
}

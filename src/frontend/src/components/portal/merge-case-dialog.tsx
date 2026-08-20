/**
 * Merging a case: the operator names the person who STAYS, and picks which of
 * the others are merged into them.
 *
 * Why here and not in the account window: a merge is a claim about people ("all
 * of these are one human"), and the case is where the people are listed. In the
 * account window the button sat in the row of the person who would SURVIVE while
 * the one being absorbed was named in a different section above it — nothing on
 * screen said which way round it went.
 *
 * Choosing only the survivor is what makes the direction unambiguous: there is
 * no "from", because everyone else is the from. The row's button says the same
 * thing in words — it merges INTO the person whose row it sits in.
 *
 * A case can argue over more than two people, and then all-or-nothing is the
 * wrong offer: two of them may be one human while the third is a different one
 * who happens to share an address. So the absorbed set is CHOSEN here, and only
 * what is ticked moves. With one other person there is nothing to choose and the
 * list is just shown.
 *
 * The endpoint joins exactly two persons, so a case of three or more is a short
 * SEQUENCE of calls. Whatever the sequence does is reported by TOAST, never by
 * this dialog's own error slot alone: the first successful merge prunes the rows
 * it decided, which changes the case's identity and remounts the block that owns
 * this dialog — so a message left in component state would be unmounted before
 * it could be read.
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import type { CorrectionResponse, PersonSummary } from "@/api/identity-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { PersonCell } from "@/components/portal/person-cell";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { personDisplayName } from "@/lib/identities/person-display";
import {
  combineOutcomes,
  fullyDecided,
  refusedCount,
} from "@/lib/identities/outcomes";
import { apiErrorReason } from "@/lib/query-console/api-error";
import {
  useMergePersons,
  usePersonAccountsMany,
} from "@/queries/identity-resolution";

/** Accounts named in the preview before it says "…and N more". */
const PREVIEW_ROWS = 5;

export function MergeCaseDialog({
  survivor,
  absorbed,
  onClose,
}: {
  /** The person the operator pressed: they keep their id and gain the rest. */
  survivor: PersonSummary;
  /** Everyone else in the case, offered for absorption. Never empty — the caller
   *  offers no merge for a case with a single person. */
  absorbed: PersonSummary[];
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const merge = useMergePersons();
  // Own state, not the mutation's: one decision spans several calls, and the
  // hook's flags describe only the last of them.
  const [running, setRunning] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [chosen, setChosen] = useState<ReadonlySet<string>>(
    () => new Set(absorbed.map((person) => person.person_id)),
  );

  const taking = absorbed.filter((person) => chosen.has(person.person_id));
  const moving = usePersonAccountsMany(taking.map((person) => person.person_id));
  const survivorName = personDisplayName(survivor);

  const toggle = (personId: string) =>
    setChosen((current) => {
      const next = new Set(current);
      if (!next.delete(personId)) next.add(personId);
      return next;
    });

  const run = async () => {
    if (running) return;
    setRunning(true);
    setFailure(null);

    const landed: CorrectionResponse[] = [];
    for (const person of taking) {
      try {
        landed.push(
          await merge.mutateAsync({
            source_person_id: person.person_id,
            target_person_id: survivor.person_id,
          }),
        );
      } catch (error) {
        setRunning(false);
        report(t, setFailure, apiErrorReason(error, t("identities.dialogs.failed")), landed);
        return;
      }
    }
    setRunning(false);

    const result = combineOutcomes(landed);
    // Not "no refusals": the outcome vocabulary is open by contract, and a value
    // this build has never heard of must not pass for success.
    if (!fullyDecided(result)) {
      const refused = refusedCount(result);
      report(
        t,
        setFailure,
        refused > 0
          ? t("identities.outcomes.toast_refused", { count: refused })
          : t("identities.dialogs.failed"),
        landed,
      );
      return;
    }

    toast.success(t("identities.outcomes.toast_applied", { count: result.applied }));
    onClose();
  };

  const nothingChosen = taking.length === 0;
  return (
    <ConfirmDialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={t("identities.dialogs.merge_case_title", { name: survivorName })}
      description={t("identities.dialogs.merge_case_description", {
        name: survivorName,
      })}
      confirmLabel={t("identities.actions.merge_confirm")}
      destructive
      isPending={running}
      // The preview IS the consent: confirmed before the reads land, this would
      // move accounts the operator never saw named. And a merge that moves
      // nothing is not a decision — it answers "already decided, nothing
      // changed" while the case comes straight back on the next read.
      confirmDisabled={
        nothingChosen || !moving.ready || moving.accounts.length === 0
      }
      error={failure}
      onConfirm={() => void run()}
    >
      <div className="flex flex-col gap-3">
        <div className="flex flex-col gap-1.5">
          <SectionLabel>{t("identities.dialogs.merge_case_stays")}</SectionLabel>
          <PersonCell person={survivor} />
        </div>

        <div className="flex flex-col gap-1.5">
          <SectionLabel>
            {t("identities.dialogs.merge_case_absorbed", {
              count: absorbed.length,
            })}
          </SectionLabel>
          {absorbed.map((person) =>
            absorbed.length === 1 ? (
              <PersonCell key={person.person_id} person={person} />
            ) : (
              // Ticked by default: the common case is that the whole case is one
              // human. Unticking is for the case that is not.
              <label
                key={person.person_id}
                htmlFor={`absorb-${person.person_id}`}
                className="flex cursor-pointer items-center gap-2 rounded-sm p-1 hover:bg-muted"
              >
                <Checkbox
                  id={`absorb-${person.person_id}`}
                  checked={chosen.has(person.person_id)}
                  disabled={running}
                  onCheckedChange={() => toggle(person.person_id)}
                />
                <PersonCell person={person} className="min-w-0 flex-1" />
              </label>
            ),
          )}
        </div>

        {nothingChosen ? (
          <p className="text-sm text-muted-foreground">
            {t("identities.dialogs.merge_case_none_chosen")}
          </p>
        ) : moving.failed ? (
          <div className="flex items-center gap-2 text-sm text-destructive">
            <span>{t("identities.dialogs.merge_preview_failed")}</span>
            <Button
              type="button"
              size="xs"
              variant="outline"
              onClick={() => moving.refetch()}
            >
              {t("common.actions.retry")}
            </Button>
          </div>
        ) : !moving.ready ? (
          <p className="text-sm text-muted-foreground">
            {t("identities.dialogs.merge_preview_loading")}
          </p>
        ) : (
          <div className="text-sm">
            <p>
              {t("identities.dialogs.merge_preview", {
                count: moving.accounts.length,
              })}
            </p>
            <ul className="mt-1.5 flex flex-col gap-1">
              {moving.accounts.slice(0, PREVIEW_ROWS).map((account) => (
                <li
                  key={`${account.source}:${account.source_id}:${account.account_id}`}
                  className="truncate font-mono text-xs text-muted-foreground"
                >
                  {account.source} ·{" "}
                  {account.email ?? account.username ?? account.account_id}
                </li>
              ))}
            </ul>
            {moving.accounts.length > PREVIEW_ROWS ? (
              <p className="mt-1 text-xs text-muted-foreground">
                {t("identities.dialogs.merge_preview_more", {
                  count: moving.accounts.length - PREVIEW_ROWS,
                })}
              </p>
            ) : null}
          </div>
        )}
      </div>
    </ConfirmDialog>
  );
}

/**
 * Say what went wrong through BOTH channels.
 *
 * The dialog's own slot is the one an operator is looking at, but it may be gone:
 * a merge that already succeeded prunes the rows it decided, which re-keys the
 * case and remounts the block holding this dialog. The toast outlives that. And
 * whatever DID land is named, or the operator is left unable to tell a sequence
 * that failed at the first call from one that failed at the last.
 */
function report(
  t: (key: string, vars?: Record<string, unknown>) => string,
  setFailure: (message: string) => void,
  reason: string,
  landed: readonly CorrectionResponse[],
) {
  const applied = combineOutcomes(landed).applied;
  setFailure(reason);
  toast.error(reason, {
    description:
      applied > 0
        ? t("identities.outcomes.toast_partial", { count: applied })
        : undefined,
  });
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
      {children}
    </div>
  );
}

/**
 * Confirm a whole queue group at once.
 *
 * A roster sync can add a hundred people in one go, and every one of them lands
 * as its own case asking the same question: is the resolver's guess yours? One
 * at a time that is a hundred windows and a hundred presses for a single
 * decision, so the decision is offered once, over the group.
 *
 * Which groups qualify is `groupIsConfirmable`'s call, next to the rest of the
 * queue-case vocabulary.
 *
 * The comment is required, not optional: one press stands in for a hundred
 * decisions and the trail has to say on what grounds.
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "@/components/ui/sonner";

import {
  MAX_BINDINGS_PER_CALL,
  type AttentionItem,
  type CorrectionResponse,
  type WireBinding,
} from "@/api/identity-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { combineOutcomes, fullyDecided, refusedCount } from "@/lib/identities/outcomes";
import { apiErrorReason } from "@/lib/query-console/api-error";
import { useBindAccounts } from "@/queries/identity-resolution";

/** Split into calls the endpoint will accept. */
function chunk(bindings: WireBinding[]): WireBinding[][] {
  const calls: WireBinding[][] = [];
  for (let at = 0; at < bindings.length; at += MAX_BINDINGS_PER_CALL) {
    calls.push(bindings.slice(at, at + MAX_BINDINGS_PER_CALL));
  }
  return calls;
}

export function ConfirmGroupButton({
  items,
  className,
}: {
  /** Every row of the group. The caller has already checked
   *  {@link groupIsConfirmable}. */
  items: AttentionItem[];
  className?: string;
}) {
  const { t } = useTranslation();
  const bind = useBindAccounts();
  const [open, setOpen] = useState(false);
  const [comment, setComment] = useState("");
  const [running, setRunning] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const bindings: WireBinding[] = items.flatMap((item) =>
    item.bound_to
      ? [
          {
            account: {
              source: item.source,
              source_id: item.source_id,
              id: item.account_id,
            },
            person_id: item.bound_to,
          },
        ]
      : [],
  );

  const close = () => {
    setOpen(false);
    setComment("");
    setFailure(null);
    bind.reset();
  };

  const run = async () => {
    if (running) return;
    setRunning(true);
    setFailure(null);

    const landed: CorrectionResponse[] = [];
    for (const call of chunk(bindings)) {
      try {
        landed.push(await bind.mutateAsync({ bindings: call, comment }));
      } catch (error) {
        setRunning(false);
        // By toast as well: the group's rows are pruned as each call lands, so
        // this dialog can be unmounted before a later failure is read.
        const reason = apiErrorReason(error, t("identities.dialogs.failed"));
        setFailure(reason);
        toast.error(reason);
        return;
      }
    }
    setRunning(false);

    const result = combineOutcomes(landed);
    if (!fullyDecided(result)) {
      const refused = refusedCount(result);
      const reason =
        refused > 0
          ? t("identities.outcomes.toast_refused", { count: refused })
          : t("identities.dialogs.failed");
      setFailure(reason);
      toast.error(reason);
      return;
    }

    toast.success(
      t("identities.outcomes.toast_applied", { count: result.applied }),
    );
    close();
  };

  return (
    <>
      <Button
        type="button"
        size="sm"
        variant="outline"
        className={className}
        onClick={() => setOpen(true)}
      >
        {t("identities.actions.confirm_group", { count: bindings.length })}
      </Button>
      {open ? (
        <ConfirmDialog
          open
          onOpenChange={(next) => !next && close()}
          title={t("identities.dialogs.confirm_group_title", {
            count: bindings.length,
          })}
          description={t("identities.dialogs.confirm_group_description")}
          confirmLabel={t("identities.actions.confirm")}
          isPending={running}
          // One press for a hundred decisions has to say on what grounds.
          confirmDisabled={comment.trim().length === 0}
          error={failure}
          onConfirm={() => void run()}
        >
          <Textarea
            value={comment}
            onChange={(event) => setComment(event.target.value)}
            placeholder={t("identities.dialogs.confirm_group_comment")}
            rows={3}
            aria-label={t("identities.dialogs.confirm_group_comment")}
          />
        </ConfirmDialog>
      ) : null}
    </>
  );
}

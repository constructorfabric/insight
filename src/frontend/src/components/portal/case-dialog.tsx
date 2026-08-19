import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type { AttentionItem, PersonSummary } from "@/api/identity-client";
import { CopyValueButton } from "@/components/copy-value-button";
import { AccountDetail } from "@/components/portal/account-detail";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { itemKey, parseAccountKey } from "@/lib/identities/account-key";

/**
 * A row this window can open.
 *
 * `candidates` is a QUEUE concept — the persons the evidence says could own the
 * account, which is a question only an undecided account has. The surfaces that
 * list settled accounts have no such question, so they carry `holder` instead:
 * the card of whoever holds it, which the binding read answers with an id alone.
 * Without that channel those surfaces had to pass the holder AS a candidate, and
 * the window then offered to bind the account to the person already holding it.
 * That offer belongs in the queue: re-asserting a binding the resolver made is
 * the confirm act, and the accounts it matters for are queued for exactly that.
 * Here it either changed nothing at all (the binding was already an operator's)
 * or only flipped a badge.
 */
export interface CaseRow extends AttentionItem {
  holder?: PersonSummary | null;
}

/** What the open case keeps when the list under it moves (see below). */
interface HeldCase {
  acct?: string;
  item?: CaseRow;
  at?: number;
}

/**
 * One account under review, in a window rather than a column: this is where
 * every decision is taken, and a decision that re-attributes a person's work
 * deserves the room to show what it acts on.
 *
 * Opened by the `?acct=` in the URL — never by click state alone — so a link
 * an operator shares lands their colleague on the same case.
 *
 * A decision prunes its row from the list at once (see `useCorrection`), and
 * that list feeds this window: taken literally, the operator's own success
 * would yank the candidates, the outcome alert and the prev/next footer out
 * from under them. The window therefore HOLDS what it knew about the open
 * case — the row and its position — and lets the list move underneath.
 */
export function CaseDialog({
  acct,
  items,
  ordered,
  bindTo,
  onSelect,
  onClose,
}: {
  acct: string | undefined;
  items: CaseRow[];
  /** Account keys in the order the queue renders them. */
  ordered: string[];
  /** The person the surface behind this window has open, when it has one:
   *  binding to them is then one press rather than a search. */
  bindTo?: PersonSummary | null;
  onSelect: (key: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const popupRef = useRef<HTMLDivElement>(null);
  const ref = parseAccountKey(acct);

  const live = items.find((i) => itemKey(i) === acct);
  const liveAt = acct ? ordered.indexOf(acct) : -1;

  // Held via state adjusted during render (the sanctioned previous-render
  // pattern), so the freshest row and position stick for the open case.
  const [held, setHeld] = useState<HeldCase>({});
  const fresh: HeldCase = {
    acct,
    item: live ?? (held.acct === acct ? held.item : undefined),
    at: liveAt >= 0 ? liveAt : held.acct === acct ? held.at : undefined,
  };
  if (held.acct !== fresh.acct || held.item !== fresh.item || held.at !== fresh.at) {
    setHeld(fresh);
  }
  const queueItem = fresh.item;

  const heading =
    queueItem?.email?.trim() ||
    queueItem?.username?.trim() ||
    queueItem?.display_name?.trim() ||
    ref?.account_id;
  // The caller vouching for the account (a queue row, a search hit) is what
  // separates a real account from a mistyped link; the vouching survives the
  // row being pruned, since the account did not stop existing.
  const observed = queueItem != null || liveAt >= 0;

  // When the open row was pruned, everything after it shifted left — so the
  // held index now points at the NEXT account, which is where a conveyor
  // should go, and one before it is still the previous one.
  const at = liveAt >= 0 ? liveAt : (fresh.at ?? -1);
  const previous = at > 0 ? ordered[at - 1] : undefined;
  const next = liveAt >= 0 ? ordered[at + 1] : at >= 0 ? ordered[at] : undefined;

  return (
    <Dialog
      open={ref != null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      {/* A fixed height, not a fitted one: an operator walks from case to case
          and a window that resizes to each one moves the verbs under their
          cursor. The history takes the slack instead. */}
      <DialogContent
        ref={popupRef}
        className="flex h-[85vh] flex-col gap-4 sm:max-w-3xl"
        // Focus the window itself. The default first-tabbable is the header's
        // copy control ("Copy <account id>" — the wrong first thing to hear),
        // and `initialFocus={false}` does not move focus AT ALL, stranding it
        // in the aria-hidden page behind the dialog.
        tabIndex={-1}
        initialFocus={popupRef}
      >
        <DialogHeader>
          <DialogTitle className="truncate select-text">{heading}</DialogTitle>
          <DialogDescription
            render={<div className="flex items-center gap-1" />}
          >
            <span className="truncate font-mono text-xs select-text">
              {ref?.source} · {ref?.account_id}
            </span>
            {ref ? (
              <CopyValueButton
                value={ref.account_id}
                title={t("identities.detail.copy_account_id")}
                copyLabel={t("common.copy")}
                copiedLabel={t("common.copied")}
                errorMessage={t("common.copy_failed")}
              />
            ) : null}
          </DialogDescription>
        </DialogHeader>
        {/* Keyed by the account: the body holds per-account state (a verb's
            outcome, an open confirmation), and a cached binding renders the
            next case synchronously — unkeyed, that state would follow. */}
        {ref ? (
          <AccountDetail
            key={acct}
            accountRef={ref}
            queueItem={queueItem}
            observed={observed}
            holder={queueItem?.holder ?? null}
            bindTo={bindTo}
          />
        ) : null}
        {/* Working a backlog is a conveyor: the next case is one press away,
            without a trip back through the list. */}
        {previous || next ? (
          <div className="flex shrink-0 justify-between gap-2 border-t pt-3">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={!previous}
              onClick={() => previous && onSelect(previous)}
            >
              {t("identities.queue.previous_case")}
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={!next}
              onClick={() => next && onSelect(next)}
            >
              {t("identities.queue.next_case")}
            </Button>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

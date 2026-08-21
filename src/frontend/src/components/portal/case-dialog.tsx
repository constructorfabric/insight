import { useRef, useState } from "react";

import type { AttentionItem, PersonSummary } from "@/api/identity-client";
import { AccountDetail } from "@/components/portal/account-detail";
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
 * account, which is a question only an undecided account has. The accounts mode
 * lists settled accounts and has no such question, so it carries `holder`
 * instead: the card of whoever holds it, which the binding read answers with an
 * id alone. Without that channel it had to pass the holder AS a candidate, and
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
 * would yank the candidates and the outcome alert out from under them. The
 * window therefore HOLDS the row it was opened on and lets the list move
 * underneath.
 */
export function CaseDialog({
  acct,
  items,
  onClose,
}: {
  acct: string | undefined;
  items: CaseRow[];
  onClose: () => void;
}) {
  const popupRef = useRef<HTMLDivElement>(null);
  const ref = parseAccountKey(acct);

  const live = items.find((i) => itemKey(i) === acct);

  // Held via state adjusted during render (the sanctioned previous-render
  // pattern), so the freshest row sticks for the open case.
  const [held, setHeld] = useState<HeldCase>({});
  const fresh: HeldCase = {
    acct,
    item: live ?? (held.acct === acct ? held.item : undefined),
  };
  if (held.acct !== fresh.acct || held.item !== fresh.item) {
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
  const observed = queueItem != null;

  return (
    <Dialog
      open={ref != null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      {/* A fixed height, not a fitted one: an operator moves between windows
          and one that resizes to each subject moves the verbs under their
          cursor. The history takes the slack instead. */}
      <DialogContent
        ref={popupRef}
        className="flex h-[85vh] flex-col gap-4 sm:max-w-3xl"
        // Focus the window itself, not its first tabbable — that is a verb, and
        // a decision should not be one keypress from arriving. `initialFocus={false}`
        // is not the alternative: it does not move focus AT ALL, stranding it in
        // the aria-hidden page behind the dialog.
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
            // A decided account has nothing left to answer here: its candidate
            // list and its binding are both reads the server has moved past.
            // The surface behind re-reads (see `useCorrection`), so handing the
            // window back shows the new state instead of the one just replaced.
            onDecided={onClose}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

import { useTranslation } from "react-i18next";

import type { AttentionItem } from "@/api/identity-client";
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
 * One account under review, in a window rather than a column: this is where
 * every decision is taken, and a decision that re-attributes a person's work
 * deserves the room to show what it acts on.
 *
 * Opened by the `?acct=` in the URL — never by click state alone — so a link
 * an operator shares lands their colleague on the same case.
 */
export function CaseDialog({
  acct,
  items,
  ordered,
  onSelect,
  onClose,
}: {
  acct: string | undefined;
  items: AttentionItem[];
  /** Account keys in the order the queue renders them. */
  ordered: string[];
  onSelect: (key: string) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const ref = parseAccountKey(acct);
  const queueItem = items.find((i) => itemKey(i) === acct);
  const heading =
    queueItem?.email?.trim() || queueItem?.username?.trim() || ref?.account_id;
  const at = acct ? ordered.indexOf(acct) : -1;
  const previous = at > 0 ? ordered[at - 1] : undefined;
  const next = at >= 0 ? ordered[at + 1] : undefined;

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
        className="flex h-[85vh] flex-col gap-4 sm:max-w-3xl"
        // The window itself, not its first tabbable child — which is a copy
        // control in the header, and opening a case reading "Copy dev-42"
        // announces the wrong thing and rings the wrong element.
        initialFocus={false}
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
          <AccountDetail key={acct} accountRef={ref} queueItem={queueItem} />
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

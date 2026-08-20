/**
 * A confirmation step for actions that change shared state — the first one in
 * this codebase (existing delete flows fire immediately; identity corrections
 * must not). Composed from `ui/dialog` in the console `StatusDialog` idiom
 * rather than vendoring a new primitive.
 *
 * The body is the caller's: a merge renders its preview there, a bind names
 * its target. The error slot keeps the dialog open on failure — closing over
 * an unshown error would read as success.
 */
import { TriangleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  children,
  confirmLabel,
  destructive = false,
  isPending,
  confirmDisabled = false,
  error,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  children?: React.ReactNode;
  confirmLabel: string;
  destructive?: boolean;
  isPending: boolean;
  /** The caller's body is not ready to be confirmed — a preview still
   *  loading or failed. Cancel stays available; only confirm locks. */
  confirmDisabled?: boolean;
  error?: string | null;
  onConfirm: () => void;
}) {
  const { t } = useTranslation();
  // A verb in flight is not dismissable. Disabling Cancel is not enough:
  // Escape, an overlay click and the built-in close button all bypass it, and
  // closing resets the mutation — so the operator would lose the outcome of a
  // write the server goes on to apply.
  const requestClose = (next: boolean) => {
    if (isPending && !next) return;
    onOpenChange(next);
  };
  return (
    <Dialog open={open} onOpenChange={requestClose}>
      <DialogContent className="sm:max-w-md" showCloseButton={!isPending}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description ? (
            <DialogDescription>{description}</DialogDescription>
          ) : null}
        </DialogHeader>
        {children}
        {error ? (
          <Alert variant="destructive">
            <TriangleAlert />
            <AlertDescription>{error}</AlertDescription>
          </Alert>
        ) : null}
        <DialogFooter>
          <Button
            type="button"
            variant="ghost"
            onClick={() => requestClose(false)}
            disabled={isPending}
          >
            {t("common.actions.cancel")}
          </Button>
          <Button
            type="button"
            variant={destructive ? "destructive" : "default"}
            onClick={onConfirm}
            disabled={isPending || confirmDisabled}
          >
            {confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

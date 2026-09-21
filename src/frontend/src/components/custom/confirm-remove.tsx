import { useState, type ReactNode } from "react";

import { refusal } from "@/components/custom/refusal";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/**
 * Removal in two clicks, with the refusal shown where the choice was made.
 *
 * A refusal leaves the choice open: the service names what still reads the
 * thing, and the reader may fix that and try again without leaving the page.
 */
export function ConfirmRemove({
  ask,
  confirm,
  pending,
  error,
  onRemove,
  onKeep,
}: {
  /** The first click, drawn by the caller: an icon in a row, a button on a page. */
  ask: (open: () => void) => ReactNode;
  confirm: string;
  pending: boolean;
  error: unknown;
  onRemove: () => void;
  /** Called when the reader steps back, so a stale refusal is cleared. */
  onKeep: () => void;
}) {
  const [asked, setAsked] = useState(false);

  if (!asked) return <>{ask(() => setAsked(true))}</>;

  return (
    <span className="flex flex-col items-end gap-1">
      <span className="flex items-center gap-1">
        <Button
          variant="ghost"
          size="sm"
          className="text-destructive"
          disabled={pending}
          onClick={onRemove}
        >
          {pending ? <Spinner className="size-3" /> : null}
          {confirm}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            setAsked(false);
            onKeep();
          }}
        >
          Keep
        </Button>
      </span>

      {error ? (
        <span role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
          {refusal(error, "Couldn't remove it.")}
        </span>
      ) : null}
    </span>
  );
}

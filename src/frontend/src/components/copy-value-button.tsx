/**
 * Copy one value, with the result fed back on the control itself rather than
 * as a toast: the button is what the operator is looking at when they press it.
 *
 * The value goes into the accessible name — a column of identical "Copy"
 * buttons tells a screen-reader user nothing about which row they are on.
 */
import { useEffect, useRef, useState } from "react";
import { Check, Copy } from "lucide-react";
import { toast } from "@/components/ui/sonner";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

const COPIED_RESET_MS = 1500;

export function CopyValueButton({
  value,
  title,
  copyLabel = "Copy",
  copiedLabel = "Copied",
  errorMessage = "Unable to copy",
  className,
}: {
  value: string;
  /** Hover text — what kind of value this is. */
  title?: string;
  /** Verb the accessible name is built from: `<copyLabel> <value>`. */
  copyLabel?: string;
  copiedLabel?: string;
  errorMessage?: string;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (resetTimer.current != null) window.clearTimeout(resetTimer.current);
    },
    [],
  );

  async function copyValue(): Promise<void> {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      if (resetTimer.current != null) window.clearTimeout(resetTimer.current);
      resetTimer.current = window.setTimeout(
        () => setCopied(false),
        COPIED_RESET_MS,
      );
    } catch {
      setCopied(false);
      toast.error(errorMessage);
    }
  }

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-xs"
      className={cn(
        "shrink-0 text-muted-foreground hover:text-foreground",
        className,
      )}
      aria-label={copied ? copiedLabel : `${copyLabel} ${value}`}
      title={copied ? copiedLabel : (title ?? copyLabel)}
      onClick={(event) => {
        event.stopPropagation();
        void copyValue();
      }}
    >
      {copied ? <Check /> : <Copy />}
    </Button>
  );
}

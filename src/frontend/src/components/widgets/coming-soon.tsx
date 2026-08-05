import { AlertTriangle, Clock, Loader2 } from "lucide-react";
import { memo } from "react";

import { cn } from "@/lib/utils";

type ComingSoonVariant = "card" | "chip" | "row";
type ComingSoonState = "empty" | "error" | "loading";

export interface ComingSoonProps {
  variant?: ComingSoonVariant;
  state?: ComingSoonState;
  label?: string;
  onRetry?: () => void;
}

const DEFAULT_LABELS: Record<ComingSoonState, string> = {
  empty: "No data for this period",
  error: "Unable to load",
  loading: "Loading…",
};

const STATE_STYLES: Record<
  ComingSoonState,
  { border: string; bg: string; text: string; icon: string; Icon: typeof Clock }
> = {
  empty: {
    border: "border-border",
    bg: "bg-muted/30",
    text: "text-muted-foreground",
    icon: "text-muted-foreground",
    Icon: Clock,
  },
  error: {
    border: "border-destructive/30",
    bg: "bg-destructive/5",
    text: "text-destructive",
    icon: "text-destructive",
    Icon: AlertTriangle,
  },
  loading: {
    border: "border-border",
    bg: "bg-muted/40 animate-pulse",
    text: "text-muted-foreground",
    icon: "text-muted-foreground",
    Icon: Loader2,
  },
};

function ComingSoonImpl({
  variant = "card",
  state = "empty",
  label,
  onRetry,
}: ComingSoonProps) {
  const s = STATE_STYLES[state];
  const Icon = s.Icon;
  const displayLabel = label ?? DEFAULT_LABELS[state];
  const showRetry = state === "error" && Boolean(onRetry);

  if (variant === "chip") {
    return (
      <span
        role="status"
        aria-label={displayLabel}
        className={cn(
          "inline-flex items-center gap-1 rounded-full border border-dashed px-2 py-0.5 text-xs font-medium",
          s.border,
          s.text,
        )}
      >
        <Icon className={cn("size-3", s.icon)} aria-hidden />
        {displayLabel}
        {showRetry ? (
          <button
            type="button"
            onClick={onRetry}
            className="text-destructive hover:text-destructive/80 ml-1 text-xs font-semibold focus-visible:underline focus-visible:outline-none"
          >
            Retry
          </button>
        ) : null}
      </span>
    );
  }

  if (variant === "row") {
    return (
      <div
        role="status"
        aria-label={displayLabel}
        className={cn(
          "flex items-center gap-2 rounded-md border border-dashed px-3 py-2 text-xs",
          s.border,
          s.text,
        )}
      >
        <Icon className={cn("size-3.5", s.icon)} aria-hidden />
        <span>{displayLabel}</span>
        {showRetry ? (
          <button
            type="button"
            onClick={onRetry}
            className="text-destructive hover:text-destructive/80 ml-auto text-xs font-semibold focus-visible:underline focus-visible:outline-none"
          >
            Retry
          </button>
        ) : null}
      </div>
    );
  }

  return (
    <div
      role="status"
      aria-label={displayLabel}
      className={cn(
        "flex flex-col items-center justify-center gap-1 rounded-lg border border-dashed px-4 py-6 text-center",
        s.border,
        s.bg,
      )}
    >
      <Icon className={cn("size-5", s.icon)} aria-hidden />
      <span className={cn("text-xs font-medium", s.text)}>{displayLabel}</span>
      {showRetry ? (
        <button
          type="button"
          onClick={onRetry}
          className="border-destructive/30 bg-card text-destructive hover:bg-destructive/10 focus-visible:ring-destructive/40 mt-1 rounded-md border px-2.5 py-1 text-xs font-semibold focus-visible:ring-2 focus-visible:outline-none"
        >
          Retry
        </button>
      ) : null}
    </div>
  );
}

export const ComingSoon = memo(ComingSoonImpl);

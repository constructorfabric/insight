import type { KeyboardEvent, ReactNode } from "react";

import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { ComingSoon } from "@/components/widgets/coming-soon";
import { TEXT_HEADING } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const STANDARD_HEIGHT = 304;
const TALL_HEIGHT = 464;
const SKELETON_BARS = [40, 65, 45, 80, 55, 90, 70];

interface WidgetFrameProps {
  title: ReactNode;
  subtitle?: ReactNode;
  action?: ReactNode;
  state: "ready" | "loading" | "error";
  errorLabel?: string;
  onRetry?: () => void;
  onBodyActivate?: () => void;
  bodyLabel?: string;
  tall?: boolean;
  className?: string;
  children?: ReactNode;
}

export function WidgetFrame({
  title,
  subtitle,
  action,
  state,
  errorLabel = "Could not load this widget.",
  onRetry,
  onBodyActivate,
  bodyLabel,
  tall = false,
  className,
  children,
}: WidgetFrameProps) {
  return (
    <Card
      className={cn(
        "min-w-0 gap-0 rounded-[12px] border border-border py-0 shadow-[0_2px_3px_#12204805] ring-0",
        className
      )}
      style={{ height: tall ? TALL_HEIGHT : STANDARD_HEIGHT }}
    >
      <div className="flex min-h-[76px] shrink-0 items-start justify-between gap-2.5 px-6 pt-[23px] pb-3.5">
        <div className="min-w-0">
          <h3 className={cn(TEXT_HEADING, "truncate leading-tight")}>
            {title}
          </h3>
          {subtitle ? (
            <p className="mt-1.5 line-clamp-2 max-w-[440px] text-xs leading-normal text-muted-foreground">
              {subtitle}
            </p>
          ) : null}
        </div>
        {action ? (
          <div className="flex shrink-0 items-center gap-1">{action}</div>
        ) : null}
      </div>
      <Body onActivate={onBodyActivate} label={bodyLabel} tall={tall}>
        {state === "ready" ? children : null}
        {state === "loading" ? <Loading /> : null}
        {state === "error" ? (
          <ComingSoon
            variant="card"
            state="error"
            label={errorLabel}
            onRetry={onRetry}
          />
        ) : null}
      </Body>
    </Card>
  );
}

function Body({
  onActivate,
  label,
  tall,
  children,
}: {
  onActivate?: () => void;
  label?: string;
  tall: boolean;
  children: ReactNode;
}) {
  const layout = cn(
    "min-h-0 min-w-0 flex-1 overflow-auto px-5 pb-[23px]",
    tall
      ? "[scrollbar-width:thin] [scrollbar-color:var(--border)_transparent] pt-0"
      : "pt-2"
  );

  if (!onActivate) return <div className={layout}>{children}</div>;

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Enter" && event.key !== " ") return;

    event.preventDefault();
    onActivate();
  };

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={label}
      className={cn(
        layout,
        "cursor-pointer rounded-b-[12px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
      )}
      onClick={onActivate}
      onKeyDown={onKeyDown}
    >
      {children}
    </div>
  );
}

function Loading() {
  return (
    <div role="status" className="flex h-full items-center justify-center">
      <span className="sr-only">Loading</span>
      <div
        aria-hidden="true"
        className="flex h-24 w-full max-w-60 items-end gap-2"
      >
        {SKELETON_BARS.map((height, index) => (
          <Skeleton
            key={index}
            className="flex-1 rounded-t-[5px] rounded-b-none"
            style={{ height: `${height}%` }}
          />
        ))}
      </div>
    </div>
  );
}

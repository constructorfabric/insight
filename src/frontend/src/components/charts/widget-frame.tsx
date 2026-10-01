import type { KeyboardEvent, ReactNode } from "react";
import { AlertCircle, Grid2X2, RotateCcw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { TEXT_HEADING } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

export type WidgetState = "ready" | "loading" | "empty" | "error";

export type WidgetSize = "standard" | "tall";

const WIDGET_HEIGHT: Record<WidgetSize, number> = {
  standard: 304,
  tall: 464,
};

const SKELETON_BARS = [40, 65, 45, 80, 55, 90, 70];

export interface WidgetFrameProps {
  title: ReactNode;
  subtitle?: ReactNode;
  action?: ReactNode;
  state: WidgetState;
  emptyLabel?: string;
  errorLabel?: string;
  onRetry?: () => void;
  onBodyActivate?: () => void;
  scroll?: boolean;
  size?: WidgetSize;
  bodyLabel?: string;
  className?: string;
  children: ReactNode;
}

export function WidgetFrame({
  title,
  subtitle,
  action,
  state,
  emptyLabel = "No data in this window.",
  errorLabel = "Could not load this widget.",
  onRetry,
  onBodyActivate,
  scroll = false,
  size = "standard",
  bodyLabel,
  className,
  children,
}: WidgetFrameProps) {
  return (
    <Card
      className={cn(
        "min-w-0 gap-0 rounded-[12px] border border-border py-0 shadow-[0_2px_3px_#12204805] ring-0",
        className
      )}
      style={{ height: WIDGET_HEIGHT[size] }}
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
      <Body onActivate={onBodyActivate} label={bodyLabel} scroll={scroll}>
        {state === "ready" ? (
          children
        ) : (
          <StateBody
            state={state}
            emptyLabel={emptyLabel}
            errorLabel={errorLabel}
            onRetry={onRetry}
          />
        )}
      </Body>
    </Card>
  );
}

function Body({
  onActivate,
  label,
  scroll,
  children,
}: {
  onActivate?: () => void;
  label?: string;
  scroll: boolean;
  children: ReactNode;
}) {
  const layout = cn(
    "min-h-0 min-w-0 flex-1 overflow-auto px-5 pb-[23px]",
    scroll
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

function StateBody({
  state,
  emptyLabel,
  errorLabel,
  onRetry,
}: {
  state: Exclude<WidgetState, "ready">;
  emptyLabel: string;
  errorLabel: string;
  onRetry?: () => void;
}) {
  const layout =
    "flex h-full flex-col items-center justify-center gap-2 text-center text-xs text-muted-foreground [&>svg]:size-5";

  if (state === "loading") {
    return (
      <div role="status" className={layout}>
        <span className="sr-only">Loading</span>
        <div
          aria-hidden="true"
          className="flex h-24 w-full max-w-60 items-end gap-2"
        >
          {SKELETON_BARS.map((height, index) => (
            <i
              key={index}
              className="flex-1 animate-pulse rounded-t-[5px] bg-muted"
              style={{ height: `${height}%` }}
            />
          ))}
        </div>
      </div>
    );
  }

  if (state === "empty") {
    return (
      <div role="status" className={layout}>
        <Grid2X2 aria-hidden="true" />
        <span>{emptyLabel}</span>
      </div>
    );
  }

  return (
    <div className={layout}>
      <AlertCircle aria-hidden="true" />
      <span role="alert" className="text-foreground">
        {errorLabel}
      </span>
      {onRetry ? (
        <Button
          variant="outline"
          size="sm"
          onClick={(event) => {
            event.stopPropagation();
            onRetry();
          }}
        >
          <RotateCcw />
          Retry
        </Button>
      ) : null}
    </div>
  );
}

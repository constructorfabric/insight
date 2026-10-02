import { useState, type KeyboardEvent, type ReactNode } from "react";
import { Maximize2, Minimize2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
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
  fullscreen?: boolean;
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
  fullscreen = false,
  className,
  children,
}: WidgetFrameProps) {
  const [expanded, setExpanded] = useState(false);

  const content = (
    <>
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
    </>
  );

  return (
    <Card
      className={cn(
        "min-w-0 gap-0 rounded-[12px] border border-border py-0 shadow-[0_2px_3px_#12204805] ring-0",
        className
      )}
      style={{ height: tall ? TALL_HEIGHT : STANDARD_HEIGHT }}
    >
      <Header
        title={<h3 className={HEADING}>{title}</h3>}
        subtitle={subtitle}
        action={action}
        toggle={
          fullscreen ? (
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Full screen"
              className="text-muted-foreground"
              onClick={() => setExpanded(true)}
            >
              <Maximize2 />
            </Button>
          ) : null
        }
      />
      <Body onActivate={onBodyActivate} label={bodyLabel} tall={tall}>
        {expanded ? null : content}
      </Body>

      {fullscreen ? (
        <Dialog open={expanded} onOpenChange={setExpanded}>
          <DialogContent className="flex h-[96vh] w-[98vw] max-w-[98vw] flex-col gap-0 p-0">
            <Header
              title={
                <DialogTitle className={cn(HEADING, "pt-3")}>
                  {title}
                </DialogTitle>
              }
              subtitle={subtitle}
              action={
                action ? (
                  <div
                    onClickCapture={() => setExpanded(false)}
                    className="contents"
                  >
                    {action}
                  </div>
                ) : null
              }
              toggle={
                <Button
                  variant="ghost"
                  size="icon"
                  className="w-9 text-muted-foreground"
                  aria-label="Exit full screen"
                  onClick={() => setExpanded(false)}
                >
                  <Minimize2 />
                </Button>
              }
              className="min-h-0 pe-14 pt-4"
              controlsClassName="h-9"
            />
            <Body
              onActivate={
                onBodyActivate
                  ? () => {
                      setExpanded(false);
                      onBodyActivate();
                    }
                  : undefined
              }
              label={bodyLabel}
              tall={tall}
            >
              {content}
            </Body>
          </DialogContent>
        </Dialog>
      ) : null}
    </Card>
  );
}

const HEADING = cn(TEXT_HEADING, "truncate leading-tight");

function Header({
  title,
  subtitle,
  action,
  toggle,
  className,
  controlsClassName,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  action?: ReactNode;
  toggle?: ReactNode;
  className?: string;
  controlsClassName?: string;
}) {
  return (
    <div
      className={cn(
        "flex min-h-[76px] shrink-0 items-start justify-between gap-2.5 px-6 pt-[23px] pb-3.5",
        className
      )}
    >
      <div className="min-w-0">
        {title}
        {subtitle ? (
          <p className="mt-1.5 line-clamp-2 max-w-[440px] text-xs leading-normal text-muted-foreground">
            {subtitle}
          </p>
        ) : null}
      </div>
      {action || toggle ? (
        <div
          className={cn("flex shrink-0 items-center gap-1", controlsClassName)}
        >
          {action}
          {toggle}
        </div>
      ) : null}
    </div>
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

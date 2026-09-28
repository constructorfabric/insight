import { useState, type ReactNode } from "react";
import { PanelRightClose, Sparkles } from "lucide-react";

import { Button } from "@/components/ui/button";

const HIDDEN_KEY = "insight.custom.assistant-hidden";

/**
 * Two columns inside the portal inset: the dashboards scroll on the left, the
 * assistant keeps a fixed column on the right. Below `lg` the panel drops under
 * the content instead of squeezing it — one chat instance either way, so the
 * thread survives the breakpoint.
 *
 * The panel can be put away, because a dashboard being shown to a room does not
 * need a chat box beside it. The chat stays mounted while hidden, so the thread
 * is still there when it comes back, and the choice is remembered per browser.
 */
export function CustomPageShell({
  children,
  chat,
}: {
  children: ReactNode;
  chat: ReactNode;
}) {
  const [hidden, setHidden] = useState(remembered);

  function toggle() {
    setHidden((wasHidden) => {
      const next = !wasHidden;
      try {
        window.localStorage.setItem(HIDDEN_KEY, next ? "1" : "0");
      } catch {
        // Storage blocked: the toggle still works, it just is not remembered.
      }
      return next;
    });
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
      <div className="@container relative min-w-0 flex-1 overflow-y-auto p-4 md:p-6">
        {children}
      </div>
      {hidden ? (
        <div className="flex shrink-0 justify-center border-t bg-sidebar p-2 lg:w-12 lg:border-t-0 lg:border-s">
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Open assistant"
            aria-expanded={false}
            className="text-muted-foreground"
            onClick={toggle}
          >
            <Sparkles />
          </Button>
        </div>
      ) : null}
      <div
        hidden={hidden}
        className="relative flex min-h-96 shrink-0 flex-col lg:min-h-0 lg:w-80"
      >
        {/* Open, it belongs in the panel's own header, beside its title. */}
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Collapse assistant"
          aria-expanded
          className="absolute end-2 top-2 z-10 text-muted-foreground"
          onClick={toggle}
        >
          <PanelRightClose />
        </Button>
        {chat}
      </div>
    </div>
  );
}

function remembered(): boolean {
  try {
    return window.localStorage.getItem(HIDDEN_KEY) !== "0";
  } catch {
    return true;
  }
}

/**
 * Jump to either end of a long list.
 *
 * A list that loads as it scrolls has no end in sight and no scrollbar worth
 * aiming at, so getting back to the top costs a lot of wheel. Shown only once
 * the list is long enough for that to be true — measured in rows rather than
 * pixels, so the control appears for the same list on every screen.
 */
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Rows past which scrolling back by hand stops being reasonable. */
export const SCROLL_ENDS_AFTER_ROWS = 25;

export function ScrollToEnds({
  scroller,
  rows,
  className,
}: {
  scroller: React.RefObject<HTMLElement | null>;
  /** Rows currently listed — what decides whether the control is warranted. */
  rows: number;
  className?: string;
}) {
  const { t } = useTranslation();
  if (rows <= SCROLL_ENDS_AFTER_ROWS) return null;

  function jump(to: "top" | "bottom"): void {
    const node = scroller.current;
    if (node === null) return;
    node.scrollTo({
      top: to === "top" ? 0 : node.scrollHeight,
      behavior: "smooth",
    });
  }

  return (
    // Floating over the list's own bottom corner: in flow it would sit past the
    // last row, which on a list that keeps growing is nowhere the reader is.
    <div
      className={cn(
        "pointer-events-none absolute end-4 bottom-4 z-10 flex flex-col gap-1",
        className,
      )}
    >
      <Button
        type="button"
        variant="outline"
        size="icon"
        className="pointer-events-auto size-8 shadow-sm"
        aria-label={t("common.actions.scroll_to_top")}
        onClick={() => jump("top")}
      >
        <ArrowUp className="size-4" />
      </Button>
      <Button
        type="button"
        variant="outline"
        size="icon"
        className="pointer-events-auto size-8 shadow-sm"
        aria-label={t("common.actions.scroll_to_bottom")}
        onClick={() => jump("bottom")}
      >
        <ArrowDown className="size-4" />
      </Button>
    </div>
  );
}

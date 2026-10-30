import { Tag } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";

import { fittingCount } from "@/lib/custom/fitting-tags";
import { cn } from "@/lib/utils";

const MOST_SHOWN_TAGS = 3;

const CHIP =
  "inline-flex h-5 shrink-0 items-center gap-1 rounded-sm border px-1.5 text-xs whitespace-nowrap text-muted-foreground";

const OFF_ROW = "invisible absolute top-0 left-0";

/** A chip's width with its label untruncated, even while it is squeezed. */
function naturalWidth(chip: HTMLElement): number {
  const label = chip.querySelector<HTMLElement>("[data-label]");
  if (!label) return chip.offsetWidth;

  return chip.offsetWidth + label.scrollWidth - label.clientWidth;
}

function useFittingCount(candidates: string[], overflowing: boolean) {
  const row = useRef<HTMLUListElement>(null);
  const [fitting, setFitting] = useState(candidates.length);
  const shape = `${candidates.join("\u0000")}\u0000${overflowing}`;

  useLayoutEffect(() => {
    const list = row.current;
    if (!list) return;

    const fit = () => {
      const widths = Array.from(list.children, (chip) =>
        naturalWidth(chip as HTMLElement)
      );
      const moreWidth = widths.pop() ?? 0;

      setFitting(
        fittingCount({
          widths,
          available: list.clientWidth,
          gap: Number.parseFloat(getComputedStyle(list).columnGap) || 0,
          moreWidth,
          overflowing,
        })
      );
    };

    fit();
    if (typeof ResizeObserver === "undefined") return;

    const observer = new ResizeObserver(fit);
    observer.observe(list);
    return () => observer.disconnect();
    // INVARIANT: `shape` stands in for `candidates` and `overflowing`, a new array every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shape]);

  return { row, fitting };
}

export function DashboardTags({ tags }: { tags: string[] }) {
  const candidates = tags.slice(0, MOST_SHOWN_TAGS);
  const { row, fitting } = useFittingCount(
    candidates,
    tags.length > MOST_SHOWN_TAGS
  );

  if (tags.length === 0) return null;

  const rest = tags.slice(fitting);

  return (
    <ul
      ref={row}
      aria-label="Tags"
      className="relative flex min-w-0 gap-1.5 overflow-hidden"
    >
      {candidates.map((tag, index) => {
        const shown = index < fitting;

        return (
          <li
            key={tag}
            aria-hidden={shown ? undefined : true}
            className={cn(
              CHIP,
              shown ? index === 0 && "min-w-0 shrink" : OFF_ROW
            )}
          >
            <Tag className="size-3 shrink-0" aria-hidden />
            <span data-label className="truncate">
              {tag}
            </span>
          </li>
        );
      })}
      {rest.length > 0 ? (
        <li className={CHIP} title={rest.join(", ")}>
          <span aria-hidden="true">+{rest.length}</span>
          <span className="sr-only">
            {`${rest.length} more: ${rest.join(", ")}`}
          </span>
        </li>
      ) : (
        <li aria-hidden="true" className={cn(CHIP, OFF_ROW)}>
          +{tags.length}
        </li>
      )}
    </ul>
  );
}

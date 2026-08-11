import { ChevronRight } from "lucide-react";
import { useState } from "react";

import { Card, CardContent } from "@/components/ui/card";
import { MetricHelpTooltip } from "@/components/widgets/metric-help-tooltip";
import { useSettings } from "@/hooks/use-settings";
import type { AttentionItem } from "@/lib/insight/attention";
import type { GroupId } from "@/lib/insight/groups";
import { PEER_TEXT, applyFocus } from "@/lib/peers";
import { TEXT_EYEBROW, TEXT_LABEL, TEXT_NAME } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const COLLAPSED_ATTENTION = 6;
const COLLAPSE_THRESHOLD = 7;

export interface IcNeedsAttentionProps {
  items: AttentionItem[];
  onOpenGroup: (id: GroupId) => void;
}

/**
 * Cross-group "needs attention" surface. Items arrive precomputed from the
 * per-source selectors in `lib/insight/attention.ts`; this component only
 * ranks (relGap descending), collapses, and renders.
 */
export function IcNeedsAttention({
  items,
  onOpenGroup,
}: IcNeedsAttentionProps) {
  const { focusMode } = useSettings();
  const [showAll, setShowAll] = useState(false);

  // Furthest outside the cohort's own spread first, percent-of-median only to
  // break a tie. Ordering by percentage put a metric whose cohort median is
  // two above one whose median is two hundred, for the same one-unit slip.
  // Ordering and deduplication belong with the rule that produced the items
  // (`orderAttentionItems`), not with the view: which of two rows says a thing
  // more strongly is a question about the finding, not about the pixels.
  const attentionAll = items;

  if (attentionAll.length === 0) return null;

  const shouldCollapse = attentionAll.length >= COLLAPSE_THRESHOLD;
  const visible =
    !shouldCollapse || showAll
      ? attentionAll
      : attentionAll.slice(0, COLLAPSED_ATTENTION);
  const badStatus = applyFocus("bottom", focusMode);

  return (
    <section>
      <h2 className={cn("mb-3", TEXT_EYEBROW)}>
        Needs attention
      </h2>
      <Card data-size="sm">
        <CardContent className="text-sm">
          {/* One column, not two. Six columns of content inside half the
              content width truncated the two things a row exists to say — the
              metric's name and its unit — and a name cut to "Messages per
              Activ…" is not a shorter name. The block rarely runs long enough
              for the height to matter. */}
          {/* The COLUMNS belong to the list, not to a row: sized per row, they
              lined up only with themselves, and four rows put four values at
              four different x-positions. One grid here, and each row a subgrid
              of it, so the widths are decided once by the widest content and
              every row obeys them — while the row stays a real button with its
              own padding and hover. */}
          <ul className="flex flex-col gap-y-1 sm:grid sm:grid-cols-[minmax(0,max-content)_auto_auto_auto_minmax(0,1fr)_auto]">
            {visible.map((item) => (
              <li
                key={`${item.group}-${item.key}`}
                className="sm:col-span-full sm:grid sm:grid-cols-subgrid"
              >
                <MetricHelpTooltip help={item.help}>
                  <button
                    type="button"
                    onClick={() => onOpenGroup(item.group)}
                    /* Content-sized columns, not fixed ones. Fixed widths
                       decide what dies under width pressure before knowing
                       what is in them: the unit column held its full width
                       while metric names collapsed to "Em…" and the
                       comparison — the number the row exists for — was
                       squeezed to nothing. Only the comparison flexes now, so
                       it is the last thing to be cut, and the name and the
                       value keep the width they need.

                       Below `sm` the row becomes two lines instead: at 390px
                       there is no arrangement of six columns that leaves a
                       name legible. */
                    className="-mx-2 flex w-[calc(100%+1rem)] flex-col gap-x-2 rounded px-2 py-1 text-left text-sm transition-colors hover:bg-accent sm:col-span-full sm:grid sm:w-auto sm:grid-cols-subgrid sm:items-baseline"
                  >
                    <span className="flex min-w-0 items-baseline gap-2 sm:contents">
                      <span className={cn("min-w-0 truncate", TEXT_NAME)}>
                        {item.label}
                      </span>
                      {/* Beside the name, not stranded at the far edge of the
                          row: it says what KIND of finding this name is, and
                          half a row away it was read as a fifth number. */}
                      <span className={cn("shrink-0 justify-self-start rounded border px-1.5 whitespace-nowrap", TEXT_LABEL)}>
                        {item.kind === "fell"
                          ? "fell this period"
                          : item.noPrevious
                            ? "no earlier period"
                            : "ongoing"}
                      </span>
                    </span>
                    <span className="flex items-baseline gap-2 sm:contents">
                      {/* Digits right, unit left: the numbers line up on their
                          last figure AND the units start together, so neither
                          column is ragged. One cell of "143 lines" could only
                          give one of the two. */}
                      <span
                        className={cn(
                          "text-right tabular-nums sm:justify-self-end",
                          PEER_TEXT[badStatus]
                        )}
                      >
                        {item.valueNumber}
                      </span>
                      <span
                        className={cn(
                          "text-left whitespace-nowrap",
                          PEER_TEXT[badStatus]
                        )}
                      >
                        {item.valueUnit}
                      </span>
                      <span className={cn("truncate whitespace-nowrap tabular-nums", TEXT_LABEL)}>
                        {item.medianText ? (
                          <>
                            {item.gapText ? <>{item.gapText} vs </> : null}
                            median {item.medianText}
                          </>
                        ) : null}
                      </span>
                    </span>
                    {/* Same standing affordance as every other openable surface:
                        a row that only reacts to hover is indistinguishable from
                        a line of text until the mouse happens to cross it. */}
                    <ChevronRight
                      className="size-3.5 shrink-0 self-center text-muted-foreground"
                      aria-hidden
                    />
                  </button>
                </MetricHelpTooltip>
              </li>
            ))}
            {shouldCollapse ? (
              <li className="sm:col-span-full">
                <button
                  type="button"
                  onClick={() => setShowAll((v) => !v)}
                  className={cn(TEXT_LABEL, "rounded transition-colors hover:text-foreground")}
                >
                  {showAll
                    ? "Show fewer"
                    : `Show ${attentionAll.length - COLLAPSED_ATTENTION} more`}
                </button>
              </li>
            ) : null}
          </ul>
        </CardContent>
      </Card>
    </section>
  );
}

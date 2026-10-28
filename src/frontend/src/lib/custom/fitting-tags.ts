export interface TagRow {
  widths: number[];
  available: number;
  gap: number;
  moreWidth: number;
  /** Whether tags beyond `widths` exist, so "+N" shows even when they all fit. */
  overflowing: boolean;
}

function rowWidth(widths: number[], gap: number): number {
  if (widths.length === 0) return 0;

  const chips = widths.reduce((sum, width) => sum + width, 0);
  return chips + gap * (widths.length - 1);
}

/**
 * How many whole chips fit on one row beside the "+N" chip. Never fewer than
 * one, so a row too narrow for any whole chip still names a tag, truncated.
 */
export function fittingCount({
  widths,
  available,
  gap,
  moreWidth,
  overflowing,
}: TagRow): number {
  if (widths.length === 0) return 0;
  if (!overflowing && rowWidth(widths, gap) <= available) return widths.length;

  let used = moreWidth;
  let count = 0;
  for (const width of widths) {
    used += gap + width;
    if (used > available) break;
    count += 1;
  }

  return Math.max(1, count);
}

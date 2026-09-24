/** Which column a table is ordered by, and which way. */
export interface RowOrder {
  column: number;
  direction: "ascending" | "descending";
}

/**
 * The next state of a header clicked three times: up, down, back to the order
 * the metric returned.
 */
export function nextOrder(
  held: RowOrder | undefined,
  column: number
): RowOrder | undefined {
  if (held?.column !== column) return { column, direction: "ascending" };
  if (held.direction === "ascending") return { column, direction: "descending" };

  return undefined;
}

/**
 * The rows in the asked-for order, leaving the given array alone.
 *
 * INVARIANT: ordering happens before any row cap, so a sorted table shows the
 * top of the whole result and not the top of the first page of it.
 */
export function ordered(
  rows: readonly unknown[][],
  order: RowOrder | undefined
): readonly unknown[][] {
  if (order === undefined) return rows;

  const sign = order.direction === "ascending" ? 1 : -1;

  return [...rows].sort((left, right) =>
    compare(left[order.column], right[order.column], sign)
  );
}

/** Nothing to compare sorts last, whichever way the column is pointing. */
function absent(value: unknown): boolean {
  return value === undefined || value === null || value === "";
}

function compare(left: unknown, right: unknown, sign: number): number {
  if (absent(left) || absent(right)) {
    if (absent(left) && absent(right)) return 0;
    return absent(left) ? 1 : -1;
  }
  if (typeof left === "number" && typeof right === "number") {
    return sign * (left - right);
  }

  return (
    sign *
    String(left).localeCompare(String(right), undefined, { numeric: true })
  );
}

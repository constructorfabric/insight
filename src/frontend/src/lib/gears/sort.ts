export type SortDirection = "asc" | "desc";

export interface SortState<Key extends string> {
  key: Key;
  direction: SortDirection;
}

/**
 * The next sort state for a header click. A column cycles through three
 * states — largest first, smallest first, then off — so a reader can put a
 * table back the way they found it with the control they already used, rather
 * than hunting for a reset.
 */
export function nextSort<Key extends string>(
  sort: SortState<Key> | null,
  key: Key,
): SortState<Key> | null {
  if (sort === null || sort.key !== key) return { key, direction: "desc" };
  if (sort.direction === "desc") return { key, direction: "asc" };

  return null;
}

/**
 * Rows ordered by one column. Absent values sort last in both directions:
 * "nobody estimated this" is not a small number, and burying it under real
 * ones would read as if it were.
 */
export function sortRows<Row, Key extends string>(
  rows: readonly Row[],
  sort: SortState<Key>,
  valueOf: (row: Row, key: Key) => string | number | null | undefined,
): Row[] {
  const factor = sort.direction === "asc" ? 1 : -1;

  return [...rows].sort((left, right) => {
    const a = valueOf(left, sort.key);
    const b = valueOf(right, sort.key);

    if (a == null && b == null) return 0;
    if (a == null) return 1;
    if (b == null) return -1;

    if (typeof a === "number" && typeof b === "number") {
      return (a - b) * factor;
    }

    return String(a).localeCompare(String(b)) * factor;
  });
}

export function ariaSort(
  sort: SortState<string> | null,
  key: string,
): "ascending" | "descending" | "none" {
  if (sort === null || sort.key !== key) return "none";

  return sort.direction === "asc" ? "ascending" : "descending";
}

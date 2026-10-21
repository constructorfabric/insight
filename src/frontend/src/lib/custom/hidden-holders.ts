/** How many holders the caller cannot see, as one line under the ones they can. */
export function hiddenHolders(hidden: number): string | null {
  if (hidden <= 0) return null;
  if (hidden === 1) return "and 1 dashboard you can't see";

  return `and ${hidden} dashboards you can't see`;
}

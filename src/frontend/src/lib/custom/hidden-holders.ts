/** How many holders the caller cannot see, as one line under the ones they can. */
export function hiddenHolders(hidden: number, visible: number): string | null {
  if (hidden <= 0) return null;

  const count = hidden === 1 ? "1 dashboard" : `${hidden} dashboards`;
  const line = `${count} you can't see`;

  return visible > 0 ? `and ${line}` : line;
}

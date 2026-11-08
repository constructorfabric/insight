export function nextInWalk(
  walk: readonly string[],
  from: string
): string | null {
  const at = walk.indexOf(from);
  return at === -1 ? null : (walk[at + 1] ?? null);
}

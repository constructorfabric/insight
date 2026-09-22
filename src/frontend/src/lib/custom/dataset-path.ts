/**
 * A declared field's key path, split into the segments that address it.
 *
 * INVARIANT: this mirrors the service's own `path_segments`. A payload key is
 * free to hold a dot, so a path escapes one with a backslash; splitting on a
 * plain `.` here would show a reader an empty cell for a field every metric
 * over the same declaration reads a value from.
 */
export function pathSegments(path: string): string[] {
  const segments: string[] = [];
  let current = "";
  let escaped = false;

  for (const character of path) {
    if (escaped) {
      current += character;
      escaped = false;
    } else if (character === "\\") {
      escaped = true;
    } else if (character === ".") {
      segments.push(current);
      current = "";
    } else {
      current += character;
    }
  }
  segments.push(current);

  return segments;
}

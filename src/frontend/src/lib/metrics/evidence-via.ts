/**
 * Which records explain a figure, where the figure's own measure cannot.
 *
 * A line count is a per-day summary in the warehouse, so its own evidence is
 * a table of days — which answers "when" and never "what changed". The lines
 * are carried by commits, and those are the records a reader is asking for
 * when they click the tile. A key listed here drills into the metric named
 * beside it, carrying whatever the click narrowed to; anything absent drills
 * into itself.
 */
const CARRIED_BY: Record<string, string> = {
  "git.code_lines": "git.commits",
  "git.lines_added": "git.commits",
  "git.lines_removed": "git.commits",
  "git.default_branch_code_lines": "git.default_branch_commits",
  "git.default_branch_lines_added": "git.default_branch_commits",
  "git.default_branch_lines_removed": "git.default_branch_commits",
  "git.non_default_branch_code_lines": "git.non_default_branch_commits",
  "git.non_default_branch_lines_added": "git.non_default_branch_commits",
  "git.non_default_branch_lines_removed": "git.non_default_branch_commits",
};

/** The metric whose records explain `key` — `key` itself unless listed. */
export function evidenceMetricFor(key: string): string {
  return CARRIED_BY[key] ?? key;
}

/** Every metric a lens must ask for so its figures stay drillable. */
export function evidenceCarriers(keys: readonly string[]): string[] {
  const carriers = keys
    .map((key) => CARRIED_BY[key])
    .filter((carrier): carrier is string => carrier != null);
  return [...new Set(carriers)];
}

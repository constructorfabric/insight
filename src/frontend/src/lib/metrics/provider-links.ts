/**
 * Links into the provider a metric's record came from — git records and
 * tracker issues alike.
 *
 * Only GitHub, and only because its web layout is derivable from data we
 * already hold: `https://github.com/{owner}/{repo}`. GitLab and Bitbucket can
 * be self-hosted at any address, and nothing in a metric row says which — so
 * they get no link rather than a guessed one.
 *
 * KNOWN GAP: the `source` dimension names the CONNECTOR, not the host, so a
 * GitHub Enterprise tenant reports `github` while living at its own domain.
 * Those links will point at github.com and miss. Carrying the host (or the
 * `html_url` the API already returns) is what fixes it properly.
 */

import type { MetricEvidenceSelection } from "@/api/metric-drilldown-client";

/**
 * GitHub's `source` dimension, as EITHER serving path spells it: a breakdown
 * row carries the connector key (`github`), while a drilldown row carries the
 * display label the same dimension resolves to (`GitHub`) — it projects a
 * dimension's label in preference to its value. Both name one connector, so
 * both are accepted rather than making callers know which path they are on.
 */
const GITHUB_SOURCE = "github";

const GITHUB_WEB_BASE = "https://github.com";

/**
 * `owner/repo`, and nothing that only looks like it: gold builds the label from
 * `project_key/repo_slug`, and an absent project leaves the slug alone (no
 * slash) or the `Unknown` placeholder — neither addresses a repository.
 */
const REPO_PATH = /^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/;

/** The repository's page, or null when the row is not linkable. */
export function githubRepoUrl(
  source: string | null | undefined,
  repository: string | null | undefined
): string | null {
  if (source?.trim().toLowerCase() !== GITHUB_SOURCE) return null;
  const path = repository?.trim();
  if (!path || !REPO_PATH.test(path)) return null;
  return `${GITHUB_WEB_BASE}/${path}`;
}

/**
 * A git commit hash is fixed-length hex (SHA-1: 40, SHA-256: 64); a PR number
 * is a short plain decimal. The two shapes never collide — a repo would need
 * 10^40 PRs — so the ref alone says which URL a record's own page needs,
 * without a `record_kind` column the evidence row does not carry.
 */
const COMMIT_REF = /^[0-9a-f]{40}$|^[0-9a-f]{64}$/i;
const PULL_REF = /^[0-9]+$/;

/** The record's own page under a repository, or null when `ref` is neither shape. */
export function githubRecordUrl(
  repoUrl: string,
  ref: string | null | undefined
): string | null {
  const value = ref?.trim();
  if (!value) return null;
  if (COMMIT_REF.test(value)) return `${repoUrl}/commit/${value}`;
  if (PULL_REF.test(value)) return `${repoUrl}/pull/${value}`;
  return null;
}

/** Two-part `family.name` metric keys namespace by family; these are fixed. */
const GIT_METRIC_PREFIX = "git.";
const TASK_METRIC_PREFIX = "tasks.";

/** Whether links are worth attempting at all for this metric's evidence. */
export function isGitMetric(metricKey: string): boolean {
  return metricKey.startsWith(GIT_METRIC_PREFIX);
}

export function isTaskMetric(metricKey: string): boolean {
  return metricKey.startsWith(TASK_METRIC_PREFIX);
}

/**
 * A tracker states an issue's readable key its own way — GitHub as
 * `owner/repo#12`, Jira as `PROJ-7` — and only the GitHub shape carries the
 * repository the URL needs. Matching the shape is what keeps a Jira key from
 * being linked to a github.com path that does not exist.
 */
const GITHUB_ISSUE_REF = /^([A-Za-z0-9._-]+\/[A-Za-z0-9._-]+)#([0-9]+)$/;

/** The issue's own page, or null when the row is not linkable. */
export function githubIssueUrl(
  source: string | null | undefined,
  ref: string | null | undefined
): string | null {
  if (source?.trim().toLowerCase() !== GITHUB_SOURCE) return null;
  const matched = GITHUB_ISSUE_REF.exec(ref?.trim() ?? "");
  if (!matched) return null;
  return `${GITHUB_WEB_BASE}/${matched[1]}/issues/${matched[2]}`;
}

/**
 * How an issue ref reads in a column: the repository prefix is the same for
 * nearly every row of one drilldown, so it costs width and says nothing —
 * `#12` is what distinguishes rows, and the repository stays in the link and
 * in what the copy button yields. A ref of any other shape reads as-is.
 */
export function evidenceRefText(
  metricKey: string,
  value: string
): string {
  if (!isTaskMetric(metricKey)) return value;
  const matched = GITHUB_ISSUE_REF.exec(value.trim());
  return matched ? `#${matched[2]}` : value;
}

/** The dimension key requested so a row's own connector rides along. */
export const SOURCE_DIMENSION = "source";

/**
 * Adds `source` to a metric's requested display dimensions, so its
 * evidence rows carry the per-row connector a link needs to be safe.
 *
 * SAFETY: only when the metric DECLARES the dimension — a drilldown that asks
 * for an undeclared one is rejected outright, so asking on spec would trade a
 * missing link for an unopenable dialog. `declared` is null while the catalog
 * is unknown, which is also a no-op.
 */
export function withSourceDimension(
  selection: MetricEvidenceSelection,
  declared: ReadonlySet<string> | null | undefined
): MetricEvidenceSelection {
  if (!isGitMetric(selection.metric_key) && !isTaskMetric(selection.metric_key)) {
    return selection;
  }
  if (!declared?.has(SOURCE_DIMENSION)) return selection;
  if (selection.display_dimensions.includes(SOURCE_DIMENSION)) return selection;
  return {
    ...selection,
    display_dimensions: [
      ...selection.display_dimensions,
      SOURCE_DIMENSION,
    ].sort(),
  };
}

/**
 * Where each of a drilldown row's columns links to, by column key — empty for
 * every row nothing can be said about. The id of a record and the human-readable
 * summary of it address the same page, so both carry it.
 */
export function evidenceRecordLinks(
  metricKey: string,
  values: Readonly<Record<string, unknown>>
): Readonly<Record<string, string | undefined>> {
  if (isTaskMetric(metricKey)) {
    const issue = githubIssueUrl(
      asString(values[SOURCE_DIMENSION]),
      asString(values.ref)
    );
    return issue ? { ref: issue, title: issue } : {};
  }
  if (!isGitMetric(metricKey)) return {};
  const repoUrl = githubRepoUrl(
    asString(values[SOURCE_DIMENSION]),
    asString(values.repository)
  );
  if (!repoUrl) return {};
  const record = githubRecordUrl(repoUrl, asString(values.ref)) ?? undefined;
  return { repository: repoUrl, ref: record, title: record };
}

function asString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

/**
 * One line naming an issue in an activity list: `#12: what it is about`.
 *
 * The list has one column for the thing itself, so the number and the summary
 * share it — the number alone does not say what the work was, and the summary
 * alone cannot be told apart from another issue with a similar name. Either
 * half may be missing; what is left still reads.
 */
export function activityEventLabel(
  metricKey: string,
  ref: string | null | undefined,
  title: string | null | undefined
): string | null {
  const shortRef = ref ? evidenceRefText(metricKey, ref) : null;
  if (!isTaskMetric(metricKey)) return title ?? null;
  if (shortRef && title) return `${shortRef}: ${title}`;
  return title ?? shortRef ?? null;
}

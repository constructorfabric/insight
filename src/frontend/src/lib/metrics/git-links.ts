/**
 * Links into the provider a git metric came from.
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

/** Two-part `family.name` metric keys namespace by family; git's is fixed. */
const GIT_METRIC_PREFIX = "git.";

/** Whether links are worth attempting at all for this metric's evidence. */
export function isGitMetric(metricKey: string): boolean {
  return metricKey.startsWith(GIT_METRIC_PREFIX);
}

/** The dimension key requested so a row's own connector rides along. */
export const SOURCE_DIMENSION = "source";

/**
 * Adds `source` to a git metric's requested display dimensions, so its
 * evidence rows carry the per-row connector a link needs to be safe.
 *
 * SAFETY: only when the metric DECLARES the dimension — a drilldown that asks
 * for an undeclared one is rejected outright, so asking on spec would trade a
 * missing link for an unopenable dialog. `declared` is null while the catalog
 * is unknown, which is also a no-op.
 */
export function withGitSourceDimension(
  selection: MetricEvidenceSelection,
  declared: ReadonlySet<string> | null | undefined
): MetricEvidenceSelection {
  if (!isGitMetric(selection.metric_key)) return selection;
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

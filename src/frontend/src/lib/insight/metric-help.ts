/**
 * The catalog's own words for a metric.
 *
 * `description` says what the number is; `explanation` says how it is counted
 * and what it is worth reading for. Both are server-owned copy that rides on
 * every metric result — the screens simply had nowhere to put it, so a reader
 * met "Focus Time 88%" with no way to learn what either word meant.
 */
export interface MetricHelpText {
  description: string | null;
  explanation: string | null;
}

/** Source shape: any metric result or definition carries these two fields. */
interface MetricCopy {
  description?: string | null;
  explanation?: string | null;
}

function trimmed(value: string | null | undefined): string | null {
  const text = value?.trim();
  return text ? text : null;
}

/**
 * Help text for a metric, or null when the catalog supplies none — callers
 * render no tooltip at all rather than an empty bubble, which is worse than
 * no affordance: it promises an answer and gives blank space.
 */
export function metricHelp(metric: MetricCopy): MetricHelpText | null {
  const description = trimmed(metric.description);
  const explanation = trimmed(metric.explanation);
  if (!description && !explanation) return null;
  // Some definitions repeat one field in the other; showing it twice reads as
  // a stutter, not as detail.
  return {
    description,
    explanation: explanation === description ? null : explanation,
  };
}

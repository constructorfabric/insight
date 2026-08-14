/**
 * The line naming what a section was watching.
 *
 * Its value is entirely in what it refuses to say: a dimension that describes
 * the thing being counted is not a system, and a system that produced nothing
 * cannot be told apart from one nobody wired — so neither is claimed.
 */
import { describe, expect, it } from "vitest";

import type { MetricResult } from "@/api/metric-results-client";
import type { MetricGroup } from "@/lib/insight/groups";
import { sectionSources } from "@/lib/insight/section-sources";
import { normalizeMetricResults } from "@/lib/metrics/collection";

const ME = "019e27bc-dec0-7626-81a9-c5524662a6a9";

/** One metric with a breakdown over `dimension`, as the wire delivers it. */
function metric(
  key: string,
  dimension: string,
  rows: Array<[value: string, label: string | null, amount: number]>
): MetricResult {
  return {
    metric_key: key,
    label: key,
    unit: null,
    format: "integer",
    computation: "sum",
    direction: "higher_is_better",
    views: [
      {
        view: "breakdown",
        dimensions: [dimension],
        values: rows.map(([value, label, amount]) => ({
          entity_id: ME,
          dimensions: [{ key: dimension, value, label: label ?? undefined }],
          value: amount,
        })),
      },
    ],
  } as unknown as MetricResult;
}

function group(keys: string[]): MetricGroup {
  return {
    id: "collaboration",
    title: "Collaboration",
    collection: { metrics: keys.map((key) => ({ key, views: [] })) },
    card: { preview: [] },
    drilldown: [],
  } as unknown as MetricGroup;
}

describe("sectionSources", () => {
  it("names the systems behind the numbers, alphabetically and once each", () => {
    const results = [
      metric("collab.messages_sent", "tool", [
        ["zulip", "Zulip", 400],
        ["m365", "Microsoft Teams", 20],
      ]),
      metric("collab.meeting_hours", "tool", [
        ["zoom", "Zoom", 27],
        ["m365", "Microsoft Teams", 13],
      ]),
    ];
    expect(
      sectionSources(
        group(["collab.messages_sent", "collab.meeting_hours"]),
        normalizeMetricResults(results),
        ME
      )
    ).toEqual(["Microsoft Teams", "Zoom", "Zulip"]);
  });

  it("ignores dimensions that describe the work rather than the system", () => {
    // "Internal" and "external" are properties of a shared file, not products.
    // Naming them here would claim a connector that does not exist.
    const results = [
      metric("collab.files_shared", "scope", [
        ["internal", "Internal", 9],
        ["external", "External", 1],
      ]),
    ];
    expect(
      sectionSources(
        group(["collab.files_shared"]),
        normalizeMetricResults(results),
        ME
      )
    ).toEqual([]);
  });

  it("leaves out a system that produced nothing", () => {
    // A zero means either "this person did not use it" or "nobody wired it",
    // and this line cannot tell those apart — so it claims neither.
    const results = [
      metric("ai.accepted_lines", "tool", [
        ["claude_code", "Claude Code", 164],
        ["codex", "Codex", 0],
      ]),
    ];
    expect(
      sectionSources(
        group(["ai.accepted_lines"]),
        normalizeMetricResults(results),
        ME
      )
    ).toEqual(["Claude Code"]);
  });

  it("falls back to the raw value when the wire sends no label", () => {
    const results = [metric("git.commits", "source", [["gitlab", null, 14]])];
    expect(
      sectionSources(
        group(["git.commits"]),
        normalizeMetricResults(results),
        ME
      )
    ).toEqual(["gitlab"]);
  });

  it("says nothing when no metric carries a breakdown", () => {
    // A section whose metrics were fetched without a system split states no
    // sources rather than guessing from the metric keys.
    expect(sectionSources(group(["git.commits"]), new Map(), ME)).toEqual([]);
  });
});

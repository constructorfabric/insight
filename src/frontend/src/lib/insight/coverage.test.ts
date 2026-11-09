/**
 * What the product can see about a person, and about how many people.
 *
 * The tests that matter here are the ones about what it REFUSES to conclude:
 * that a source is missing because nobody in view uses it, that an unlinked
 * person is a person with nothing to show, or that connecting something would
 * light up a knowable number of people.
 */
import { describe, expect, it } from "vitest";

import type { MetricDefinition } from "@/api/metric-definitions-client";
import type { MetricResult } from "@/api/metric-results-client";
import type { MetricGroup } from "@/lib/insight/groups";
import {
  partState,
  reachableMetricKeys,
  unreachableParts,
} from "@/lib/insight/coverage";
import { normalizeMetricResults } from "@/lib/metrics/collection";

const ME = "019e27bc-dec0-7626-81a9-c5524662a6a9";
const SOMEONE_ELSE = "019e27bc-dec0-7626-81a9-000000000002";

/** A definition as the listing delivers it; only the health fields matter. */
function def(
  metric_key: string,
  over: Partial<MetricDefinition> = {},
): MetricDefinition {
  return {
    metric_key,
    is_enabled: true,
    origin: "builtin",
    schema_status: "ok",
    schema_error_code: null,
    last_observed_date: "2026-08-01",
    ...over,
  } as unknown as MetricDefinition;
}

/** One metric carrying a period value for the listed entities. */
function metric(
  key: string,
  values: Array<[entity: string, value: number | null]>,
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
        view: "period",
        values: values.map(([entity_id, value]) => ({ entity_id, value })),
      },
    ],
  } as unknown as MetricResult;
}

function group(id: string, keys: string[]): MetricGroup {
  return {
    id,
    title: `${id} title`,
    collection: { metrics: keys.map((key) => ({ key, views: [] })) },
    card: { preview: [] },
    drilldown: [],
  } as unknown as MetricGroup;
}

describe("reachableMetricKeys", () => {
  it("counts a metric that has observed something", () => {
    expect(reachableMetricKeys([def("git.commits")])).toEqual(
      new Set(["git.commits"]),
    );
  });

  it("drops a metric that has never observed anything", () => {
    // `last_observed_date: null` is the listing saying no data has ever
    // arrived — the source is declared but nothing came through it.
    expect(
      reachableMetricKeys([def("ai.accepted", { last_observed_date: null })]),
    ).toEqual(new Set());
  });

  it("drops a disabled or schema-broken metric", () => {
    expect(
      reachableMetricKeys([
        def("wiki.edits", { is_enabled: false }),
        def("task.closed", {
          schema_status: "error",
          schema_error_code: "table_not_found",
        }),
      ]),
    ).toEqual(new Set());
  });

  it("keeps a custom metric that has never stamped a freshness date", () => {
    // The validator stamps last_observed_date from materialized relations
    // only, and a custom metric runs its SQL at query time — so the field
    // stays absent however much data it serves. Reading that as "never
    // measured" would report a working metric as one nothing reaches, which is
    // the one thing this function must never do.
    expect(
      reachableMetricKeys([
        def("custom.thing", { origin: "custom", last_observed_date: null }),
      ]),
    ).toEqual(new Set(["custom.thing"]));
  });

  it("still drops a custom metric that is disabled or schema-broken", () => {
    expect(
      reachableMetricKeys([
        def("custom.off", { origin: "custom", is_enabled: false }),
        def("custom.broken", {
          origin: "custom",
          schema_status: "error",
          schema_error_code: "table_not_found",
        }),
      ]),
    ).toEqual(new Set());
  });

  it("keeps an unchecked schema — unchecked is not broken", () => {
    expect(
      reachableMetricKeys([def("ai.accepted", { schema_status: "unchecked" })]),
    ).toEqual(new Set(["ai.accepted"]));
  });
});

describe("partState", () => {
  const PART = group("collaboration", ["collab.messages", "collab.meetings"]);

  it("reads when any metric of the part has a value", () => {
    const byKey = normalizeMetricResults([metric("collab.messages", [[ME, 12]])]);
    expect(
      partState(PART, byKey, ME, reachableMetricKeys([def("collab.messages")])),
    ).toBe("reads");
  });

  it("says nothing recorded when the source reaches us but this person is absent from it", () => {
    const byKey = normalizeMetricResults([
      metric("collab.messages", [[ME, null]]),
    ]);
    expect(
      partState(PART, byKey, ME, reachableMetricKeys([def("collab.messages")])),
    ).toBe("nothing_recorded");
  });

  it("says no data reaches us when nothing in the part has ever observed anything", () => {
    const byKey = normalizeMetricResults([
      metric("collab.messages", [[ME, null]]),
    ]);
    const reachable = reachableMetricKeys([
      def("collab.messages", { last_observed_date: null }),
      def("collab.meetings", { last_observed_date: null }),
    ]);
    expect(partState(PART, byKey, ME, reachable)).toBe("no_data_reaches_us");
  });

  it("does NOT call a part unreachable because nobody in view uses it", () => {
    // The whole point. This viewer sees two people, neither of whom has a
    // value. The listing says the source has observed data for the tenant, so
    // the part is reachable and these two simply did none of that work.
    // Concluding otherwise would report a live connector as missing, and would
    // do so more often the smaller the viewer's reach.
    const byKey = normalizeMetricResults([
      metric("collab.messages", [
        [ME, null],
        [SOMEONE_ELSE, null],
      ]),
    ]);
    expect(
      partState(PART, byKey, ME, reachableMetricKeys([def("collab.messages")])),
    ).toBe("nothing_recorded");
  });

  it("still reads when only one of several metrics is wired", () => {
    // A part is not unobservable because one of its metrics is missing.
    const byKey = normalizeMetricResults([metric("collab.meetings", [[ME, 3]])]);
    const reachable = reachableMetricKeys([
      def("collab.meetings"),
      def("collab.messages", { last_observed_date: null }),
    ]);
    expect(partState(PART, byKey, ME, reachable)).toBe("reads");
  });
});

describe("unreachableParts", () => {
  const GROUPS = [
    group("git_output", ["git.commits", "git.prs"]),
    group("ai_adoption", ["ai.accepted"]),
  ];

  it("names a part no metric of which has ever observed anything", () => {
    const reachable = reachableMetricKeys([def("git.commits")]);
    expect(unreachableParts(GROUPS, reachable)).toEqual([
      { id: "ai_adoption", title: "ai_adoption title" },
    ]);
  });

  it("does not name a part where one metric of several reaches us", () => {
    const reachable = reachableMetricKeys([def("git.prs"), def("ai.accepted")]);
    expect(unreachableParts(GROUPS, reachable)).toEqual([]);
  });

  it("offers no estimate of who connecting one would reveal", () => {
    // The people who do that work are invisible BECAUSE the source is missing,
    // so any such number would be invented. The shape of the return value is
    // the guarantee: there is nowhere to put one.
    const [only] = unreachableParts(GROUPS, new Set(["git.commits"]));
    expect(Object.keys(only)).toEqual(["id", "title"]);
  });
});

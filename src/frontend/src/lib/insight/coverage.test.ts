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
  coverageDistribution,
  partCoverage,
  partState,
  personCoverage,
  reachableMetricKeys,
  thinlyCovered,
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

describe("personCoverage", () => {
  const GROUPS = [
    group("git_output", ["git.commits"]),
    group("collaboration", ["collab.messages"]),
    group("ai_adoption", ["ai.accepted"]),
  ];

  it("levels a person by how many parts read, and keeps each part's reason", () => {
    const byKey = normalizeMetricResults([
      metric("git.commits", [[ME, 4]]),
      metric("collab.messages", [[ME, null]]),
      metric("ai.accepted", [[ME, null]]),
    ]);
    const reachable = reachableMetricKeys([
      def("git.commits"),
      def("collab.messages"),
      def("ai.accepted", { last_observed_date: null }),
    ]);
    const cov = personCoverage(GROUPS, byKey, ME, reachable);

    expect(cov.level).toBe(1);
    expect(cov.states.get("git_output")).toBe("reads");
    expect(cov.states.get("collaboration")).toBe("nothing_recorded");
    expect(cov.states.get("ai_adoption")).toBe("no_data_reaches_us");
  });

  it("gives level zero without inventing a reason for it", () => {
    const byKey = normalizeMetricResults([metric("git.commits", [[ME, null]])]);
    const cov = personCoverage(GROUPS, byKey, ME, new Set<string>());
    expect(cov.level).toBe(0);
    expect([...cov.states.values()]).toEqual([
      "no_data_reaches_us",
      "no_data_reaches_us",
      "no_data_reaches_us",
    ]);
  });
});

describe("coverageDistribution", () => {
  const at = (level: number): ReturnType<typeof personCoverage> => ({
    entityId: `p${level}`,
    states: new Map(),
    level,
  });

  it("reports how many people it counted", () => {
    // Not decoration. The same distribution is a true statement about the
    // people counted and a false one about the organisation, and this number
    // is the only thing separating them.
    const d = coverageDistribution([at(0), at(2), at(2)], 3);
    expect(d.counted).toBe(3);
  });

  it("seeds every level so an empty one reads as zero rather than as a gap", () => {
    const d = coverageDistribution([at(3), at(3)], 3);
    expect([...d.byLevel.entries()]).toEqual([
      [0, 0],
      [1, 0],
      [2, 0],
      [3, 2],
    ]);
  });

  it("counts nobody without claiming a shape", () => {
    const d = coverageDistribution([], 2);
    expect(d.counted).toBe(0);
    expect([...d.byLevel.values()]).toEqual([0, 0, 0]);
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

describe("thinlyCovered", () => {
  const at = (level: number): ReturnType<typeof personCoverage> => ({
    entityId: `p${level}`,
    states: new Map(),
    level,
  });

  it("counts people seen in fewer than half their parts", () => {
    // With five parts the boundary is unambiguous: two is under half, three is
    // over, and the midpoint is not a level anyone can be at.
    expect(thinlyCovered([at(0), at(1), at(2), at(3), at(4), at(5)], 5)).toBe(3);
  });

  it("puts exactly half on the covered side", () => {
    // Four parts, two seen: half is not "fewer than half". Stated because the
    // line this feeds is about whom the product cannot carry, and drawing an
    // exact half into that group would overstate the problem.
    expect(thinlyCovered([at(2)], 4)).toBe(0);
  });

  it("counts nobody when everyone is fully covered", () => {
    expect(thinlyCovered([at(5), at(5)], 5)).toBe(0);
  });

  it("counts everybody when nothing reaches us", () => {
    expect(thinlyCovered([at(0), at(0)], 5)).toBe(2);
  });
});

describe("partCoverage", () => {
  const GROUPS = [
    group("git_output", ["git.commits"]),
    group("collaboration", ["collab.messages"]),
  ];
  const person = (
    entityId: string,
    git: "reads" | "nothing_recorded" | "no_data_reaches_us",
    collab: "reads" | "nothing_recorded" | "no_data_reaches_us",
  ): ReturnType<typeof personCoverage> => ({
    entityId,
    states: new Map([
      ["git_output", git],
      ["collaboration", collab],
    ] as const),
    level: [git, collab].filter((s) => s === "reads").length,
  });

  it("counts, per part, the people it reads for", () => {
    expect(
      partCoverage(GROUPS, [
        person("a", "reads", "reads"),
        person("b", "nothing_recorded", "reads"),
      ]),
    ).toEqual([
      { id: "git_output", title: "git_output title", seen: 1, unreachable: false },
      { id: "collaboration", title: "collaboration title", seen: 2, unreachable: false },
    ]);
  });

  it("separates a part nobody is measured in from one everybody was idle in", () => {
    // Both read zero. Only one of them is a missing connector, and drawing
    // them the same would blame people for a pipe that was never laid.
    const [git, collab] = partCoverage(GROUPS, [
      person("a", "no_data_reaches_us", "nothing_recorded"),
      person("b", "no_data_reaches_us", "nothing_recorded"),
    ]);
    expect(git).toMatchObject({ seen: 0, unreachable: true });
    expect(collab).toMatchObject({ seen: 0, unreachable: false });
  });

  it("is derived from the same states as the per-person levels", () => {
    // The guarantee that matters: one computation, two cuts. Summing the
    // per-part counts must equal summing the per-person levels, always.
    const people = [
      person("a", "reads", "reads"),
      person("b", "reads", "nothing_recorded"),
      person("c", "no_data_reaches_us", "reads"),
    ];
    const byPart = partCoverage(GROUPS, people).reduce((n, p) => n + p.seen, 0);
    const byPerson = people.reduce((n, p) => n + p.level, 0);
    expect(byPart).toBe(byPerson);
  });
});

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  directionHidden,
  directionPlanned,
  EMPTY_NAV_PATHS,
  EMPTY_NAV_POLICY,
  gatesAnyMetric,
  itemHidden,
  itemPlanned,
  lensHidden,
  lensPlanned,
  metricPlanned,
  metricVisible,
  parseNavPaths,
  parseNavPolicy,
  personSectionHidden,
  personSectionPlanned,
  personSectionVisible,
  visibleMetricKeys,
  zoneHidden,
  zonePlanned,
} from "./nav-policy";

afterEach(() => {
  vi.restoreAllMocks();
});

function silenceWarnings() {
  return vi.spyOn(console, "warn").mockImplementation(() => {});
}

const ALL_LEVELS = [
  "zone:scorecard",
  "zone:aicost/item:idle-seats",
  "zone:directions/dir:sales",
  "zone:directions/dir:dev/lens:git-output",
  "zone:person/section:git_output",
];

describe("parseNavPaths", () => {
  it("reads every documented path form into its own tier", () => {
    const paths = parseNavPaths(ALL_LEVELS, "hide");
    const policy = { ...EMPTY_NAV_POLICY, hide: paths };

    expect(zoneHidden("scorecard", policy)).toBe(true);
    expect(itemHidden("aicost", "idle-seats", policy)).toBe(true);
    expect(directionHidden("sales", policy)).toBe(true);
    expect(lensHidden("dev", "git-output", policy)).toBe(true);
    expect(personSectionHidden("git_output", policy)).toBe(true);
  });

  it("matches nothing it was not told to match", () => {
    const paths = parseNavPaths(["zone:scorecard", "zone:aicost/item:idle-seats"], "hide");
    const policy = { ...EMPTY_NAV_POLICY, hide: paths };

    expect(zoneHidden("aicost", policy)).toBe(false);
    expect(itemHidden("aicost", "adoption-funnel", policy)).toBe(false);
    // Item ids are unique per zone — naming one zone's item leaves the
    // same-named item of another zone alone.
    expect(itemHidden("overview", "idle-seats", policy)).toBe(false);
  });

  it("scopes a lens to its direction", () => {
    const paths = parseNavPaths(["zone:directions/dir:dev/lens:overview"], "hide");
    const policy = { ...EMPTY_NAV_POLICY, hide: paths };

    expect(lensHidden("dev", "overview", policy)).toBe(true);
    expect(lensHidden("collab", "overview", policy)).toBe(false);
  });

  it.each([
    ["a non-string entry", 42],
    ["an empty string", ""],
    ["a bare id with no kind", "scorecard"],
    ["an unknown kind", "page:scorecard"],
    ["an item-first path", "item:idle-seats"],
    ["an empty value", "zone:"],
    ["an uppercase id", "zone:Scorecard"],
    ["a lens with no direction", "zone:directions/lens:git-output"],
    ["a section outside the person zone", "zone:aicost/section:git_output"],
    ["a path nested too deep", "zone:aicost/item:idle-seats/lens:x"],
  ])("ignores and warns on %s", (_label, entry) => {
    const warn = silenceWarnings();

    const paths = parseNavPaths([entry], "hide");

    expect(paths, `should ignore: ${JSON.stringify(entry)}`).toEqual(EMPTY_NAV_PATHS);
    expect(warn).toHaveBeenCalledOnce();
  });

  it("keeps the valid rest when one entry is malformed", () => {
    silenceWarnings();

    const paths = parseNavPaths(["zone:scorecard", "nonsense", "zone:reports"], "hide");

    expect(paths.zones.has("scorecard")).toBe(true);
    expect(paths.zones.has("reports")).toBe(true);
  });

  it("returns the empty set when the config carries nothing", () => {
    expect(parseNavPaths(undefined, "hide")).toEqual(EMPTY_NAV_PATHS);
    expect(parseNavPaths(null, "hide")).toEqual(EMPTY_NAV_PATHS);
    expect(parseNavPaths([], "hide")).toEqual(EMPTY_NAV_PATHS);
  });

  it("rejects a non-list value with a warning instead of crashing", () => {
    const warn = silenceWarnings();

    expect(parseNavPaths("zone:scorecard", "hide")).toEqual(EMPTY_NAV_PATHS);
    expect(warn).toHaveBeenCalledOnce();
  });
});

describe("parseNavPolicy", () => {
  it("keeps the two facets apart — hiding one entry does not plan it", () => {
    const policy = parseNavPolicy({
      hide: ["zone:scorecard"],
      planned: ALL_LEVELS,
    });

    expect(zoneHidden("scorecard", policy)).toBe(true);
    expect(zonePlanned("scorecard", policy)).toBe(true);
    expect(itemPlanned("aicost", "idle-seats", policy)).toBe(true);
    expect(itemHidden("aicost", "idle-seats", policy)).toBe(false);
    expect(directionPlanned("sales", policy)).toBe(true);
    expect(lensPlanned("dev", "git-output", policy)).toBe(true);
    expect(personSectionPlanned("git_output", policy)).toBe(true);
  });

  it("returns the empty policy when nav is absent", () => {
    expect(parseNavPolicy(undefined)).toEqual(EMPTY_NAV_POLICY);
    expect(parseNavPolicy(null)).toEqual(EMPTY_NAV_POLICY);
  });

  it("rejects a non-object nav with a warning instead of crashing", () => {
    const warn = silenceWarnings();

    expect(parseNavPolicy(["zone:scorecard"])).toEqual(EMPTY_NAV_POLICY);
    expect(warn).toHaveBeenCalledOnce();
  });
});

describe("personSectionVisible", () => {
  const policy = parseNavPolicy({
    hide: ["zone:person/section:wiki"],
    planned: ["zone:person/section:git_output"],
  });

  it("never lists a hidden section, whatever the toggle says", () => {
    for (const show of [true, false]) {
      expect(personSectionVisible("wiki", show, policy), `show=${show}`).toBe(false);
    }
  });

  it("lists a planned section only for a reader who opted in", () => {
    expect(personSectionVisible("git_output", true, policy)).toBe(true);
    expect(personSectionVisible("git_output", false, policy)).toBe(false);
  });

  it("always lists an unmarked section", () => {
    for (const show of [true, false]) {
      expect(personSectionVisible("collaboration", show, policy), `show=${show}`).toBe(
        true,
      );
    }
  });
});

describe("metric paths", () => {
  const policyWith = (planned: string[], hide: string[] = []) => ({
    hide: parseNavPaths(hide, "hide"),
    planned: parseNavPaths(planned, "planned"),
  });

  it("reads a whole key and a family into their own tiers", () => {
    const paths = parseNavPaths(
      ["metric:tasks.resolution_time", "metric:ai.*"],
      "planned"
    );

    expect(paths.metrics).toEqual(new Set(["tasks.resolution_time"]));
    expect(paths.metricFamilies).toEqual(new Set(["ai."]));
  });

  it("takes every key of a family named with a star", () => {
    const policy = policyWith(["metric:ai.*"]);

    expect(metricPlanned("ai.cost", policy)).toBe(true);
    expect(metricPlanned("ai.accepted_lines", policy)).toBe(true);
    // A family is a prefix up to the dot, so a key that merely starts with
    // the same letters is a different family.
    expect(metricPlanned("airflow.runs", policy)).toBe(false);
  });

  it("keeps a hidden metric off screen whatever the reader toggled", () => {
    const policy = policyWith([], ["metric:tasks.on_time_delivery"]);

    expect(metricVisible("tasks.on_time_delivery", false, policy)).toBe(false);
    expect(metricVisible("tasks.on_time_delivery", true, policy)).toBe(false);
  });

  it("shows a planned metric only to a reader who asked for planned sections", () => {
    const policy = policyWith(["metric:tasks.pickup_time"]);

    expect(metricVisible("tasks.pickup_time", false, policy)).toBe(false);
    expect(metricVisible("tasks.pickup_time", true, policy)).toBe(true);
    expect(metricVisible("tasks.closed", false, policy)).toBe(true);
  });

  it("filters a key list in place, preserving order", () => {
    const policy = policyWith(["metric:ai.*"], ["metric:tasks.pickup_time"]);
    const keys = ["git.commits", "ai.cost", "tasks.pickup_time", "tasks.closed"];

    expect(visibleMetricKeys(keys, false, policy)).toEqual([
      "git.commits",
      "tasks.closed",
    ]);
    expect(visibleMetricKeys(keys, true, policy)).toEqual([
      "git.commits",
      "ai.cost",
      "tasks.closed",
    ]);
  });

  it("reports whether the install gates any metric at all", () => {
    expect(gatesAnyMetric(EMPTY_NAV_POLICY)).toBe(false);
    expect(gatesAnyMetric(policyWith(["metric:ai.*"]))).toBe(true);
    expect(
      gatesAnyMetric(policyWith([], ["metric:tasks.resolution_time"]))
    ).toBe(true);
    // A navigation path is not a metric gate.
    expect(gatesAnyMetric(policyWith(["zone:scorecard"]))).toBe(false);
  });

  it.each([
    ["a metric with no family", "metric:commits"],
    ["a bare star", "metric:*"],
    ["a star inside the family", "metric:*.cost"],
    ["a three-part key", "metric:ai.cost.usd"],
    ["an uppercase key", "metric:AI.cost"],
    ["a metric nested under a zone", "zone:overview/metric:ai.cost"],
  ])("ignores and warns on %s", (_label, entry) => {
    const warn = silenceWarnings();
    const paths = parseNavPaths([entry], "planned");

    expect(paths.metrics.size + paths.metricFamilies.size).toBe(0);
    expect(warn).toHaveBeenCalled();
  });
});

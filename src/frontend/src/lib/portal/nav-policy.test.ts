import { afterEach, describe, expect, it, vi } from "vitest";

import {
  directionHidden,
  directionPlanned,
  EMPTY_NAV_PATHS,
  EMPTY_NAV_POLICY,
  itemHidden,
  itemPlanned,
  lensHidden,
  lensPlanned,
  parseNavPaths,
  parseNavPolicy,
  personSectionHidden,
  personSectionPlanned,
  personSectionVisible,
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

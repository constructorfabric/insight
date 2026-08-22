import { afterEach, describe, expect, it, vi } from "vitest";

import {
  directionHidden,
  EMPTY_NAV_HIDE,
  itemHidden,
  lensHidden,
  parseNavHide,
  personSectionHidden,
  zoneHidden,
} from "./nav-hide";

afterEach(() => {
  vi.restoreAllMocks();
});

function silenceWarnings() {
  return vi.spyOn(console, "warn").mockImplementation(() => {});
}

describe("parseNavHide", () => {
  it("reads every documented path form into its own tier", () => {
    const policy = parseNavHide([
      "zone:scorecard",
      "zone:aicost/item:idle-seats",
      "zone:directions/dir:sales",
      "zone:directions/dir:dev/lens:git-output",
      "zone:person/section:git_output",
    ]);

    expect(zoneHidden("scorecard", policy)).toBe(true);
    expect(itemHidden("aicost", "idle-seats", policy)).toBe(true);
    expect(directionHidden("sales", policy)).toBe(true);
    expect(lensHidden("dev", "git-output", policy)).toBe(true);
    expect(personSectionHidden("git_output", policy)).toBe(true);
  });

  it("hides nothing it was not told to hide", () => {
    const policy = parseNavHide(["zone:scorecard", "zone:aicost/item:idle-seats"]);

    expect(zoneHidden("aicost", policy)).toBe(false);
    expect(itemHidden("aicost", "adoption-funnel", policy)).toBe(false);
    // Item ids are unique per zone — hiding one zone's item leaves the
    // same-named item of another zone alone.
    expect(itemHidden("overview", "idle-seats", policy)).toBe(false);
  });

  it("scopes a lens to its direction", () => {
    const policy = parseNavHide(["zone:directions/dir:dev/lens:overview"]);

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

    const policy = parseNavHide([entry]);

    expect(policy, `should ignore: ${JSON.stringify(entry)}`).toEqual(EMPTY_NAV_HIDE);
    expect(warn).toHaveBeenCalledOnce();
  });

  it("keeps the valid rest when one entry is malformed", () => {
    silenceWarnings();

    const policy = parseNavHide(["zone:scorecard", "nonsense", "zone:reports"]);

    expect(zoneHidden("scorecard", policy)).toBe(true);
    expect(zoneHidden("reports", policy)).toBe(true);
  });

  it("returns the empty policy when the config carries nothing", () => {
    expect(parseNavHide(undefined)).toEqual(EMPTY_NAV_HIDE);
    expect(parseNavHide(null)).toEqual(EMPTY_NAV_HIDE);
    expect(parseNavHide([])).toEqual(EMPTY_NAV_HIDE);
  });

  it("rejects a non-list value with a warning instead of crashing", () => {
    const warn = silenceWarnings();

    expect(parseNavHide("zone:scorecard")).toEqual(EMPTY_NAV_HIDE);
    expect(warn).toHaveBeenCalledOnce();
  });
});

import { describe, expect, it } from "vitest";

import {
  emptyCase,
  type EmptyAction,
  type EmptyListing,
} from "./dashboards-empty";

const ADMIN: EmptyListing = {
  owned: undefined,
  folder: undefined,
  searching: false,
  tagged: false,
  isAdmin: true,
  assistant: false,
};

describe("emptyCase", () => {
  it.each<[string, Partial<EmptyListing>, string, EmptyAction]>([
    [
      "a search and tags that together match nothing",
      { searching: true, tagged: true, owned: "me" },
      "No dashboards match your search and tags",
      "clear-filters",
    ],
    [
      "a search that matched nothing",
      { searching: true, owned: "me" },
      "No dashboards match your search",
      "clear-search",
    ],
    [
      "tags that matched nothing",
      { tagged: true, owned: "shared" },
      "No dashboards with these tags",
      "clear-filters",
    ],
    ["Shared with me", { owned: "shared" }, "No shared dashboards", "none"],
    ["My dashboards", { owned: "me" }, "No dashboards yet", "create"],
    [
      "Unfiled",
      { folder: "unfiled" },
      "Every dashboard is in a folder",
      "none",
    ],
    ["a folder", { folder: "named" }, "This folder is empty", "none"],
    [
      "a member's whole list",
      { isAdmin: false },
      "No shared dashboards",
      "none",
    ],
    ["an admin's whole list", {}, "No dashboards yet", "create"],
  ])("names %s and offers its action", (_case, listing, title, action) => {
    const shown = emptyCase({ ...ADMIN, ...listing });

    expect(shown.title, `case: ${_case}`).toBe(title);
    expect(shown.action, `case: ${_case}`).toBe(action);
  });

  it("points to the assistant only where it is available", () => {
    expect(emptyCase(ADMIN).description).toBeUndefined();
    expect(emptyCase({ ...ADMIN, assistant: true }).description).toBe(
      "Describe one to the assistant and it will build it."
    );
  });
});

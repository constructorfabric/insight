import { describe, expect, it } from "vitest";

import type { PeopleListItem } from "@/api/identity-client";

import { rollsUpWholeRoster } from "./whole-roster";

function person(id: string, manager: string | null = null): PeopleListItem {
  return {
    person_id: id,
    email: `${id}@example.com`,
    display_name: id,
    first_name: null,
    last_name: null,
    username: null,
    attributes: {},
    manager_person_id: manager,
  };
}

describe("rollsUpWholeRoster", () => {
  it.each<[string, boolean, PeopleListItem[], boolean]>([
    ["a flat policy, even for a viewer alone", true, [person("me")], true],
    [
      "a viewer with no reports who is shown others",
      false,
      [person("me"), person("ada")],
      true,
    ],
    [
      "a manager, who keeps their subtree",
      false,
      [person("me"), person("ada", "me")],
      false,
    ],
    ["a viewer shown only themselves", false, [person("me")], false],
    [
      "a viewer whose id differs only in case",
      false,
      [person("ME"), person("ada", "Me")],
      false,
    ],
    [
      "a viewer outside the roster who is shown others",
      false,
      [person("ada")],
      true,
    ],
  ])("is decided for %s", (_case, isFlat, roster, expected) => {
    expect(rollsUpWholeRoster(isFlat, roster, "me"), `case: ${_case}`).toBe(
      expected
    );
  });

  it("decides nothing before the viewer is known", () => {
    expect(rollsUpWholeRoster(false, [person("ada")], null)).toBe(false);
  });
});

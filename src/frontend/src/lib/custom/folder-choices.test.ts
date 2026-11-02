import { describe, expect, it } from "vitest";

import type { Folder, FolderSummary } from "@/api/custom-client";

import { folderChoices } from "./folder-choices";

const folder = (
  id: string,
  access: FolderSummary["access"]
): FolderSummary => ({
  id,
  name: id,
  owner_person_id: access === "ownerless" ? null : "p-1",
  access,
  dashboards: 1,
});

const LISTED = [
  folder("Mine", "owner"),
  folder("Edited", "editor"),
  folder("Viewed", "viewer"),
  folder("Legacy", "ownerless"),
];

const held = (...ids: string[]): Folder[] =>
  ids.map((id) => ({ id, name: id }));

describe("folderChoices", () => {
  it.each([
    [
      "an owned dashboard filed nowhere: the folders it may go in",
      "owner",
      held(),
      [
        ["Mine", false, true],
        ["Edited", false, true],
      ],
    ],
    [
      "an ownerless dashboard: only ownerless folders",
      "ownerless",
      held(),
      [["Legacy", false, true]],
    ],
    [
      "a dashboard in a folder the caller edits: ticked and movable",
      "owner",
      held("Mine"),
      [
        ["Mine", true, true],
        ["Edited", false, true],
      ],
    ],
    [
      "a dashboard in a folder the caller only views: ticked, not movable",
      "owner",
      held("Viewed"),
      [
        ["Mine", false, true],
        ["Edited", false, true],
        ["Viewed", true, false],
      ],
    ],
    [
      "a dashboard in a folder the list does not hold: ticked, not movable",
      "owner",
      held("Elsewhere"),
      [
        ["Mine", false, true],
        ["Edited", false, true],
        ["Elsewhere", true, false],
      ],
    ],
  ] as const)("offers %s", (_case, access, holding, expected) => {
    const offered = folderChoices(access, LISTED, holding, false).map(
      (choice) => [choice.folder.name, choice.holds, choice.movable]
    );

    expect(offered, `case: ${_case}`).toEqual(expected);
  });

  it("lets an administrator take a dashboard out of a folder they own", () => {
    const [mine] = folderChoices("owner", LISTED, held("Mine"), true);

    expect(mine).toMatchObject({ holds: true, movable: true });
  });
});

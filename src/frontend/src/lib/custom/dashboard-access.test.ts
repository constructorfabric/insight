import { describe, expect, it } from "vitest";

import type {
  DashboardAccess,
  FolderAccess,
  FolderSummary,
} from "@/api/custom-client";

import {
  dashboardAllows,
  fileableFolders,
  folderAllows,
} from "./dashboard-access";

describe("dashboardAllows", () => {
  it.each([
    ["owner", { edit: true, share: true, remove: true }],
    ["editor", { edit: true, share: true, remove: false }],
    ["ownerless", { edit: true, share: false, remove: true }],
    ["viewer", { edit: false, share: false, remove: false }],
  ] as const)("lets %s do %o", (access, allowed) => {
    expect(dashboardAllows(access), `access: ${access}`).toEqual(allowed);
  });
});

describe("folderAllows", () => {
  it.each([
    ["owner", true, { edit: true, share: true, remove: true }],
    ["owner", false, { edit: true, share: false, remove: true }],
    ["editor", true, { edit: true, share: true, remove: false }],
    ["editor", false, { edit: true, share: false, remove: false }],
    ["ownerless", true, { edit: true, share: false, remove: true }],
    ["viewer", true, { edit: false, share: false, remove: false }],
  ] as [FolderAccess, boolean, object][])(
    "lets %s (administrator: %s) do %o",
    (access, isAdmin, allowed) => {
      expect(
        folderAllows(access, isAdmin),
        `access: ${access}, administrator: ${isAdmin}`
      ).toEqual(allowed);
    }
  );
});

describe("fileableFolders", () => {
  const folder = (id: string, access: FolderAccess): FolderSummary => ({
    id,
    name: id,
    owner_person_id: access === "ownerless" ? null : "p-1",
    access,
    dashboards: 0,
  });
  const FOLDERS = [
    folder("mine", "owner"),
    folder("edited", "editor"),
    folder("viewed", "viewer"),
    folder("legacy", "ownerless"),
  ];

  it.each([
    ["owner", [], ["mine", "edited"]],
    ["viewer", [], ["mine", "edited"]],
    ["ownerless", [], ["legacy"]],
    ["owner", [{ id: "mine" }], ["edited"]],
  ] as [DashboardAccess, { id: string }[], string[]][])(
    "offers a %s dashboard held in %o the folders %o",
    (access, holding, offered) => {
      expect(
        fileableFolders(access, FOLDERS, holding).map((one) => one.id),
        `access: ${access}`
      ).toEqual(offered);
    }
  );
});

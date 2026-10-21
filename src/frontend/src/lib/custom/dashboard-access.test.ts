import { describe, expect, it } from "vitest";

import type { DashboardAccess } from "@/api/custom-client";

import { dashboardAllows, fileableFolders } from "./dashboard-access";

describe("dashboardAllows", () => {
  it.each([
    ["owner", { change: true, share: true }],
    ["ownerless", { change: true, share: false }],
    ["viewer", { change: false, share: false }],
  ] as const)("lets %s do %o", (access, allowed) => {
    expect(dashboardAllows(access), `access: ${access}`).toEqual(allowed);
  });
});

describe("fileableFolders", () => {
  const FOLDERS = [
    { id: "mine", owner_person_id: "p-1" },
    { id: "shared-legacy", owner_person_id: null },
  ];

  it.each([
    ["owner", ["mine"]],
    ["viewer", ["mine"]],
    ["ownerless", ["shared-legacy"]],
  ] as [DashboardAccess, string[]][])(
    "files a %s dashboard only in %o",
    (access, offered) => {
      expect(
        fileableFolders(access, FOLDERS).map((folder) => folder.id),
        `access: ${access}`
      ).toEqual(offered);
    }
  );
});

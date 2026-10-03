import { describe, expect, it } from "vitest";

import { alertIdFromPath } from "./route";

describe("alertIdFromPath", () => {
  it.each([
    ["/portal/custom/alerts/a1", "a1"],
    ["/portal/custom/alerts/a1/edit", "a1"],
    ["/portal/custom/alerts/new", ""],
    ["/portal/custom/alerts", ""],
    ["/portal/custom/metrics/a1", ""],
  ])("reads %s as %o", (path, id) => {
    expect(alertIdFromPath(path), `path: ${path}`).toBe(id);
  });
});
